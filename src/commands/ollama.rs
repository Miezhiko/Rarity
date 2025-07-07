use crate::types::common::{ State, ConversationHistory };

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

pub async fn reply(msg: Message, text: String, state: State) -> anyhow::Result<()> {
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
          .color(state.personality.embed_color)
          .footer(
              EmbedFooterBuilder::new(&state.personality.footer_text)
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

    if cloned.messages.len() > 5 {
      cloned.messages.drain(0..cloned.messages.len() - 5);
    }
    cloned
  };

  let ollama_response = generate_ollama_response(&text, &history, &state).await
      .context("Failed to generate response")?;

  history.messages.push((text.clone(), ollama_response.clone()));
  {
    let mut history_lock = state.conversation_history.lock().await;
    history_lock.insert(msg.channel_id.to_string(), history);
  }

  let embed = EmbedBuilder::new()
      .description(ollama_response)
      .color(state.personality.embed_color)
      .footer(
          EmbedFooterBuilder::new(&state.personality.footer_text)
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

async fn generate_ollama_response( input: &str
                                 , history: &ConversationHistory
                                 , state: &State ) -> anyhow::Result<String> {
  let mut prompt = state.personality.system_prompt.clone();
  for (user_msg, bot_response) in &history.messages {
    prompt.push_str(&format!("User: {}\nAssistant: {}\n", user_msg, bot_response));
  }
  prompt.push_str(&format!("User: {}\nAssistant: ", input));

  let request_body = json!({
      "model": "deepseek-r1:latest",
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
