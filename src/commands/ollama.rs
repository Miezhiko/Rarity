use crate::{
  types::state::{ State, ConversationHistory },
  state,
  options
};

use twilight_model::channel::Message;
use twilight_util::builder::embed::{
    EmbedBuilder,
    EmbedFooterBuilder
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

pub async fn speak( msg: Message
                  , text: String
                  , author: String
                  , state: State ) -> anyhow::Result<()> {
  tracing::debug!("speak command in channel {} by {}",
    msg.channel_id,
    msg.author.name
  );

  let permit = match state.generation_lock.try_acquire() {
    Ok(p) => p,
    Err(_) => {
      return Ok(());
    }
  };

  let history_lock = state.global_conversation_history.lock().await;
  let ollama_response = generate_ollama_with_chat(&text, &author, &history_lock, &state).await
      .context("Failed to generate response")?;

  let embed = EmbedBuilder::new()
      .description(ollama_response)
      .color(0xFF69B4)
      .footer(
          EmbedFooterBuilder::new(&options::CONFIG.footer_text)
              .build()
      )
      .timestamp(msg.timestamp)
      .build();

  state.http
      .create_message(msg.channel_id)
      .embeds(&[embed])
      .reply(msg.id)
      .await
      .context("Failed to send Discord message")?;

  drop(permit);

  Ok(())
}

pub async fn reply( msg: Message
                  , text: String
                  , author: String
                  , state: State ) -> anyhow::Result<()> {
  tracing::debug!(
      "reply command in channel {} by {}",
      msg.channel_id,
      msg.author.name
  );

  let permit = match state.generation_lock.try_acquire() {
    Ok(p) => p,
    Err(_) => {
      let busy_embed = EmbedBuilder::new()
          .description("I'm super busy throwing a party for another request! 🎉 Try again in ten minutes, okay?")
          .color(0xFF69B4)
          .footer(
              EmbedFooterBuilder::new(&options::CONFIG.footer_text)
                  .build()
          )
          .timestamp(msg.timestamp)
          .build();

      state.http
          .create_message(msg.channel_id)
          .embeds(&[busy_embed])
          .reply(msg.id)
          .await
          .context("Failed to send busy message")?;

      return Ok(());
    }
  };

  let mut history = {
    let mut history_lock = state.conversation_history.lock().await;
    let entry = history_lock.entry(msg.channel_id.to_string())
      .or_insert_with(|| ConversationHistory { messages: Vec::new() });

    let mut cloned = entry.clone();

    if cloned.messages.len() > state::HISTORY_LIMIT {
      cloned.messages.drain(0..cloned.messages.len() - state::HISTORY_LIMIT);
    }
    cloned
  };

  let ollama_response = generate_ollama_with_history(&text, &author, &history, &state).await
      .context("Failed to generate response")?;

  history.messages.push((text.clone(), ollama_response.clone()));
  {
    let mut history_lock = state.conversation_history.lock().await;
    history_lock.insert(msg.channel_id.to_string(), history);
  }

  let embed = EmbedBuilder::new()
      .description(ollama_response)
      .color(0xFF69B4)
      .footer(
          EmbedFooterBuilder::new(&options::CONFIG.footer_text)
              .build()
      )
      .timestamp(msg.timestamp)
      .build();

  state.http
      .create_message(msg.channel_id)
      .embeds(&[embed])
      .reply(msg.id)
      .await
      .context("Failed to send Discord message")?;

  drop(permit);

  Ok(())
}

async fn generate_ollama_with_history( input: &str
                                     , author: &str
                                     , history: &ConversationHistory
                                     , state: &State ) -> anyhow::Result<String> {
  let mut chat_history = String::new();
  for (user_msg, bot_response) in &history.messages {
    chat_history.push_str(&format!("{}: {}\nAssistant: {}\n", author, user_msg, bot_response));
  }
  chat_history.push_str(&format!("{}: {}\nAssistant: ", author, input));
  generate_ollama_response(chat_history.as_str(), state).await
}

async fn generate_ollama_with_chat( input: &str
                                  , author: &str
                                  , chat: &ConversationHistory
                                  , state: &State ) -> anyhow::Result<String> {
  let mut chat_history = String::new();
  for (author, message) in &chat.messages {
    chat_history.push_str(&format!("{}: {}", author, message));
  }
  chat_history.push_str(&format!("{}: {}\nAssistant: ", author, input));
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
