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
pub const TRUNCATION_BUFFER: usize                = 50;
pub const CONTINUATION_SUFFIX: &str               = "...";

// twilight-validate's own pre-send embed check sums each field's UTF-8
// *byte* length (not codepoints/UTF-16 units) against this same 6000 limit,
// so multi-byte text (Cyrillic, emoji, ...) can trip it well before the
// UTF-16-based limits above do. Chunk descriptions to this byte budget and,
// as a last-resort safety net in `send_embed_message`, also enforce it there
// so a message is never rejected client-side after all this sanitizing.
pub const MAX_CHUNK_BYTES: usize                  = 3600;

// CRITICAL: Count UTF-16 code points, not characters
fn count_utf16(text: &str) -> usize {
  text.encode_utf16().count()
}

/// Shortens a model name for display in an embed footer, e.g.
/// "glm-4.7-flash:latest" -> "glm-4.7-flash". Tags other than "latest"
/// (like "gemma4:e2b") are kept since they're part of what identifies the
/// model.
fn compact_model_name(model: &str) -> &str {
  model.strip_suffix(":latest").unwrap_or(model)
}

fn truncate_to_byte_budget(text: &str, max_bytes: usize) -> String {
  if max_bytes == 0 {
    return String::new();
  }

  let mut result = String::new();
  let mut byte_len = 0;

  for ch in text.chars() {
    let ch_len = ch.len_utf8();
    if byte_len + ch_len > max_bytes {
      break;
    }
    result.push(ch);
    byte_len += ch_len;
  }

  result
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
  timestamp: Option<i64>,
  model_used: Option<&str>
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
  
  let footer_text = match model_used {
    Some(model) => format!("{} 💎 v{} · 🤖 {}", options::CONFIG.footer_text, options::VERSION, compact_model_name(model)),
    None        => format!("{} 💎 v{}", options::CONFIG.footer_text, options::VERSION),
  };

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

  // twilight-validate's pre-send check adds up UTF-8 *byte* length (not
  // UTF-16 units) across title + description + footer against
  // DISCORD_EMBED_TOTAL_LIMIT, so heavily non-ASCII text (Cyrillic, emoji)
  // can still be rejected here even though it passed the checks above.
  let byte_budget = DISCORD_EMBED_TOTAL_LIMIT
    .saturating_sub(sanitized_title.len())
    .saturating_sub(footer_text.len());

  if sanitized_description.len() > byte_budget {
    warn!("Description is {} UTF-8 bytes, over the embed's combined byte budget of {}; truncating further",
          sanitized_description.len(), byte_budget);
    sanitized_description = truncate_to_byte_budget(&sanitized_description, byte_budget);
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

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn compact_model_name_strips_latest_tag() {
    assert_eq!(compact_model_name("glm-4.7-flash:latest"), "glm-4.7-flash");
    assert_eq!(compact_model_name("hermes3:latest"), "hermes3");
  }

  #[test]
  fn compact_model_name_keeps_other_tags() {
    assert_eq!(compact_model_name("gemma4:e2b"), "gemma4:e2b");
  }

  #[test]
  fn compact_model_name_leaves_untagged_names_alone() {
    assert_eq!(compact_model_name("mistral-small3.2"), "mistral-small3.2");
  }

  #[test]
  fn truncate_to_byte_budget_respects_multibyte_boundaries() {
    let text = "Привет мир"; // Cyrillic: 2 bytes per char in UTF-8
    let truncated = truncate_to_byte_budget(text, 7);

    assert!(truncated.len() <= 7);
    assert!(text.starts_with(&truncated));
  }

  #[test]
  fn truncate_to_byte_budget_zero_is_empty() {
    assert_eq!(truncate_to_byte_budget("hello", 0), String::new());
  }

  #[test]
  fn truncate_to_byte_budget_never_exceeds_budget() {
    let text = "п".repeat(50); // each 'п' is 2 bytes in UTF-8
    for budget in 0..=text.len() {
      assert!(truncate_to_byte_budget(&text, budget).len() <= budget);
    }
  }

  #[test]
  fn truncate_to_byte_budget_keeps_whole_string_when_it_fits() {
    let text = "Привет";
    assert_eq!(truncate_to_byte_budget(text, text.len()), text);
  }
}