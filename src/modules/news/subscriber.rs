use crate::{
  types::state::State,
  types::rss::*,
  options,
  modules::news::{ rss_fetcher, discord_poster }
};

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::task::JoinHandle;
use twilight_model::id::{Id, marker::{ChannelMarker}};
use tracing::{info, error};

use rss_fetcher::RssFetcher;
use discord_poster::DiscordPoster;

impl RssSubscriber {
  pub fn new(
    state: State,
    channel_id: Id<ChannelMarker>
  ) -> Self {
    Self { state, channel_id }
  }

  pub fn start(&self, poll_interval: Duration) -> JoinHandle<()> {
    let state = self.state.clone();
    let channel_id = self.channel_id;

    tokio::spawn(async move {
      let mut last_items: Vec<FeedItem> = Vec::new();
      let start_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

      info!("Starting RSS subscriber for new tweets after timestamp: {}", start_time);

      if options::CONFIG.test_subscriber {
        // Test mode: process 5 recent items on startup, ensuring one from Bing if available
        match RssFetcher::fetch_all_feeds(&state).await {
          Ok(current_items) => {
            let mut recent_items = current_items.clone();
            // Ensure at least one Bing item is included if available
            let bing_item = recent_items.iter()
              .find(|item| item.link.contains("bing.com"))
              .cloned();

            // Sort by timestamp descending
            recent_items.sort_by_key(|b| std::cmp::Reverse(b.published_timestamp));

            // Take 4 most recent items and add one Bing item if available
            let mut selected_items = recent_items.into_iter().take(4).collect::<Vec<_>>();
            if let Some(bing) = bing_item {
              if !selected_items.iter().any(|item| item.link.contains("bing.com")) {
                selected_items.push(bing);
              }
            }

            // Ensure we have at most 5 items
            selected_items.truncate(5);

            if !selected_items.is_empty() {
              info!("Test mode: Processing {} recent news items on startup", selected_items.len());

              if let Err(e) = DiscordPoster::post_to_discord(&state, channel_id, &selected_items).await {
                error!("Failed to post test news to Discord: {}", e);
              }
            }

            // Remember all current items to avoid reposting
            last_items = current_items;
          }
          Err(e) => {
            error!("Error fetching RSS feed for test mode: {}", e);
          }
        }
      }

      loop {
        match RssFetcher::fetch_all_feeds(&state).await {
          Ok(current_items) => {
            let new_items: Vec<FeedItem> = current_items
              .iter()
              .filter(|item| {
                item.published_timestamp > start_time &&
                !last_items.iter().any(|old_item| old_item.link == item.link)
              })
              .cloned()
              .collect();

            last_items = current_items;

            if !new_items.is_empty() {
              info!("Found {} new news items, combining all", new_items.len());

              if let Err(e) = DiscordPoster::post_to_discord(&state, channel_id, &new_items).await {
                error!("Failed to post news to Discord: {}", e);
              }
            }
          }
          Err(e) => {
            error!("Error fetching RSS feed: {}", e);
          }
        }

        tokio::time::sleep(poll_interval).await;
      }
    })
  }
}
