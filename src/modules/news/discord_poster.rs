use crate::{
  types::state::State,
  types::rss::*,
  options,
  ollama
};

use std::time::{SystemTime, UNIX_EPOCH};
use twilight_model::id::{Id, marker::{ChannelMarker}};
use tracing::{info, warn, error};
use twilight_model::util::Timestamp;
use twilight_util::builder::embed::{
  EmbedBuilder,
  EmbedFooterBuilder
};

const DISCORD_EMBED_TITLE_LIMIT: usize        = 256;
const DISCORD_EMBED_DESCRIPTION_LIMIT: usize  = 4050;
const DISCORD_EMBED_FOOTER_LIMIT: usize       = 666;
const DISCORD_EMBED_TOTAL_LIMIT: usize        = 6000;
const MAX_CHUNK_SIZE: usize                   = 3900;
const TRUNCATION_BUFFER: usize                = 50;
const CONTINUATION_SUFFIX: &str               = "...";
const CONTINUATION_PREFIX: &str               = " (часть ";

pub struct DiscordPoster;

impl DiscordPoster {
  pub async fn post_to_discord(
    state: &State,
    channel_id: Id<ChannelMarker>, 
    items: &[FeedItem]
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let valid_items: Vec<&FeedItem> = items
      .iter()
      .filter(|item| !item.title.trim().is_empty() && !item.description.trim().is_empty())
      .take(5)
      .collect();

    if valid_items.is_empty() {
      warn!("No valid items to post to Discord");
      return Ok(());
    }

    let all_titles: Vec<String> = valid_items
      .iter()
      .map(|item| Self::sanitize_discord_text(&item.title))
      .collect();
    let combined_titles = all_titles.join(", ");

    info!("got news: {combined_titles}");

    let all_descriptions: Vec<String> = valid_items
      .iter()
      .map(|item| Self::sanitize_discord_text(&item.description))
      .collect();
    let combined_descriptions = all_descriptions.join(". ");

    let generation_permit = match state.generation_lock.try_acquire() {
      Ok(permit) => permit,
      Err(_) => {
        warn!("Generation lock not available, skipping message generation");
        return Ok(());
      }
    };

    let result = (|| async {
      let rarity_response_title =
        ollama::generate_ollama_response_with_secondary( &combined_titles
                                                       , &options::CONFIG.title_mod_msg
                                                       , state ).await?;

      let rarity_response_desc =
        ollama::generate_ollama_response_with_secondary( &combined_descriptions
                                                       , &options::CONFIG.desc_mod_msg
                                                       , state ).await?;

      let mut title_no_q = Self::sanitize_discord_text(&Self::remove_quotes(&rarity_response_title));
      let sanitized_description = Self::sanitize_discord_text(&rarity_response_desc);

      // Safely truncate title if needed
      if title_no_q.chars().count() > DISCORD_EMBED_TITLE_LIMIT {
        title_no_q = Self::safe_truncate(&title_no_q, DISCORD_EMBED_TITLE_LIMIT);
        warn!("Title truncated to fit Discord limits: {} chars", title_no_q.chars().count());
      }

      if sanitized_description.chars().count() > DISCORD_EMBED_DESCRIPTION_LIMIT {
        warn!("Description too long ({}), splitting into multiple messages", sanitized_description.chars().count());
        Self::send_chunked_message( state
                                  , channel_id
                                  , &title_no_q
                                  , &sanitized_description
                                  , &valid_items ).await?;
      } else {
        info!("Sending single message with title length: {}, description length: {}", title_no_q.chars().count(), sanitized_description.chars().count());
        match Self::send_embed_message( state
                                      , channel_id
                                      , &title_no_q
                                      , &sanitized_description
                                      , &valid_items ).await {
          Ok(_) => info!("Single message sent successfully"),
          Err(e) => {
            error!("Failed to send single message: {}. Embed content: title='{}', description='{}', items={:?}",
                   e, title_no_q, sanitized_description, valid_items);
            return Err(e);
          }
        }
      }
      
      unsafe {
        options::GLOBAL.last_news = combined_descriptions;
      }

      info!("Posted combined news to Discord with {} items", valid_items.len());
      
      Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    })().await;

    drop(generation_permit);
    result
  }

