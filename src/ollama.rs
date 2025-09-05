use crate::{
  types::state::{ State, ConversationHistory, GlobalConversationHistory },
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

const OLLAMA_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_RESTART_ATTEMPTS: u8 = 3;
const RESTART_DELAY: Duration = Duration::from_secs(10);

static XML_TAG_REGEX: Lazy<Regex> = Lazy::new(|| {
  Regex::new(r"<[^>]+>").expect("Failed to compile XML tag regex")
});

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

pub async fn generate_ollama_response(prompt: &str, state: &State) -> Result<String> {
  generate_ollama_response_with_retry(prompt, state, 0).await
}

#[async_recursion]
async fn generate_ollama_response_with_retry(
  prompt: &str, 
  state: &State, 
  attempt: u8
) -> Result<String> {
  if attempt >= MAX_RESTART_ATTEMPTS {
    anyhow::bail!("Max restart attempts ({}) reached for Ollama", MAX_RESTART_ATTEMPTS);
  }

  info!("Generating Ollama response (attempt {})", attempt + 1);
  
  let result = timeout(
    OLLAMA_TIMEOUT,
    make_ollama_request(prompt, state)
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
      generate_ollama_response_with_retry(prompt, state, attempt + 1).await
    }
  }
}

async fn make_ollama_request(prompt: &str, state: &State) -> Result<String> {
  let selected_model = get_random_model();
  info!("Using model: {}", selected_model);

  let request_body = json!({
    "model": selected_model,
    "system": options::CONFIG.system_prompt,
    "prompt": prompt,
    "stream": false,
    "max_tokens": 500_u16,
    "temperature": 0.7_f32,
    "top_p": 0.9_f32
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
  generate_ollama_response(&chat_history, state).await
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
  generate_ollama_response(&chat_history, state).await
}
