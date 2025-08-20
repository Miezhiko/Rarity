use std::{ collections::{ HashMap, HashSet }
         , sync::Arc };

use twilight_http::Client as HttpClient;
use twilight_model::id::{ Id, marker::GuildMarker };

use reqwest::Client as Reqwest;

use tokio::sync::{ Mutex, Semaphore };

use smallvec::SmallVec;

type MessageTuple = (Arc<str>, Arc<str>, Arc<str>);
type GlobalMessageTuple = (Arc<str>, Arc<str>);

#[derive(Clone, Debug)]
pub struct ConversationHistory {
  pub messages: SmallVec<[MessageTuple; 8]>
}

#[derive(Clone, Debug)]
pub struct GlobalConversationHistory {
  pub messages: SmallVec<[GlobalMessageTuple; 16]>
}

#[derive(Debug)]
pub struct StateRef {
  pub http: HttpClient,
  pub request_client: Reqwest,
  pub generation_lock: Arc<Semaphore>,
  pub conversation_history: Arc<Mutex<HashMap<String, ConversationHistory>>>,
  pub global_conversation_history: Arc<Mutex<GlobalConversationHistory>>,
  pub allowed_guilds: HashSet<Id<GuildMarker>>
}

pub type State = Arc<StateRef>;
