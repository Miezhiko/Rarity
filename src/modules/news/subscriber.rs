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

const DISCORD_EMBED_TITLE_LIMIT: usize = 256;
const DISCORD_EMBED_DESCRIPTION_LIMIT: usize = 4050;
const DISCORD_EMBED_FOOTER_LIMIT: usize = 666;
const DISCORD_EMBED_TOTAL_LIMIT: usize = 6000;
const MAX_CHUNK_SIZE: usize = 3900;

const TRUNCATION_BUFFER: usize = 50;
const CONTINUATION_SUFFIX: &str = "...";
const CONTINUATION_PREFIX: &str = " (часть ";

fn remove_quotes(s: &str) -> String {
  s.strip_prefix('"')
   .and_then(|stripped| stripped.strip_suffix('"'))
   .map(|stripped| stripped.to_string())
   .unwrap_or_else(|| s.to_string())
}

fn sanitize_discord_text(text: &str) -> String {
  text
    .chars()
    .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
    .filter(|c| {
      match *c as u32 {
        0x200B..=0x200F |  // Zero-width space, zero-width non-joiner, etc.
        0x202A..=0x202E |  // Directional formatting
        0xFEFF => false,   // Byte Order Mark
        _ => true,
      }
    })
    .collect::<String>()
    .trim()
    .to_string()
}

fn safe_truncate(text: &str, max_len: usize) -> String {
  if text.chars().count() <= max_len {
    return text.to_string();
  }
  
  let truncate_at = max_len.saturating_sub(CONTINUATION_SUFFIX.len());
  let mut result: String = text.chars().take(truncate_at).collect();
  
  // Ensure we don't break in the middle of a word
  if let Some(last_space) = result.rfind(' ') {
    if last_space > truncate_at.saturating_sub(TRUNCATION_BUFFER) {
      result.truncate(last_space);
    }
  }
  
  result.push_str(CONTINUATION_SUFFIX);
  result
}

