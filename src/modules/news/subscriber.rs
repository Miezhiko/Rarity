use crate::{
  types::state::State,
  types::rss::*,
  options,
  modules::news::{ rss_fetcher, discord_poster }
};

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use twilight_model::id::{Id, marker::{ChannelMarker}};
use tracing::{info, error};

use rss_fetcher::RssFetcher;
use discord_poster::DiscordPoster;

impl RssSubscriber {
  pub fn new(
    state: State,
    channel_id: Id<ChannelMarker>
  ) -> Self {
    Self {
      last_items: Arc::new(Mutex::new(Vec::new())),
      running: Arc::new(Mutex::new(false)),
      state,
      channel_id
    }
  }

  pub fn start(&self, poll_interval: Duration) -> thread::JoinHandle<()> {
    let last_items = Arc::clone(&self.last_items);
    let running = Arc::clone(&self.running);
    let state = Arc::clone(&self.state);
    let channel_id = self.channel_id;

    {
      let mut running_guard = running.lock().unwrap();
      *running_guard = true;
    }

    thread::spawn(move || {
      let rt = tokio::runtime::Runtime::new().unwrap();
      let start_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

      info!("Starting RSS subscriber for new tweets after timestamp: {}", start_time);

      if options::CONFIG.test_subscriber {
        // Test mode: process 5 recent items on startup, ensuring one from Bing if available
        match rt.block_on(RssFetcher::fetch_all_feeds(&state)) {
          Ok(current_items) => {
            let mut recent_items = current_items.clone();
            // Ensure at least one Bing item is included if available
            let mut bing_item = None;
            for item in &recent_items {
              if item.link.contains("bing.com") {
                bing_item = Some(item.clone());
                break;
              }
            }
            
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
              
              if let Err(e) = rt.block_on(DiscordPoster::post_to_discord(&state, channel_id, &selected_items)) {
                error!("Failed to post test news to Discord: {}", e);
              }
            }
            
            // Update last_items with all current items to avoid reposting
            {
              let mut last_items_guard = last_items.lock().unwrap();
              *last_items_guard = current_items;
            }
          }
          Err(e) => {
            error!("Error fetching RSS feed for test mode: {}", e);
          }
        }
      }

      loop {
        {
          let running_guard = running.lock().unwrap();
          if !*running_guard {
            break;
          }
        }

        match rt.block_on(RssFetcher::fetch_all_feeds(&state)) {
          Ok(current_items) => {
            let new_items = {
              let mut last_items_guard = last_items.lock().unwrap();
              
              let new_items: Vec<FeedItem> = current_items
                .iter()
                .filter(|item| {
                  item.published_timestamp > start_time &&
                  !last_items_guard.iter().any(|old_item| old_item.link == item.link)
                })
                .cloned()
                .collect();
              
              *last_items_guard = current_items;
              
              new_items
            };

            if !new_items.is_empty() {
              info!("Found {} new news items, combining all", new_items.len());
              
              if let Err(e) = rt.block_on(DiscordPoster::post_to_discord(&state, channel_id, &new_items)) {
                error!("Failed to post news to Discord: {}", e);
              }
            }
          }
          Err(e) => {
            error!("Error fetching RSS feed: {}", e);
          }
        }

        thread::sleep(poll_interval);
      }
      
      info!("RSS subscriber thread stopped");
    })
  }

  #[allow(dead_code)]
  pub fn stop(&self) {
    let mut running_guard = self.running.lock().unwrap();
    *running_guard = false;
    info!("Stopping RSS subscriber...");
  }
}
