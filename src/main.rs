#[allow(unused_imports)]
#[macro_use] extern crate anyhow;

mod types;
mod modules;
mod options;
mod presence;
mod handler;
mod commands;
mod proxy;

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
  ConfigBuilder,
  EventTypeFlags,
  Intents, Shard,
  ShardId,
  StreamExt as _
};

use twilight_http::client::ClientBuilder;
use twilight_model::id::{ Id, marker::GuildMarker };

use tracing_subscriber::FmtSubscriber;
use tracing::Level;

/// Connects a new shard, using a self-hosted gateway proxy (via
/// `DISCORD_GATEWAY_PROXY_URL`) instead of connecting to Discord directly
/// when one is configured.
fn connect_shard(id: ShardId) -> Shard {
  let mut config_builder = ConfigBuilder::new(
    options::CONFIG.discord.clone(),
    Intents::GUILD_MESSAGES | Intents::MESSAGE_CONTENT,
  );
  if let Some(proxy_url) = proxy::env_discord_gateway_proxy_url() {
    config_builder = config_builder.proxy_url(proxy_url);
  }
  Shard::with_config(id, config_builder.build())
}

#[tokio::main(worker_threads=16)]
async fn main() -> anyhow::Result<()> {
  rustls::crypto::ring::default_provider()
      .install_default()
      .expect("Failed to install crypto provider");

  let subscriber = FmtSubscriber::builder()
    .with_max_level(Level::INFO)
    .finish();
  tracing::subscriber::set_global_default(subscriber)?;

  let mut shard = connect_shard(ShardId::ONE);

  let mut http_builder = ClientBuilder::new()
                .token(options::CONFIG.discord.clone());
  if let Some((proxy_url, use_http)) = proxy::env_discord_http_proxy() {
    http_builder = http_builder.proxy(proxy_url, use_http);
  }
  let http = http_builder.build();

  let cache = DefaultInMemoryCache::builder()
                .resource_types(ResourceType::MESSAGE)
                .build();

  let request_client = proxy::build_request_client(
    proxy::env_proxy_url().as_deref(),
    &proxy::env_no_proxy()
  )?;

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

  let _rss_handle = rss_subscriber.start(Duration::from_secs(6000));
  tracing::info!("Twitter RSS subscriber has started");

  loop {
    tracing::info!("Connecting to Discord gateway...");

    while let Some(item) = shard.next_event(EventTypeFlags::all()).await {
      let Ok(event) = item else {
        tracing::warn!(source = ?item.unwrap_err(), "error receiving event");
        continue;
      };

      cache.update(&event);
      tokio::spawn(handle_event(event, Arc::clone(&state)));
    }

    tracing::warn!("Discord gateway disconnected, reconnecting in 5 seconds...");
    tokio::time::sleep(Duration::from_secs(5)).await;

    shard = connect_shard(ShardId::ONE);
  }
}