  async fn send_chunked_message(
    state: &State,
    channel_id: Id<ChannelMarker>,
    title: &str,
    description: &str,
    items: &[&FeedItem]
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut chunks = Vec::new();
    let chars: Vec<char> = description.chars().collect();
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
    info!("Sending first chunk with title length: {}, description length: {}", title.chars().count(), first_description.chars().count());
    
    match Self::send_embed_message(state, channel_id, title, &first_description, items).await {
      Ok(_) => info!("First chunk sent successfully"),
      Err(e) => {
        error!("Failed to send first chunk: {}", e);
        return Err(e);
      }
    }

    for (i, chunk) in chunks.iter().skip(1).enumerate() {
      let continuation_title = format!("{}{}{})", title, CONTINUATION_PREFIX, i + 2);
      let safe_continuation_title = if continuation_title.chars()
                                                         .count() > DISCORD_EMBED_TITLE_LIMIT {
        Self::safe_truncate(&continuation_title, DISCORD_EMBED_TITLE_LIMIT)
      } else {
        continuation_title
      };

      info!("Sending continuation chunk {} with title length: {}, description length: {}", i + 2, safe_continuation_title.chars().count(), chunk.chars().count());
      
      match Self::send_embed_message( state
                                    , channel_id
                                    , &safe_continuation_title
                                    , chunk
                                    , items ).await {
        Ok(_) => info!("Continuation chunk {} sent successfully", i + 2),
        Err(e) => {
          error!("Failed to send continuation chunk {}: {}", i + 2, e);
          return Err(e);
        }
      }
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
    let sanitized_title       = Self::sanitize_discord_text(title);
    let sanitized_description = Self::sanitize_discord_text(description);
    let footer_text = format!("{} | v{}", options::CONFIG.footer_text, options::VERSION);

    if let Err(validation_error) =
        Self::validate_embed_content( &sanitized_title
                                    , &sanitized_description
                                    , &footer_text ) {
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
      .footer(EmbedFooterBuilder::new(footer_text).build())
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

  fn remove_quotes(s: &str) -> String {
    let mut result = s.to_string();

    if result.starts_with("**\"") && result.ends_with("\"**") {
      result = format!("**{}**", &result[3..result.len()-3]);
    } else if result.starts_with("**'") && result.ends_with("'**") {
      result = format!("**{}**", &result[3..result.len()-3]);
    } else if (result.starts_with('"') && result.ends_with('"')) || 
              (result.starts_with('\'') && result.ends_with('\'')) {
      result = result[1..result.len()-1].to_string();
    }
    
    result = result
      .chars()
      .map(|c| if c.is_control() { ' ' } else { c })
      .collect::<String>();

    result = result
      .split_whitespace()
      .collect::<Vec<&str>>()
      .join(" ")
      .trim()
      .to_string();

    info!("Title: '{result}'");
    
    result
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
    let normalized: String = text
      .chars()
      .map(|c| if c.is_control() { ' ' } else { c })
      .collect::<String>()
      .split_whitespace()
      .collect::<Vec<&str>>()
      .join(" ")
      .trim()
      .to_string();

    if normalized.chars().count() <= max_len {
      return normalized;
    }

    let truncate_at = max_len.saturating_sub(CONTINUATION_SUFFIX.len());
    let mut result: String = normalized.chars().take(truncate_at).collect();

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
      return Err(format!("Title too long: {} > {}", title.chars()
                                                         .count()
                                                  , DISCORD_EMBED_TITLE_LIMIT));
    }
    
    if description.chars().count() > DISCORD_EMBED_DESCRIPTION_LIMIT {
      return Err(format!("Description too long: {} > {}", description.chars()
                                                                     .count()
                                                        , DISCORD_EMBED_DESCRIPTION_LIMIT));
    }
    
    if footer.chars().count() > DISCORD_EMBED_FOOTER_LIMIT {
      return Err(format!("Footer too long: {} > {}", footer.chars()
                                                           .count()
                                                   , DISCORD_EMBED_FOOTER_LIMIT));
    }
    
    let total_length = title.chars().count() + description.chars().count() + footer.chars().count();
    if total_length > DISCORD_EMBED_TOTAL_LIMIT {
      return Err(format!("Total embed content too long: {} > {}", total_length
                                                                , DISCORD_EMBED_TOTAL_LIMIT));
    }
    
    Ok(())
  }
}
