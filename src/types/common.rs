use std::sync::Arc;
use std::collections::HashMap;

use serde::Deserialize;

use twilight_http::Client as HttpClient;
use reqwest::Client as Reqwest;

use tokio::sync::Mutex;

#[derive(Clone, Deserialize, Debug)]
pub struct IOptions {
  pub discord: String,
  pub system_prompt: String,
  pub footer_text: String
}

#[derive(Clone, Debug)]
pub struct ConversationHistory {
  pub messages: Vec<(String, String)>
}

#[derive(Clone, Debug)]
pub struct PersonalityConfig {
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
  pub personality: PersonalityConfig
}

pub type State = Arc<StateRef>;
