use crate::{
  types::state::State,
  ollama,
  commands::rarity
};

use twilight_model::channel::Message;

use anyhow::Context;

pub async fn speak(
  msg: Message,
  text: String,
  author: String,
  state: State
) -> anyhow::Result<()> {
  tracing::debug!("speak command in channel {} by {}", msg.channel_id, msg.author.name);

  let permit = match rarity::try_acquire_permit(&state, &msg).await? {
    Some(p) => p,
    None    => return Ok(())
  };

  let history_lock = state.global_conversation_history.lock().await;
  let response = ollama::generate_ollama_with_chat(&text, &author, &history_lock, &state)
    .await
    .context("Failed to generate response")?;

  rarity::send_response(&state, &msg, &response).await?;
  drop(permit);

  Ok(())
}
