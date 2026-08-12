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
use std::sync::LazyLock;

use tokio::time::timeout;
use rand::RngExt;

use tracing::{error, info, warn};
use regex::Regex;

use async_recursion::async_recursion;

const OLLAMA_TIMEOUT: Duration      = Duration::from_secs(30 * 60);
const MAX_RESTART_ATTEMPTS: u8      = 3;
const RESTART_DELAY: Duration       = Duration::from_secs(10);
const MAX_CONTEXT_TOKENS: usize     = 4096;  // Conservative estimate for mistral-small3.2
const RESPONSE_TOKENS: usize        = 1000;  // Reserve tokens for response
// Floor for num_predict: token estimation is approximate (word-boundary
// truncation vs. the model's real tokenizer), so the budget can still come
// out razor-thin or at 0 even after truncate_prompt_smartly. Asking for 0
// tokens silently produces an empty response instead of an error, so never
// go below this.
const MIN_RESPONSE_TOKENS: usize    = 256;

static XML_TAG_REGEX: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"<[^>]+>").expect("Failed to compile XML tag regex")
});

fn estimate_tokens(text: &str) -> usize {
  if let Ok(bpe) = tiktoken_rs::cl100k_base() {
    bpe.encode_with_special_tokens(text).len()
  } else {
    (text.chars().count() + 3).div_ceil(4)
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

fn assemble_prompt(system_prompt: &str, secondary_prompt: Option<&str>, user_prompt: &str) -> String {
  match secondary_prompt {
    Some(secondary) => format!("{}\n\n{}\n\n{}", system_prompt, secondary, user_prompt),
    None            => format!("{}\n\n{}", system_prompt, user_prompt)
  }
}

fn truncate_prompt_smartly(system_prompt: &str, secondary_prompt: Option<&str>, user_prompt: &str) -> String {
  let system_tokens     = estimate_tokens(system_prompt);
  let secondary_tokens  = secondary_prompt.map(estimate_tokens).unwrap_or(0);
  let user_tokens       = estimate_tokens(user_prompt);
  let total_tokens      = system_tokens + secondary_tokens + user_tokens;

  let available_tokens = MAX_CONTEXT_TOKENS.saturating_sub(RESPONSE_TOKENS);

  if total_tokens <= available_tokens {
    return assemble_prompt(system_prompt, secondary_prompt, user_prompt);
  }

  warn!("Prompt too long ({} tokens), truncating user content", total_tokens);

  // Truncate only the user-supplied portion, not the whole assembled
  // prompt: truncating the concatenation and then re-prepending
  // system_prompt/secondary_prompt would duplicate them and could push the
  // final prompt back over budget (previously seen making it *longer* than
  // before truncation), which starves num_predict down to ~0 downstream.
  let max_user_tokens = available_tokens.saturating_sub(system_tokens + secondary_tokens);
  let max_user_chars  = max_user_tokens * 4;
  let truncated_user  = truncate_at_word_boundary(user_prompt, max_user_chars);

  info!("Truncated user content from {} to {} characters",
        user_prompt.chars().count(),
        truncated_user.chars().count());

  assemble_prompt(system_prompt, secondary_prompt, &truncated_user)
}

#[inline]
fn remove_xml_tags(input: &str) -> Cow<'_, str> {
  XML_TAG_REGEX.replace_all(input, "")
}

/// Picks a random model from `models`, preferring ones not in `exclude`
/// (used to retry with a different model after an unusable response). If
/// every model has already been excluded, falls back to the full list
/// rather than getting stuck.
fn pick_random_model_from<'a>(models: &'a [String], exclude: &[String]) -> Option<&'a str> {
  if models.is_empty() {
    return None;
  }

  let candidates: Vec<usize> = (0..models.len())
    .filter(|&i| !exclude.iter().any(|e| e == &models[i]))
    .collect();
  let pool: Vec<usize> = if candidates.is_empty() { (0..models.len()).collect() } else { candidates };

  let idx = pool[rand::rng().random_range(0..pool.len())];
  Some(&models[idx])
}

fn pick_random_model(exclude: &[String]) -> &'static str {
  pick_random_model_from(&options::CONFIG.models, exclude).unwrap_or_else(|| {
    warn!("No models configured, falling back to default");
    "mistral-small3.2:latest"
  })
}

pub async fn generate_ollama_response( prompt: &str
                                     , state: &State ) -> Result<String> {
  generate_ollama_response_with_retry(prompt, None, state, 0, &[])
    .await
    .map(|(text, _model)| text)
}

/// Like [`generate_ollama_response`] but also returns which model produced
/// the response, and accepts a list of models to avoid picking (e.g. ones
/// that already produced an unusable response for this prompt).
pub async fn generate_ollama_response_with_secondary( prompt: &str
                                                    , secondary_prompt: &str
                                                    , state: &State
                                                    , exclude_models: &[String] ) -> Result<(String, String)> {
  generate_ollama_response_with_retry(prompt, Some(secondary_prompt), state, 0, exclude_models).await
}

