use crate::{
  types::state::State,
  options
};

use std::time::{SystemTime, UNIX_EPOCH};
use regex::Regex;
use tracing::{info, error, warn};
use twilight_model::id::{Id, marker::{ChannelMarker}};
use twilight_model::channel::Message;
use twilight_model::util::Timestamp;
use twilight_util::builder::embed::{
  EmbedBuilder,
  EmbedFooterBuilder
};
use anyhow::Context;

// Discord uses UTF-16 code point limits, not character limits!
pub const DISCORD_EMBED_TITLE_LIMIT: usize        = 256;
pub const DISCORD_EMBED_DESCRIPTION_LIMIT: usize  = 4096;
pub const DISCORD_EMBED_FOOTER_LIMIT: usize       = 2048;
pub const DISCORD_EMBED_TOTAL_LIMIT: usize        = 6000;
pub const MAX_CHUNK_SIZE: usize                   = 3800;  // Safety margin for UTF-16
pub const TRUNCATION_BUFFER: usize                = 50;
pub const CONTINUATION_SUFFIX: &str               = "...";

// CRITICAL: Count UTF-16 code points, not characters
fn count_utf16(text: &str) -> usize {
  text.encode_utf16().count()
}

// CRITICAL: Truncate by UTF-16 code points
fn truncate_utf16(text: &str, max_utf16_len: usize) -> String {
  let mut utf16_count = 0;
  let mut result = String::new();
  
  for ch in text.chars() {
    let char_utf16_len = ch.len_utf16();
    if utf16_count + char_utf16_len > max_utf16_len {
      break;
    }
    result.push(ch);
    utf16_count += char_utf16_len;
  }
  
  result
}

