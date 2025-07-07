use std::sync::Arc;
use std::collections::{ HashMap, HashSet };

use serde::Deserialize;

use twilight_http::Client as HttpClient;
use twilight_model::id::{ Id, marker::GuildMarker };

use reqwest::Client as Reqwest;

use tokio::sync::Mutex;

#[derive(Clone, Deserialize, Debug)]
pub struct IOptions {
  pub discord: String,
  pub model: String,
  pub system_prompt: String,
  pub footer_text: String,
  pub allowed_guilds: Vec<u64>
}

#[derive(Clone, Debug)]
pub struct ConversationHistory {
  pub messages: Vec<(String, String)>
}

#[derive(Clone, Debug)]
pub struct PersonalityConfig {
  pub model: String,
  pub system_prompt: String,
  pub embed_color: u32,
  pub footer_text: String
}

#[derive(Debug)]
pub struct StateRef {
  pub http: HttpClient,
  pub request_client: Reqwest,
  pub generation_lock: Arc<tokio::sync::Semaphore>,
  pub conversation_history: Arc<Mutex<HashMap<String, ConversationHistory>>>,
  pub personality: PersonalityConfig,
  pub allowed_guilds: HashSet<Id<GuildMarker>>
}

pub type State = Arc<StateRef>;
