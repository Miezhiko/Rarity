use crate::{
  types::state::State,
  options
};

use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use tracing::{info, error};
use twilight_model::id::{Id, marker::{ChannelMarker}};
use twilight_model::channel::Message;
use twilight_model::util::Timestamp;
use twilight_util::builder::embed::{
  EmbedBuilder,
  EmbedFooterBuilder
};

use anyhow::Context;

pub const DISCORD_EMBED_TITLE_LIMIT: usize        = 256;
pub const DISCORD_EMBED_DESCRIPTION_LIMIT: usize  = 4050;
pub const DISCORD_EMBED_FOOTER_LIMIT: usize       = 666;
pub const DISCORD_EMBED_TOTAL_LIMIT: usize        = 6000;
pub const MAX_CHUNK_SIZE: usize                   = 3900;
pub const TRUNCATION_BUFFER: usize                = 50;
pub const CONTINUATION_SUFFIX: &str               = "...";

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

pub fn validate_embed_content(title: &str, description: &str, footer: &str) -> Result<(), String> {
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
  let sanitized_title = sanitize_discord_text(title);
  let sanitized_description = sanitize_discord_text(description);
  let footer_text = format!("{} | v{}", options::CONFIG.footer_text, options::VERSION);

  EmbedBuilder::new()
    .title(sanitized_title)
    .description(sanitized_description)
    .color(0xFF69B4)
    .footer(EmbedFooterBuilder::new(footer_text).build())
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
  let sanitized_title = sanitize_discord_text(title);
  let sanitized_description = sanitize_discord_text(description);
  let footer_text = format!("{} | v{}", options::CONFIG.footer_text, options::VERSION);

  if let Err(validation_error) =
      validate_embed_content( &sanitized_title
                            , &sanitized_description
                            , &footer_text ) {
    error!("Embed validation failed: {}", validation_error);
    return Err(validation_error.into());
  }

  let embed_timestamp = create_embed_timestamp(timestamp);

  let embed = EmbedBuilder::new()
    .title(sanitized_title)
    .description(sanitized_description)
    .color(0xFF69B4)
    .timestamp(embed_timestamp)
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
