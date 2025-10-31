use crate::{
  types::state::{ State, ConversationHistory, GlobalConversationHistory },
  modules::rag,
  options
};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use std::process::Command;
use std::time::Duration;
use std::borrow::Cow;

use tokio::time::timeout;
use rand::Rng;

use tracing::{error, info, warn};
use regex::Regex;

use once_cell::sync::Lazy;

use async_recursion::async_recursion;

const OLLAMA_TIMEOUT: Duration      = Duration::from_secs(30 * 60);
const MAX_RESTART_ATTEMPTS: u8      = 3;
const RESTART_DELAY: Duration       = Duration::from_secs(10);
const MAX_CONTEXT_TOKENS: usize     = 4096;  // Conservative estimate for mistral-small3.2
const RESPONSE_TOKENS: usize        = 1000;  // Reserve tokens for response

static XML_TAG_REGEX: Lazy<Regex> = Lazy::new(|| {
  Regex::new(r"<[^>]+>").expect("Failed to compile XML tag regex")
});

fn estimate_tokens(text: &str) -> usize {
  if let Ok(bpe) = tiktoken_rs::cl100k_base() {
    bpe.encode_with_special_tokens(text).len()
  } else {
    (text.chars().count() + 3) / 4
  }
}

fn truncate_at_word_boundary(text: &str, max_chars: usize) -> String {
  if text.chars().count() <= max_chars {
    return text.to_string();
  }

  let chars: Vec<char>  = text.chars().collect();
  let max_content_chars = max_chars.saturating_sub(3);
  let mut end_pos       = std::cmp::min(max_content_chars, chars.len());

  let search_start = end_pos.saturating_sub(100);
  for i in (search_start..end_pos).rev() {
    if chars[i] == ' ' || chars[i] == '\n' || chars[i] == '.' {
      end_pos = i;
      break;
    }
  }

  chars[..end_pos].iter().collect::<String>() + "..."
}

fn truncate_prompt_smartly(system_prompt: &str, secondary_prompt: Option<&str>, user_prompt: &str) -> String {
  let system_tokens     = estimate_tokens(system_prompt);
  let secondary_tokens  = secondary_prompt.map(estimate_tokens).unwrap_or(0);
  let user_tokens       = estimate_tokens(user_prompt);
  let total_tokens      = system_tokens + secondary_tokens + user_tokens;
  
  let available_tokens = MAX_CONTEXT_TOKENS.saturating_sub(RESPONSE_TOKENS);
  
  let full_prompt = match secondary_prompt {
    Some(secondary) => format!("{}\n\n{}\n\n{}", system_prompt, secondary, user_prompt),
    None            => format!("{}\n\n{}", system_prompt, user_prompt)
  };
  
  if total_tokens <= available_tokens {
    return full_prompt;
  }
  
  warn!("Prompt too long ({} tokens), truncating user content", total_tokens);
  
  let max_user_tokens = available_tokens.saturating_sub(system_tokens + secondary_tokens);
  let max_user_chars  = max_user_tokens * 4;
  let truncated_user  = truncate_at_word_boundary(&full_prompt, max_user_chars);
  
  info!("Truncated prompt from {} to {} characters", 
        user_prompt.chars().count(), 
        truncated_user.chars().count());
  
  match secondary_prompt {
    Some(secondary) => format!("{}\n\n{}\n\n{}", system_prompt, secondary, truncated_user),
    None            => format!("{}\n\n{}", system_prompt, truncated_user)
  }
}

#[inline]
fn remove_xml_tags(input: &str) -> Cow<'_, str> {
  XML_TAG_REGEX.replace_all(input, "")
}

fn get_random_model() -> &'static str {
  let models = &options::CONFIG.models;
  if models.is_empty() {
    warn!("No models configured, falling back to default");
    return "mistral-small3.2:latest";
  }
  let idx = rand::rng().random_range(0..models.len());
  &models[idx]
}

pub async fn generate_ollama_response( prompt: &str
                                     , state: &State ) -> Result<String> {
  generate_ollama_response_with_retry(prompt, None, state, 0).await
}

pub async fn generate_ollama_response_with_secondary( prompt: &str
                                                    , secondary_prompt: &str
                                                    , state: &State ) -> Result<String> {
  generate_ollama_response_with_retry(prompt, Some(secondary_prompt), state, 0).await
}

#[async_recursion]
async fn generate_ollama_response_with_retry(
  prompt: &str,
  secondary_prompt: Option<&str>,
  state: &State, 
  attempt: u8
) -> Result<String> {
  if attempt >= MAX_RESTART_ATTEMPTS {
    anyhow::bail!("Max restart attempts ({}) reached for Ollama", MAX_RESTART_ATTEMPTS);
  }

  info!("Generating Ollama response (attempt {})", attempt + 1);
  let result = timeout(
    OLLAMA_TIMEOUT,
    make_ollama_request( prompt
                       , secondary_prompt
                       , state )
  ).await;

  match result {
    Ok(Ok(response)) => {
      info!("Ollama response generated successfully");
      Ok(response)
    }
    Ok(Err(e)) => {
      error!("Ollama request failed: {}", e);
      Err(e)
    }
    Err(_) => {
      warn!( "Ollama request timed out after {} minutes, attempting restart"
           , OLLAMA_TIMEOUT.as_secs() / 60 );
      
      restart_ollama().await?;
      info!("Ollama restarted successfully, retrying request");
      tokio::time::sleep(RESTART_DELAY).await;
      generate_ollama_response_with_retry(prompt, secondary_prompt, state, attempt + 1).await
    }
  }
}

