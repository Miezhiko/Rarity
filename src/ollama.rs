use crate::{
  types::state::{ State, ConversationHistory },
  options
};

use serde_json::{json, Value};
use anyhow::Context;

use once_cell::sync::Lazy;
use regex::Regex;

static XML_TAG_REGEX: Lazy<Regex> = Lazy::new(|| {
  Regex::new(r"<[^>]+>").expect("Failed to compile regex")
});

fn remove_xml_tags(input: &str) -> String {
  XML_TAG_REGEX.replace_all(input, "").to_string()
}

pub async fn generate_ollama_with_history( input: &str
                                         , author: &str
                                         , history: &ConversationHistory
                                         , state: &State ) -> anyhow::Result<String> {
  let mut chat_history = String::new();
  for (user_msg, bot_response) in &history.messages {
    chat_history.push_str(&format!("{author}: {user_msg}\n{}: {bot_response}\n", &options::CONFIG.bot_name));
  }
  chat_history.push_str(&format!("{author}: {input}\n{}: ", &options::CONFIG.bot_name));
  generate_ollama_response(chat_history.as_str(), state).await
}

pub async fn generate_ollama_with_chat( input: &str
                                      , author: &str
                                      , chat: &ConversationHistory
                                      , state: &State ) -> anyhow::Result<String> {
  let mut chat_history = String::new();
  for (author, message) in &chat.messages {
    chat_history.push_str(&format!("{author}: {message}\n"));
  }
  chat_history.push_str(&format!("{author}: {input}\n{}: ", &options::CONFIG.bot_name));
  generate_ollama_response(chat_history.as_str(), state).await
}

async fn generate_ollama_response( prompt: &str
                                 , state: &State ) -> anyhow::Result<String> {
  let request_body = json!({
      "model": options::CONFIG.model,
      "system": options::CONFIG.system_prompt,
      "prompt": prompt,
      "stream": false,
      "max_tokens": 500,
      "temperature": 0.7
  });

  let response = state.request_client
      .post("http://localhost:11434/api/generate")
      .json(&request_body)
      .send()
      .await
      .context("Failed to connect to Ollama API")?;

  let response_json: Value = response
      .json()
      .await
      .context("Failed to parse Ollama response")?;

  let generated_text = response_json["response"]
      .as_str()
      .context("No response text in Ollama output")?
      .to_string();

  let post_process = remove_xml_tags(&generated_text);

  Ok(post_process)
}