#[async_recursion]
async fn generate_ollama_response_with_retry(
  prompt: &str,
  secondary_prompt: Option<&str>,
  state: &State,
  attempt: u8,
  exclude_models: &[String]
) -> Result<(String, String)> {
  if attempt >= MAX_RESTART_ATTEMPTS {
    anyhow::bail!("Max restart attempts ({}) reached for Ollama", MAX_RESTART_ATTEMPTS);
  }

  info!("Generating Ollama response (attempt {})", attempt + 1);
  let result = timeout(
    OLLAMA_TIMEOUT,
    make_ollama_request( prompt
                       , secondary_prompt
                       , state
                       , exclude_models )
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
      generate_ollama_response_with_retry(prompt, secondary_prompt, state, attempt + 1, exclude_models).await
    }
  }
}

async fn make_ollama_request( prompt: &str
                            , secondary_prompt: Option<&str>
                            , state: &State
                            , exclude_models: &[String] ) -> Result<(String, String)> {

  let selected_model = pick_random_model(exclude_models);
  info!("Using model: {}", selected_model);

  let optimized_prompt = truncate_prompt_smartly( &options::CONFIG.system_prompt
                                                , secondary_prompt, prompt );
  let estimated_tokens = estimate_tokens(&optimized_prompt);
  
  info!("Prompt length: {} chars, estimated {} tokens", 
        optimized_prompt.chars().count(), 
        estimated_tokens);

  let available_for_response = MAX_CONTEXT_TOKENS.saturating_sub(estimated_tokens);
  let num_predict = std::cmp::min(4096, available_for_response.saturating_sub(100)).max(MIN_RESPONSE_TOKENS);
  
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
  Ok((processed.into_owned(), selected_model.to_string()))
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
  let read_ollama = rag::RAG_OLLAMA.read().await;

  let messages: Vec<(String, String)> = history.messages.iter().flat_map(|(msg_author, user_msg, bot_response)| {
    [
      (msg_author.to_string(), user_msg.to_string()),
      (options::CONFIG.bot_name.to_string(), bot_response.to_string()),
    ]
  }).collect();
  
  let chat_history = build_chat_history(messages.into_iter(), author, input);
  read_ollama.generate_smart(&chat_history, state).await
}

pub async fn generate_ollama_with_chat(
    input: &str,
    author: &str,
    chat: &GlobalConversationHistory,
    state: &State,
) -> Result<String> {
  let read_ollama = rag::RAG_OLLAMA.read().await;

  let messages: Vec<(String, String)> =
    chat.messages.iter()
                 .map(|(author, message)| (author.to_string(), message.to_string()))
                 .collect();

  let chat_history = build_chat_history(messages.into_iter(), author, input);
  read_ollama.generate_smart(&chat_history, state).await
}

#[cfg(test)]
mod tests {
  use super::*;

  fn to_owned_strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
  }

  #[test]
  fn no_models_returns_none() {
    assert_eq!(pick_random_model_from(&[], &[]), None);
  }

  #[test]
  fn excludes_the_only_other_model_when_possible() {
    let models = to_owned_strings(&["a", "b"]);
    let exclude = to_owned_strings(&["a"]);

    for _ in 0..20 {
      assert_eq!(pick_random_model_from(&models, &exclude), Some("b"));
    }
  }

  #[test]
  fn falls_back_to_the_full_list_once_everything_is_excluded() {
    let models = to_owned_strings(&["a", "b"]);
    let exclude = to_owned_strings(&["a", "b"]);

    for _ in 0..20 {
      assert!(pick_random_model_from(&models, &exclude).is_some());
    }
  }

  #[test]
  fn truncate_prompt_smartly_leaves_short_prompts_untouched() {
    let result = truncate_prompt_smartly("SYSTEM", Some("SECONDARY"), "short user prompt");
    assert_eq!(result, "SYSTEM\n\nSECONDARY\n\nshort user prompt");
  }

  #[test]
  fn truncate_prompt_smartly_does_not_duplicate_system_or_secondary_prompt() {
    // Long enough to force the truncation branch.
    let user_prompt = "word ".repeat(5000);

    let result = truncate_prompt_smartly("SYSTEM_MARKER", Some("SECONDARY_MARKER"), &user_prompt);

    // Previously, truncating the already-assembled prompt and then
    // re-prepending system/secondary duplicated both of them.
    assert_eq!(result.matches("SYSTEM_MARKER").count(), 1);
    assert_eq!(result.matches("SECONDARY_MARKER").count(), 1);
  }

  #[test]
  fn truncate_prompt_smartly_stays_within_the_response_token_budget() {
    let user_prompt = "word ".repeat(5000);
    let available_tokens = MAX_CONTEXT_TOKENS.saturating_sub(RESPONSE_TOKENS);

    let result = truncate_prompt_smartly("SYSTEM_MARKER", Some("SECONDARY_MARKER"), &user_prompt);

    // A prompt that still exceeds the context budget after "truncation"
    // starves num_predict down toward 0 in make_ollama_request, which
    // silently produces an empty response instead of an error.
    assert!(
      estimate_tokens(&result) <= available_tokens,
      "truncated prompt is {} estimated tokens, over the {} token budget",
      estimate_tokens(&result), available_tokens
    );
  }
}