fn validate_embed_content(title: &str, description: &str, footer: &str) -> Result<(), String> {
  if title.trim().is_empty() {
    return Err("Title cannot be empty".to_string());
  }
  
  if description.trim().is_empty() {
    return Err("Description cannot be empty".to_string());
  }
  
  if title.chars().count() > DISCORD_EMBED_TITLE_LIMIT {
    return Err(format!("Title too long: {} > {}", title.chars().count(), DISCORD_EMBED_TITLE_LIMIT));
  }
  
  if description.chars().count() > DISCORD_EMBED_DESCRIPTION_LIMIT {
    return Err(format!("Description too long: {} > {}", description.chars().count(), DISCORD_EMBED_DESCRIPTION_LIMIT));
  }
  
  if footer.chars().count() > DISCORD_EMBED_FOOTER_LIMIT {
    return Err(format!("Footer too long: {} > {}", footer.chars().count(), DISCORD_EMBED_FOOTER_LIMIT));
  }
  
  let total_length = title.chars().count() + description.chars().count() + footer.chars().count();
  if total_length > DISCORD_EMBED_TOTAL_LIMIT {
    return Err(format!("Total embed content too long: {} > {}", total_length, DISCORD_EMBED_TOTAL_LIMIT));
  }
  
  Ok(())
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
        match rt.block_on(Self::fetch_rss(&state)) {
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
            recent_items.sort_by(|a, b| b.published_timestamp.cmp(&a.published_timestamp));
            
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
              
              if let Err(e) = rt.block_on(Self::post_to_discord(&state, channel_id, &selected_items)) {
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
              
              if let Err(e) = rt.block_on(Self::post_to_discord(&state, channel_id, &new_items)) {
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
      ("bing",      "https://www.bing.com/news/search?q=%D0%BA%D0%B2%D0%B0%D0%B4%D1%80%D0%BE%D0%B1%D0%B5%D1%80%D1%8B&format=rss"),
      ("standard",  "https://www.themoscowtimes.com/rss/news"),
      ("standard",  "https://lenta.ru/rss/google-newsstand/main"),
      ("standard",  "https://meduza.io/rss/all")
    ];

    let mut all_items = Vec::new();
    let mut successful_fetches = 0;

    for (feed_type, rss_url) in &news_instances {
      info!("Attempting to fetch from {} ({})", rss_url, feed_type);
      match *feed_type {
        "bing" => {
          match Self::try_fetch_from_bing(state, rss_url).await {
            Ok(items) => {
              info!("Successfully fetched {} items from Bing: {}", items.len(), rss_url);
              all_items.extend(items);
              successful_fetches += 1;
            }
            Err(e) => {
              warn!("Failed to fetch from Bing {}: {}", rss_url, e);
            }
          }
        }
        _ => {
          match Self::try_fetch_standard(state, rss_url).await {
            Ok(items) => {
              info!("Successfully fetched {} items from: {}", items.len(), rss_url);
              all_items.extend(items);
              successful_fetches += 1;
            }
            Err(e) => {
              warn!("Failed to fetch from {}: {}", rss_url, e);
            }
          }
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

  async fn try_fetch_standard(state: &State, rss_url: &str) -> Result<Vec<FeedItem>, Box<dyn std::error::Error + Send + Sync>> {
    let response = state.request_client
      .get(rss_url)
      .header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36")
      .header("Accept", "application/rss+xml, application/xml, text/xml")
      .timeout(Duration::from_secs(3))
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
          
          let title = entry.title
            .map(|t| sanitize_discord_text(&t.content))
            .unwrap_or_else(|| "No title".to_string());
          
          let link = entry.links.first()
            .map(|l| l.href.clone())
            .unwrap_or_else(|| "No link".to_string());
          
          let description = entry.content
            .and_then(|c| c.body)
            .or_else(|| entry.summary.map(|s| s.content))
            .map(|d| sanitize_discord_text(&d))
            .unwrap_or_else(|| "No description".to_string());

          let item = FeedItem {
            title,
            link,
            description,
            published_timestamp
          };
          items.push(item);
        }
      }
      Err(e) => {
        error!("Unable to parse standard feed {}: {}, content:\n{}", rss_url, e, content);
        return Err(format!("Failed to parse standard feed: {}", e).into());
      }
    }

    Ok(items)
  }

  async fn try_fetch_from_bing(state: &State, rss_url: &str) -> Result<Vec<FeedItem>, Box<dyn std::error::Error + Send + Sync>> {
    let response = state.request_client
      .get(rss_url)
      .header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36")
      .header("Accept", "application/rss+xml, application/xml, text/xml")
      .timeout(Duration::from_secs(3))
      .send()
      .await?;
    
    if !response.status().is_success() {
      return Err(format!("Bing HTTP {}", response.status()).into());
    }
    
    let content = response.text().await?;
    let mut items = Vec::new();

    match parser::parse(content.as_bytes()) {
      Ok(feed) => {
        for entry in feed.entries.into_iter().take(10) {
          let published_timestamp = entry.published
            .map(|dt| dt.timestamp() as u64)
            .unwrap_or_else(|| {
              SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
            });
          
          let title = entry.title
            .map(|t| sanitize_discord_text(&t.content))
            .unwrap_or_else(|| "No title".to_string());
          
          let link = entry.links.iter()
            .find(|l| !l.href.is_empty())
            .map(|l| l.href.clone())
            .unwrap_or_else(|| "No link".to_string());
          
          let description = entry.summary
            .map(|s| sanitize_discord_text(&s.content))
            .or_else(|| entry.content.and_then(|c| c.body.map(|b| sanitize_discord_text(&b))))
            .unwrap_or_else(|| "No description".to_string());

          let item = FeedItem {
            title,
            link,
            description,
            published_timestamp
          };
          items.push(item);
        }
      }
      Err(e) => {
        error!("Unable to parse Bing feed {}: {}, content:\n{}", rss_url, e, content);
        return Err(format!("Failed to parse Bing feed: {}", e).into());
      }
    }

    Ok(items)
  }

  async fn post_to_discord(
    state: &State,
    channel_id: Id<ChannelMarker>, 
    items: &[FeedItem]
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Filter out empty items and sanitize content
    let valid_items: Vec<&FeedItem> = items
      .iter()
      .filter(|item| !item.title.trim().is_empty() && !item.description.trim().is_empty())
      .collect();

    if valid_items.is_empty() {
      warn!("No valid items to post to Discord");
      return Ok(());
    }

    // Concatenate all titles
    let all_titles: Vec<String> = valid_items
      .iter()
      .map(|item| sanitize_discord_text(&item.title))
      .collect();
    let combined_titles = all_titles.join(", ");

    // Concatenate all descriptions
    let all_descriptions: Vec<String> = valid_items
      .iter()
      .map(|item| sanitize_discord_text(&item.description))
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

      let mut title_no_q = sanitize_discord_text(&remove_quotes(&rarity_response_title));
      let sanitized_description = sanitize_discord_text(&rarity_response_desc);

      // Safely truncate title if needed
      if title_no_q.chars().count() > DISCORD_EMBED_TITLE_LIMIT {
        title_no_q = safe_truncate(&title_no_q, DISCORD_EMBED_TITLE_LIMIT);
        warn!("Title truncated to fit Discord limits: {} chars", title_no_q.chars().count());
      }

      if sanitized_description.chars().count() > DISCORD_EMBED_DESCRIPTION_LIMIT {
        warn!("Description too long ({}), splitting into multiple messages", sanitized_description.chars().count());
        
        let mut chunks = Vec::new();
        let chars: Vec<char> = sanitized_description.chars().collect();
        let mut current_pos = 0;
        
        while current_pos < chars.len() {
          let remaining = chars.len() - current_pos;
          let chunk_size = std::cmp::min(MAX_CHUNK_SIZE, remaining);
          let mut end_pos = current_pos + chunk_size;
          
          // Try to break at word boundary if not at the end
          if end_pos < chars.len() {
            let search_start = std::cmp::max(current_pos, end_pos.saturating_sub(TRUNCATION_BUFFER));
            if let Some(space_pos) = chars[search_start..end_pos].iter().rposition(|&c| c == ' ') {
              end_pos = search_start + space_pos;
            }
          }
          
          let mut chunk: String = chars[current_pos..end_pos].iter().collect();
          
          if end_pos < chars.len() {
            chunk.push_str(CONTINUATION_SUFFIX);
          }
          
          chunks.push(chunk);
          current_pos = end_pos;
          
          // Skip whitespace at the beginning of next chunk
          while current_pos < chars.len() && chars[current_pos].is_whitespace() {
            current_pos += 1;
          }
        }

        let first_description = chunks.first().unwrap_or(&String::new()).clone();
        info!("Sending first chunk with title length: {}, description length: {}", title_no_q.chars().count(), first_description.chars().count());
        
        match Self::send_embed_message(state, channel_id, &title_no_q, &first_description, &valid_items).await {
          Ok(_) => info!("First chunk sent successfully"),
          Err(e) => {
            error!("Failed to send first chunk: {}", e);
            return Err(e);
          }
        }

        for (i, chunk) in chunks.iter().skip(1).enumerate() {
          let continuation_title = format!("{}{}{})", &title_no_q, CONTINUATION_PREFIX, i + 2);
          let safe_continuation_title = if continuation_title.chars().count() > DISCORD_EMBED_TITLE_LIMIT {
            safe_truncate(&continuation_title, DISCORD_EMBED_TITLE_LIMIT)
          } else {
            continuation_title
          };

          info!("Sending continuation chunk {} with title length: {}, description length: {}", i + 2, safe_continuation_title.chars().count(), chunk.chars().count());
          
          match Self::send_embed_message(state, channel_id, &safe_continuation_title, chunk, &valid_items).await {
            Ok(_) => info!("Continuation chunk {} sent successfully", i + 2),
            Err(e) => {
              error!("Failed to send continuation chunk {}: {}", i + 2, e);
              return Err(e);
            }
          }
        }
      } else {
        info!("Sending single message with title length: {}, description length: {}", title_no_q.chars().count(), sanitized_description.chars().count());
        match Self::send_embed_message(state, channel_id, &title_no_q, &sanitized_description, &valid_items).await {
          Ok(_) => info!("Single message sent successfully"),
          Err(e) => {
            error!("Failed to send single message: {}", e);
            return Err(e);
          }
        }
      }
      
      unsafe {
        options::GLOBAL.last_news = combined_descriptions;
      }

      info!("Posted combined news to Discord with {} items", valid_items.len());

      drop(p)
    }
    
    Ok(())
  }

  async fn send_embed_message(
    state: &State,
    channel_id: Id<ChannelMarker>,
    title: &str,
    description: &str,
    items: &[&FeedItem]
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let sanitized_title = sanitize_discord_text(title);
    let sanitized_description = sanitize_discord_text(description);
    let sanitized_footer = sanitize_discord_text(&options::CONFIG.footer_text);

    // Validate embed content before sending
    if let Err(validation_error) = validate_embed_content(&sanitized_title, &sanitized_description, &sanitized_footer) {
      error!("Embed validation failed: {}", validation_error);
      return Err(validation_error.into());
    }

    let latest_timestamp = items
        .iter()
        .map(|item| item.published_timestamp)
        .max()
        .unwrap_or(0) as i64;

    let timestamp = if latest_timestamp > 0 {
        Timestamp::from_secs(latest_timestamp)
          .unwrap_or_else(|_| {
            let now_secs = SystemTime::now()
              .duration_since(UNIX_EPOCH)
              .unwrap()
              .as_secs() as i64;
            Timestamp::from_secs(now_secs).unwrap()
          })
      } else {
        let now_secs = SystemTime::now()
          .duration_since(UNIX_EPOCH)
          .unwrap()
          .as_secs() as i64;
        Timestamp::from_secs(now_secs).unwrap()
      };

    let embed = EmbedBuilder::new()
      .title(sanitized_title)
      .description(sanitized_description)
      .color(0xFF69B4)
      .timestamp(timestamp)
      .footer(EmbedFooterBuilder::new(sanitized_footer).build())
      .build();

    match state.http
      .create_message(channel_id)
      .embeds(&[embed])
      .await {
        Ok(_) => {
          info!("Successfully sent embed message");
          Ok(())
        }
        Err(e) => {
          error!("Failed to send embed message: {}", e);
          Err(e.into())
        }
      }
  }
}
