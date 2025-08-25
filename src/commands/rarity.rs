use crate::{
  types::state::State,
  options
};

use twilight_model::channel::Message;
use twilight_util::builder::embed::{
    EmbedBuilder,
    EmbedFooterBuilder
};

use anyhow::Context;

pub async fn try_acquire_permit<'a>(state: &'a State, msg: &Message) ->
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

pub async fn send_response(state: &State, msg: &Message, response: &str) -> anyhow::Result<()> {
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
