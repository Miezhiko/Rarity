use crate::{
  types::state::{ State, ConversationHistory, GlobalConversationHistory },
  options
};

use anyhow::{ Context, Result };

use once_cell::sync::Lazy;
use regex::Regex;

use serde_json::{ json, Value };

use std::borrow::Cow;

static XML_TAG_REGEX: Lazy<Regex> = Lazy::new(|| {
  Regex::new(r"<[^>]+>").expect("Failed to compile XML tag regex")
});

#[inline]
fn remove_xml_tags(input: &str) -> Cow<'_, str> {
  XML_TAG_REGEX.replace_all(input, "")
}

pub async fn generate_ollama_with_history(
    input: &str,
    author: &str,
    history: &ConversationHistory,
    state: &State,
) -> Result<String> {
  let estimated_capacity = history.messages.len() * 100 + input.len() + author.len() + 50;
  let mut chat_history = String::with_capacity(estimated_capacity);
  
  for (msg_author, user_msg, bot_response) in &history.messages {
    chat_history.reserve(msg_author.len() + user_msg.len() + bot_response.len() + 10);
    chat_history.push_str(msg_author);
    chat_history.push_str(": ");
    chat_history.push_str(user_msg);
    chat_history.push('\n');
    chat_history.push_str(&options::CONFIG.bot_name);
    chat_history.push_str(": ");
    chat_history.push_str(bot_response);
    chat_history.push('\n');
  }

  chat_history.push_str(author);
  chat_history.push_str(": ");
  chat_history.push_str(input);
  chat_history.push('\n');
  chat_history.push_str(&options::CONFIG.bot_name);
  chat_history.push_str(": ");
  
  generate_ollama_response(&chat_history, state).await
}

pub async fn generate_ollama_with_chat(
    input: &str,
    author: &str,
    chat: &GlobalConversationHistory,
    state: &State,
) -> Result<String> {
  let estimated_capacity = chat.messages.len() * 80 + input.len() + author.len() + 50;
  let mut chat_history = String::with_capacity(estimated_capacity);
  
  for (msg_author, message) in &chat.messages {
    chat_history.reserve(msg_author.len() + message.len() + 4);
    chat_history.push_str(msg_author);
    chat_history.push_str(": ");
    chat_history.push_str(message);
    chat_history.push('\n');
  }
  
  chat_history.push_str(author);
  chat_history.push_str(": ");
  chat_history.push_str(input);
  chat_history.push('\n');
  chat_history.push_str(&options::CONFIG.bot_name);
  chat_history.push_str(": ");
  
  generate_ollama_response(&chat_history, state).await
}

async fn generate_ollama_response(prompt: &str, state: &State) -> Result<String> {
  let request_body = json!({
    "model": options::CONFIG.model,
    "system": options::CONFIG.system_prompt,
    "prompt": prompt,
    "stream": false,
    "max_tokens": 500_u16,
    "temperature": 0.7_f32
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