pub fn remove_quotes(s: &str) -> String {
  let mut result = s.to_string();
  let quote_regex = Regex::new(r#"^\s*(\*\*)?["'](.*?)["'](\*\*)?\s*$"#).unwrap();
  
  if let Some(captures) = quote_regex.captures(&result) {
    let has_bold_start = captures.get(1).is_some();
    let content = captures.get(2).map_or("", |m| m.as_str());
    let has_bold_end = captures.get(3).is_some();
    
    if has_bold_start && has_bold_end {
      result = format!("**{}**", content);
    } else {
      result = content.to_string();
    }
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

pub fn sanitize_discord_text(text: &str) -> String {
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

pub fn safe_truncate(text: &str, max_len: usize) -> String {
  if max_len == 0 {
    return String::new();
  }
  
  let normalized: String = text
    .chars()
    .map(|c| if c.is_control() { ' ' } else { c })
    .collect::<String>()
    .split_whitespace()
    .collect::<Vec<&str>>()
    .join(" ")
    .trim()
    .to_string();

  // Use UTF-16 length for truncation
  let utf16_len = count_utf16(&normalized);
  if utf16_len <= max_len {
    return normalized;
  }

  let suffix_utf16_len = count_utf16(CONTINUATION_SUFFIX);
  let truncate_at = max_len.saturating_sub(suffix_utf16_len);
  if truncate_at == 0 {
    return CONTINUATION_SUFFIX.to_string();
  }
  
  let mut result = truncate_utf16(&normalized, truncate_at);

  // Try to break at word boundary
  if let Some(last_space) = result.rfind(' ') {
    let before_space = &result[..last_space];
    if count_utf16(before_space) > truncate_at.saturating_sub(TRUNCATION_BUFFER) {
      result.truncate(last_space);
    }
  }

  result.push_str(CONTINUATION_SUFFIX);
  result
}

pub fn validate_embed_content(title: &str, description: &str, footer: &str) -> Result<(), String> {
  let title_utf16 = count_utf16(title);
  let desc_utf16 = count_utf16(description);
  let footer_utf16 = count_utf16(footer);
  
  let title_has_content = !title.trim().is_empty();
  let desc_has_content = !description.trim().is_empty();
  
  if !title_has_content && !desc_has_content {
    return Err("Both title and description are empty or whitespace-only".to_string());
  }
  
  if title_utf16 > DISCORD_EMBED_TITLE_LIMIT {
    return Err(format!("Title too long: {} UTF-16 code points (limit: {})", 
                      title_utf16, DISCORD_EMBED_TITLE_LIMIT));
  }
  
  if desc_utf16 > DISCORD_EMBED_DESCRIPTION_LIMIT {
    return Err(format!("Description too long: {} UTF-16 code points (limit: {})", 
                      desc_utf16, DISCORD_EMBED_DESCRIPTION_LIMIT));
  }
  
  if footer_utf16 > DISCORD_EMBED_FOOTER_LIMIT {
    return Err(format!("Footer too long: {} UTF-16 code points (limit: {})", 
                      footer_utf16, DISCORD_EMBED_FOOTER_LIMIT));
  }
  
  let total_length = title_utf16 + desc_utf16 + footer_utf16;
  if total_length > DISCORD_EMBED_TOTAL_LIMIT {
    return Err(format!("Total embed content too long: {} > {} UTF-16 code points", 
                      total_length, DISCORD_EMBED_TOTAL_LIMIT));
  }
  
  Ok(())
}

pub fn create_embed_timestamp(timestamp_secs: Option<i64>) -> Timestamp {
  let timestamp_secs = timestamp_secs.unwrap_or_else(|| {
    SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap()
      .as_secs() as i64
  });

  Timestamp::from_secs(timestamp_secs)
    .unwrap_or_else(|_| {
      let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
      Timestamp::from_secs(now_secs).unwrap()
    })
}

pub fn build_embed(title: &str, description: &str) -> EmbedBuilder {
  build_embed_with_color(title, description, 0xFF69B4)
}

pub fn build_embed_with_color(title: &str, description: &str, color: u32) -> EmbedBuilder {
  let mut sanitized_title = sanitize_discord_text(title);
  let mut sanitized_description = sanitize_discord_text(description);
  let footer_text = format!("{} | v{}", options::CONFIG.footer_text, options::VERSION);

  if count_utf16(&sanitized_title) > DISCORD_EMBED_TITLE_LIMIT {
    sanitized_title = safe_truncate(&sanitized_title, DISCORD_EMBED_TITLE_LIMIT);
  }

  if count_utf16(&sanitized_description) > DISCORD_EMBED_DESCRIPTION_LIMIT {
    sanitized_description = safe_truncate(&sanitized_description, DISCORD_EMBED_DESCRIPTION_LIMIT);
  }

  let mut embed = EmbedBuilder::new()
    .color(color)
    .footer(EmbedFooterBuilder::new(footer_text).build());

  if !sanitized_title.is_empty() {
    embed = embed.title(sanitized_title);
  }

  if !sanitized_description.is_empty() {
    embed = embed.description(sanitized_description);
  }

  embed
}

pub async fn try_acquire_permit<'a>(state: &'a State, msg: &Message) ->
    anyhow::Result<Option<tokio::sync::SemaphorePermit<'a>>> {
  match state.generation_lock.try_acquire() {
    Ok(p) => Ok(Some(p)),
    Err(_) => {
      let embed = build_embed("", "Я занята, напиши минут через десять!")
        .timestamp(msg.timestamp)
        .build();

      state.http
        .create_message(msg.channel_id)
        .embeds(&[embed])
        .reply(msg.id)
        .await
        .context("Failed to send busy message")?;

      Ok(None)
    }
  }
}

pub async fn send_response(state: &State, msg: &Message, response: &str) -> anyhow::Result<()> {
  let embed = build_embed("", response)
    .timestamp(msg.timestamp)
    .build();

  state.http
    .create_message(msg.channel_id)
    .embeds(&[embed])
    .reply(msg.id)
    .await
    .context("Failed to send Discord message")?;

  Ok(())
}

pub async fn send_embed_message(
  state: &State,
  channel_id: Id<ChannelMarker>,
  title: &str,
  description: &str,
  timestamp: Option<i64>
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  let mut sanitized_title = sanitize_discord_text(title);
  let mut sanitized_description = sanitize_discord_text(description);
  
  let title_utf16 = count_utf16(&sanitized_title);
  let desc_utf16 = count_utf16(&sanitized_description);
  
  info!("After sanitization - Title: {} chars ({} UTF-16), Description: {} chars ({} UTF-16)",
        sanitized_title.chars().count(),
        title_utf16,
        sanitized_description.chars().count(),
        desc_utf16);
  
  if sanitized_title.trim().is_empty() && sanitized_description.trim().is_empty() {
    error!("Both title and description are empty after sanitization!");
    return Err("Cannot send embed with no content".into());
  }
  
  let footer_text = format!("{} | v{}", options::CONFIG.footer_text, options::VERSION);

  // Truncate using UTF-16 counts
  if title_utf16 > DISCORD_EMBED_TITLE_LIMIT {
    warn!("Truncating title from {} to {} UTF-16 code points", 
          title_utf16, DISCORD_EMBED_TITLE_LIMIT);
    sanitized_title = safe_truncate(&sanitized_title, DISCORD_EMBED_TITLE_LIMIT);
  }
  
  if desc_utf16 > DISCORD_EMBED_DESCRIPTION_LIMIT {
    warn!("Truncating description from {} to {} UTF-16 code points", 
          desc_utf16, DISCORD_EMBED_DESCRIPTION_LIMIT);
    sanitized_description = safe_truncate(&sanitized_description, DISCORD_EMBED_DESCRIPTION_LIMIT);
  }

  // Validate
  if let Err(validation_error) =
      validate_embed_content( &sanitized_title
                            , &sanitized_description
                            , &footer_text ) {
    error!("Embed validation failed: {}", validation_error);
    return Err(validation_error.into());
  }

  let embed_timestamp = create_embed_timestamp(timestamp);

  let mut embed = EmbedBuilder::new()
    .color(0xFF69B4)
    .timestamp(embed_timestamp)
    .footer(EmbedFooterBuilder::new(footer_text).build());

  if !sanitized_title.trim().is_empty() {
    embed = embed.title(sanitized_title.clone());
  }
  if !sanitized_description.trim().is_empty() {
    embed = embed.description(sanitized_description.clone());
  }

  let built_embed = embed.build();

  info!("Sending embed - title UTF-16: {}, description UTF-16: {}", 
        count_utf16(&sanitized_title),
        count_utf16(&sanitized_description));

  match state.http
    .create_message(channel_id)
    .embeds(&[built_embed])
    .await {
      Ok(_) => {
        info!("Successfully sent embed message");
        Ok(())
      }
      Err(e) => {
        error!("Failed to send embed message: {}", e);
        error!("Final UTF-16 counts - Title: {}, Description: {}", 
               count_utf16(&sanitized_title),
               count_utf16(&sanitized_description));
        Err(e.into())
      }
    }
}