async fn make_ollama_request( prompt: &str
                            , secondary_prompt: Option<&str>
                            , state: &State ) -> Result<String> {

  let selected_model = get_random_model();
  info!("Using model: {}", selected_model);

  let optimized_prompt = truncate_prompt_smartly( &options::CONFIG.system_prompt
                                                , secondary_prompt, prompt );
  let estimated_tokens = estimate_tokens(&optimized_prompt);
  
  info!("Prompt length: {} chars, estimated {} tokens", 
        optimized_prompt.chars().count(), 
        estimated_tokens);

  let available_for_response = MAX_CONTEXT_TOKENS.saturating_sub(estimated_tokens);
  let num_predict = std::cmp::min(4096, available_for_response.saturating_sub(100));
  
  let request_body = json!({
    "model": selected_model,
    "prompt": optimized_prompt,
    "stream": false,
    "options": {
      "num_predict": num_predict,
      "num_ctx": MAX_CONTEXT_TOKENS,  // Set explicit context window
      "temperature": 0.85,
      "top_p": 0.85,
      "repeat_penalty": 1.15
    }
  });

  let response = state
    .request_client
    .post("http://localhost:11434/api/generate")
    .json(&request_body)
    .send()
    .await
    .context("Failed to connect to Ollama API")?;

  if !response.status().is_success() {
    anyhow::bail!("Ollama API returned error status: {}", response.status());
  }

  let response_json: Value = response
    .json()
    .await
    .context("Failed to parse Ollama JSON response")?;

  let generated_text = response_json
    .get("response")
    .and_then(Value::as_str)
    .context("Missing or invalid 'response' field in Ollama output")?;

  let processed = remove_xml_tags(generated_text);
  Ok(processed.into_owned())
}

async fn restart_ollama() -> Result<()> {
  info!("Restarting Ollama service...");
  
  let restart_output = Command::new("sudo")
    .args(["systemctl", "restart", "ollama"])
    .output()
    .context("Failed to execute systemctl restart command")?;

  if !restart_output.status.success() {
    let stderr = String::from_utf8_lossy(&restart_output.stderr);
    anyhow::bail!("Failed to restart Ollama service: {}", stderr);
  }

  let status_output = Command::new("sudo")
    .args(["systemctl", "is-active", "ollama"])
    .output()
    .context("Failed to check Ollama service status")?;

  let status_output_stdout = status_output.stdout;
  let status = String::from_utf8_lossy(&status_output_stdout);
  let status_trimmed = status.trim();
  if status_trimmed != "active" {
    anyhow::bail!("Ollama service is not active after restart. Status: {}", status);
  }

  info!("Ollama service restarted successfully");
  Ok(())
}

fn build_chat_history<I>(messages: I, author: &str, input: &str) -> String
  where
 I: Iterator<Item = (String, String)>
{
  let mut chat_history = String::new();
  
  for (msg_author, message) in messages {
    chat_history.push_str(&msg_author);
    chat_history.push_str(": ");
    chat_history.push_str(&message);
    chat_history.push('\n');
  }
  
  chat_history.push_str(author);
  chat_history.push_str(": ");
  chat_history.push_str(input);
  chat_history.push('\n');
  chat_history.push_str(&options::CONFIG.bot_name);
  chat_history.push_str(": ");
  
  chat_history
}

pub async fn generate_ollama_with_history(
    input: &str,
    author: &str,
    history: &ConversationHistory,
    state: &State,
) -> Result<String> {
  let messages: Vec<(String, String)> = history.messages.iter().flat_map(|(msg_author, user_msg, bot_response)| {
    [
      (msg_author.to_string(), user_msg.to_string()),
      (options::CONFIG.bot_name.to_string(), bot_response.to_string()),
    ]
  }).collect();
  
  let chat_history = build_chat_history(messages.into_iter(), author, input);
  rag::RAG_OLLAMA.generate_smart(&chat_history, state).await
}

pub async fn generate_ollama_with_chat(
    input: &str,
    author: &str,
    chat: &GlobalConversationHistory,
    state: &State,
) -> Result<String> {
  let messages: Vec<(String, String)> =
    chat.messages.iter()
                 .map(|(author, message)| (author.to_string(), message.to_string()))
                 .collect();

  let chat_history = build_chat_history(messages.into_iter(), author, input);
  rag::RAG_OLLAMA.generate_smart(&chat_history, state).await
}
