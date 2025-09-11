#[allow(unused_imports)]
#[macro_use] extern crate anyhow;

mod types;
#[macro_use] mod macros;
mod modules;
mod options;
mod presence;
mod handler;
mod commands;

use crate::{
  types::{
    state::{ StateRef, GlobalConversationHistory },
    rss::RssSubscriber
  },
  handler::handle_event,
};

use std::sync::Arc;
use std::collections::{ HashMap, HashSet };
use std::time::{ Duration };

use tokio::sync::Mutex;

use smallvec::SmallVec;

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
use twilight_model::id::{ Id, marker::GuildMarker };

use tracing_subscriber::FmtSubscriber;
use tracing::Level;

#[tokio::main(worker_threads=16)]
async fn main() -> anyhow::Result<()> {
  let subscriber = FmtSubscriber::builder()
    .with_max_level(Level::INFO)
    .finish();
  tracing::subscriber::set_global_default(subscriber)?;

  let mut shard = Shard::new(
    ShardId::ONE,
    options::CONFIG.discord.clone(),
    Intents::GUILD_MESSAGES | Intents::MESSAGE_CONTENT,
  );

  let http = ClientBuilder::new()
                .token(options::CONFIG.discord.clone())
                .build();

  let cache = DefaultInMemoryCache::builder()
                .resource_types(ResourceType::MESSAGE)
                .build();

  let request_client = reqwest::Client::builder()
                .pool_max_idle_per_host(0)
                .build()?;

  let allowed_guilds: HashSet<Id<GuildMarker>> = options::CONFIG.allowed_guilds.clone()
                .into_iter()
                .map(|id| Id::new(id))
                .collect();

  let state = Arc::new(StateRef {
    http,
    shard_sender: shard.sender(),
    request_client,
    generation_lock: Arc::new(tokio::sync::Semaphore::new(1)),
    conversation_history: Arc::new(Mutex::new(HashMap::new())),
    global_conversation_history: Arc::new(Mutex::new(
      GlobalConversationHistory { messages: SmallVec::new() }
    )),
    allowed_guilds
  });

  tracing::info!("listening events");

  let rss_subscriber = RssSubscriber::new(
    Arc::clone(&state),
    Id::new(options::CONFIG.twitter_channel_id)
  );

  let _rss_handle = rss_subscriber.start(Duration::from_secs(3000));
  tracing::info!("Twitter RSS subscriber has started");

  while let Some(item) = shard.next_event(EventTypeFlags::all()).await {
    let Ok(event) = item else {
      tracing::warn!(source = ?item.unwrap_err(), "error receiving event");
      continue;
    };

    cache.update(&event);
    tokio::spawn(handle_event(event, Arc::clone(&state)));
  }

  rss_subscriber.stop();

  tracing::error!("Event loop terminated unexpectedly");

  Ok(())
}
