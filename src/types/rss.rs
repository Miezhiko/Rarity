use crate::{
  types::state::State
};

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
  pub state: State,
  pub channel_id: Id<ChannelMarker>
}
