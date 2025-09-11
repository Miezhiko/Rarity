use std::{ collections::{ HashMap, HashSet }
         , sync::Arc };

use twilight_http::Client as HttpClient;
use twilight_model::id::{ Id, marker::GuildMarker };

use reqwest::Client as Reqwest;

use tokio::sync::{ Mutex, Semaphore };

use smallvec::SmallVec;

use twilight_gateway::MessageSender;

type MessageTuple = (Arc<str>, Arc<str>, Arc<str>);
type GlobalMessageTuple = (Arc<str>, Arc<str>);

#[derive(Clone)]
pub struct ConversationHistory {
  pub messages: SmallVec<[MessageTuple; 8]>
}

#[derive(Clone)]
pub struct GlobalConversationHistory {
  pub messages: SmallVec<[GlobalMessageTuple; 16]>
}

pub struct StateRef {
  pub http: HttpClient,
  pub shard_sender: MessageSender,
  pub request_client: Reqwest,
  pub generation_lock: Arc<Semaphore>,
  pub conversation_history: Arc<Mutex<HashMap<String, ConversationHistory>>>,
  pub global_conversation_history: Arc<Mutex<GlobalConversationHistory>>,
  pub allowed_guilds: HashSet<Id<GuildMarker>>
}

pub struct GlobalState {
  pub last_news: String
}

pub type State = Arc<StateRef>;
