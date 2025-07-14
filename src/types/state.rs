use std::sync::Arc;
use std::collections::{ HashMap, HashSet };

use twilight_http::Client as HttpClient;
use twilight_model::id::{ Id, marker::GuildMarker };

use reqwest::Client as Reqwest;

use tokio::sync::Mutex;

#[derive(Clone, Debug)]
pub struct ConversationHistory {
  pub messages: Vec<(String, String)>
}

#[derive(Debug)]
pub struct StateRef {
  pub http: HttpClient,
  pub request_client: Reqwest,
  pub generation_lock: Arc<tokio::sync::Semaphore>,
  pub conversation_history: Arc<Mutex<HashMap<String, ConversationHistory>>>,
  pub global_conversation_history: Arc<Mutex<ConversationHistory>>,
  pub allowed_guilds: HashSet<Id<GuildMarker>>
}

pub type State = Arc<StateRef>;
