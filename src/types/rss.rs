use crate::{
  types::state::State
};

use std::sync::{ Arc, Mutex };

use twilight_model::id::{ Id, marker::{ChannelMarker} };

#[derive(Clone, Debug)]
pub struct FeedItem {
  pub title: String,
  pub link: String,
  pub description: String,
  pub published_timestamp: u64
}

#[derive(Clone)]
pub struct RssSubscriber {
  pub last_items: Arc<Mutex<Vec<FeedItem>>>,
  pub running: Arc<Mutex<bool>>,
  pub state: State,
  pub channel_id: Id<ChannelMarker>
}
