use crate::{
  types::state::State,
  modules::rag
};

use twilight_model::{
  channel::Message
};

pub async fn stats(msg: Message, state: State) -> anyhow::Result<()> {
  tracing::debug!(
    "stats command in channel {} by {}",
    msg.channel_id,
    msg.author.name
  );
  let read_ollama = rag::RAG_OLLAMA.read().await;
  let str_stats = read_ollama.get_stats();
  state
    .http
    .create_message(msg.channel_id)
    .reply(msg.id)
    .content(str_stats.as_str())
    .await?;
  Ok(())
}

pub async fn refresh(msg: Message, state: State) -> anyhow::Result<()> {
  tracing::debug!(
    "refresh command in channel {} by {}",
    msg.channel_id,
    msg.author.name
  );
  let mut write_ollama = rag::RAG_OLLAMA.write().await;
  let succ = match write_ollama.reload() {
    Ok(_)   => String::from("knwlage base refreshed"),
    Err(e)  => format!("failed to refres rag, {e}")
  };
  state
    .http
    .create_message(msg.channel_id)
    .reply(msg.id)
    .content(succ.as_str())
    .await?;
  Ok(())
}
