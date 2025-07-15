use crate::{
  types::state::{ State, ConversationHistory },
  state,
  options,
  ollama
};

use twilight_model::channel::Message;
use twilight_util::builder::embed::{
    EmbedBuilder,
    EmbedFooterBuilder
};

use anyhow::Context;

async fn try_acquire_permit<'a>(state: &'a State, msg: &Message) ->
    anyhow::Result<Option<tokio::sync::SemaphorePermit<'a>>> {
  match state.generation_lock.try_acquire() {
    Ok(p) => Ok(Some(p)),
    Err(_) => {
      let busy_embed = EmbedBuilder::new()
        .description("Я занята, напиши минут через десять!")
        .color(0xFF69B4)
        .footer(EmbedFooterBuilder::new(&options::CONFIG.footer_text).build())
        .timestamp(msg.timestamp)
        .build();

      state.http
        .create_message(msg.channel_id)
        .embeds(&[busy_embed])
        .reply(msg.id)
        .await
        .context("Failed to send busy message")?;

      Ok(None)
    }
  }
}

async fn send_response(state: &State, msg: &Message, response: &str) -> anyhow::Result<()> {
  let embed = EmbedBuilder::new()
    .description(response)
    .color(0xFF69B4)
    .footer(EmbedFooterBuilder::new(&options::CONFIG.footer_text).build())
    .timestamp(msg.timestamp)
    .build();

  state.http
    .create_message(msg.channel_id)
    .embeds(&[embed])
    .reply(msg.id)
    .await
    .context("Failed to send Discord message")?;

  Ok(())
}

pub async fn speak(
  msg: Message,
  text: String,
  author: String,
  state: State,
) -> anyhow::Result<()> {
  tracing::debug!("speak command in channel {} by {}", msg.channel_id, msg.author.name);

  let permit = match try_acquire_permit(&state, &msg).await? {
    Some(p) => p,
    None => return Ok(()),
  };

  let history_lock = state.global_conversation_history.lock().await;
  let response = ollama::generate_ollama_with_chat(&text, &author, &history_lock, &state)
    .await
    .context("Failed to generate response")?;

  send_response(&state, &msg, &response).await?;
  drop(permit);

  Ok(())
}

pub async fn reply(
  msg: Message,
  text: String,
  author: String,
  state: State,
) -> anyhow::Result<()> {
  tracing::debug!("reply command in channel {} by {}", msg.channel_id, msg.author.name);

  let permit = match try_acquire_permit(&state, &msg).await? {
    Some(p) => p,
    None => return Ok(()),
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

  let response = ollama::generate_ollama_with_history(&text, &author, &history, &state)
    .await
    .context("Failed to generate response")?;

  history.messages.push((text.clone(), response.clone()));
  {
    let mut history_lock = state.conversation_history.lock().await;
    history_lock.insert(msg.channel_id.to_string(), history);
  }

  send_response(&state, &msg, &response).await?;
  drop(permit);

  Ok(())
}
