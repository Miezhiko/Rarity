use crate::{
  types::state::State,
  types::rss::*,
  options,
  ollama
};

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use feed_rs::parser;

use tokio;

use twilight_model::id::{Id, marker::{ChannelMarker}};
use tracing::{info, warn, error};
use twilight_model::util::Timestamp;

use twilight_util::builder::embed::{
  EmbedBuilder,
  EmbedFooterBuilder
};

fn remove_quotes(s: &str) -> String {
  s.strip_prefix('"')
   .and_then(|stripped| stripped.strip_suffix('"'))
   .map(|stripped| stripped.to_string())
   .unwrap_or_else(|| s.to_string())
}

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
    set! { last_items   = Arc::clone(&self.last_items)
         , running      = Arc::clone(&self.running)
         , state        = Arc::clone(&self.state)
         , channel_id   = self.channel_id };

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
      
      loop {
        {
          let running_guard = running.lock().unwrap();
          if !*running_guard {
            break;
          }
        }

        match rt.block_on(Self::fetch_rss(&state)) {
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
              
              if let Err(e) = rt.block_on(Self::post_to_discord( &state
                                                               , channel_id
                                                               , &new_items )) {
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

  pub fn stop(&self) {
    let mut running_guard = self.running.lock().unwrap();
    *running_guard = false;
    info!("Stopping RSS subscriber...");
  }

  async fn fetch_rss(state: &State) -> Result<Vec<FeedItem>, Box<dyn std::error::Error + Send + Sync>> {
    let news_instances = vec![
      "https://www.themoscowtimes.com/rss/news",
      "https://lenta.ru/rss/google-newsstand/main",
      "https://meduza.io/rss/all"
    ];

    setm! { all_items = Vec::new()
          , successful_fetches = 0 };

    for rss_url in &news_instances {
      match Self::try_fetch_from_instance(state, &rss_url).await {
        Ok(items) => {
          info!("Successfully fetched {} items from: {}", items.len(), &rss_url);
          all_items.extend(items);
          successful_fetches += 1;
        }
        Err(e) => {
          warn!("Failed to fetch from {}: {}", &rss_url, e);
        }
      }
    }

    if successful_fetches > 0 {
      info!("Total items fetched from {} sources: {}", successful_fetches, all_items.len());
      Ok(all_items)
    } else {
      Err("All RSS instances failed".into())
    }
  }

  async fn try_fetch_from_instance(state: &State, rss_url: &str) -> Result<Vec<FeedItem>, Box<dyn std::error::Error + Send + Sync>> {
    let response = state.request_client
      .get(rss_url)
      .header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36")
      .header("Accept", "application/rss+xml, application/xml, text/xml")
      .send()
      .await?;
    
    if !response.status().is_success() {
      return Err(format!("HTTP {}", response.status()).into());
    }
    
    let content = response.text().await?;
    let mut items = Vec::new();

    match parser::parse(content.as_bytes()) {
      Ok(feed) => {
        for entry in feed.entries {
          let published_timestamp = entry.published
            .map(|dt| dt.timestamp() as u64)
            .unwrap_or(0);
          
          let item = FeedItem {
            title: entry.title
              .map(|t| t.content)
              .unwrap_or_else(|| "No title".to_string()),
            link: entry.links.first()
              .map(|l| l.href.clone())
              .unwrap_or_else(|| "No link".to_string()),
            description: entry.content
              .and_then(|c| c.body)
              .or_else(|| entry.summary.map(|s| s.content))
              .unwrap_or_else(|| "No description".to_string()),
            published_timestamp
          };
          items.push(item);
        }
      }, Err(e) => {
        error!("unable to parse: {e}, full rss:\n{content}")
      }
    }

    Ok(items)
  }

  async fn post_to_discord(
    state: &State,
    channel_id: Id<ChannelMarker>, 
    items: &[FeedItem]
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {

    let all_titles: Vec<String> = items
      .iter()
      .map(|item| item.title.clone())
      .collect();
    let combined_titles = all_titles.join(", ");

    let all_descriptions: Vec<String> = items
      .iter()
      .map(|item| item.description.clone())
      .collect();
    let combined_descriptions = all_descriptions.join(". ");

    let message_title = format!(
      "{}: {}",
      &options::CONFIG.title_mod_msg, &combined_titles
    );

    let message_desc = format!(
      "{}: {}",
      &options::CONFIG.desc_mod_msg, &combined_descriptions
    );

    if let Ok(p) = state.generation_lock.try_acquire() {

      let rarity_response_title =
        ollama::generate_ollama_response(&message_title, state).await?;

      let rarity_response_desc =
        ollama::generate_ollama_response(&message_desc, state).await?;

      let title_no_q = remove_quotes(&rarity_response_title);
      
      let latest_timestamp = items
        .iter()
        .map(|item| item.published_timestamp)
        .max()
        .unwrap_or(0) as i64;
      
      let timestamp = Timestamp::from_secs(latest_timestamp)
          .unwrap_or_else(|_| {
              let now_secs = SystemTime::now()
                  .duration_since(UNIX_EPOCH)
                  .unwrap()
                  .as_secs() as i64;
              Timestamp::from_secs(now_secs).unwrap()
          });

      let embed = EmbedBuilder::new()
        .title(&title_no_q)
        .description(&rarity_response_desc)
        .color(0xFF69B4)
        .timestamp(timestamp)
        .footer(EmbedFooterBuilder::new(&options::CONFIG.footer_text).build())
        .build();

      state.http
        .create_message(channel_id)
        .embeds(&[embed])
        .await?;
      
      unsafe {
        options::GLOBAL.last_news = combined_descriptions;
      }

      info!("Posted combined news to Discord with {} items", items.len());

      drop(p)
    }
    
    Ok(())
  }
}
