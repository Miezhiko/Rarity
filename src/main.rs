#[macro_use] extern crate anyhow;

mod types;
mod options;
mod handler;
mod commands;

use crate::types::common::{ StateRef, PersonalityConfig };
use crate::handler::handle_event;

use std::sync::Arc;
use std::collections::HashMap;

use tokio::sync::Mutex;

use twilight_cache_inmemory::{
  DefaultInMemoryCache,
  ResourceType
};

use twilight_gateway::{
  EventTypeFlags,
  Intents, Shard,
  ShardId,
  StreamExt as _
};

use twilight_http::client::ClientBuilder;

use tracing_subscriber::FmtSubscriber;
use tracing::Level;

#[tokio::main(worker_threads=16)]
async fn main() -> anyhow::Result<()> {
  let iopts = options::get_ioptions()
                .map_err(|e| anyhow!("Failed to parse Dhall config {e}"))?;

  let subscriber = FmtSubscriber::builder()
    .with_max_level(Level::INFO)
    .finish();
  tracing::subscriber::set_global_default(subscriber)?;

  let mut shard = Shard::new(
    ShardId::ONE,
    iopts.discord.clone(),
    Intents::GUILD_MESSAGES | Intents::MESSAGE_CONTENT,
  );

  let http = ClientBuilder::new()
                           .token(iopts.discord)
                           .build();

  let cache = DefaultInMemoryCache::builder()
                .resource_types(ResourceType::MESSAGE)
                .build();

  let request_client = reqwest::Client::builder()
                .pool_max_idle_per_host(0)
                .build()?;

  let personality = PersonalityConfig {
    system_prompt: String::from(
      iopts.system_prompt
    ),
    embed_color: 0xFF69B4,
    footer_text: String::from(iopts.footer_text)
  };

  let state = Arc::new(StateRef {
    http,
    request_client,
    generation_lock: Arc::new(tokio::sync::Semaphore::new(1)),
    conversation_history: Arc::new(Mutex::new(HashMap::new())),
    personality
  });

  tracing::info!("listening events");

  while let Some(item) = shard.next_event(EventTypeFlags::all()).await {
    let Ok(event) = item else {
      tracing::warn!(source = ?item.unwrap_err(), "error receiving event");
      continue;
    };

    cache.update(&event);
    tokio::spawn(handle_event(event, Arc::clone(&state)));
  }

  tracing::error!("Event loop terminated unexpectedly");

  Ok(())
}
