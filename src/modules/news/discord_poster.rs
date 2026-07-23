use crate::{
  types::state::State,
  types::rss::*,
  options,
  modules::{ ollama, discord::* },
  modules::rag
};

use twilight_model::id::{Id, marker::{ChannelMarker}};
use tracing::{info, warn, error};

const CONTINUATION_PREFIX: &str = " (часть ";

pub struct DiscordPoster;

impl DiscordPoster {
  pub async fn post_to_discord(
    state: &State,
    channel_id: Id<ChannelMarker>, 
    items: &[FeedItem]
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let read_ollama = rag::RAG_OLLAMA.read().await;

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
      .map(|item| sanitize_discord_text(&item.title))
      .collect();
    let combined_titles = all_titles.join(", ");

    info!("got news: {combined_titles}");

    let all_descriptions: Vec<String> = valid_items
      .iter()
      .map(|item| sanitize_discord_text(&item.description))
      .collect();
    let combined_descriptions = all_descriptions.join(". ");

    let generation_permit = match state.generation_lock.try_acquire() {
      Ok(permit) => permit,
      Err(_) => {
        warn!("Generation lock not available, skipping message generation");
        return Ok(());
      }
    };

    let gen_prompt = || async {
      let rarity_response_title =
        ollama::generate_ollama_response_with_secondary( &combined_titles
                                                       , &options::CONFIG.title_mod_msg
                                                       , state ).await?;

      let rarity_response_desc =
        read_ollama.generate_with_secondary_and_rag( &combined_descriptions
                                                   , &options::CONFIG.desc_mod_msg
                                                   , state ).await?;

      let mut title_no_q = sanitize_discord_text(&remove_quotes(&rarity_response_title));
      let sanitized_description = sanitize_discord_text(&rarity_response_desc);

      // Safely truncate title if needed
      if title_no_q.chars().count() > DISCORD_EMBED_TITLE_LIMIT {
        title_no_q = safe_truncate(&title_no_q, DISCORD_EMBED_TITLE_LIMIT);
        warn!("Title truncated to fit Discord limits: {} chars", title_no_q.chars().count());
      }

      let latest_timestamp = valid_items
        .iter()
        .map(|item| item.published_timestamp)
        .max()
        .unwrap_or(0) as i64;

      let timestamp = if latest_timestamp > 0 { Some(latest_timestamp) } else { None };

      if sanitized_description.chars().count() > DISCORD_EMBED_DESCRIPTION_LIMIT {
        warn!("Description too long ({}), splitting into multiple messages", sanitized_description.chars().count());
        Self::send_chunked_message( state
                                  , channel_id
                                  , &title_no_q
                                  , &sanitized_description
                                  , timestamp ).await?;
      } else {
        info!("Sending single message with title length: {}, description length: {}", title_no_q.chars().count(), sanitized_description.chars().count());
        match send_embed_message( state
                                , channel_id
                                , &title_no_q
                                , &sanitized_description
                                , timestamp ).await {
          Ok(_) => info!("Single message sent successfully"),
          Err(e) => {
            error!("Failed to send single message: {}. Embed content: title='{}', description='{}'",
                   e, title_no_q, sanitized_description);
            return Err(e);
          }
        }
      }
      
      options::GLOBAL.write()
        .expect("GLOBAL lock poisoned")
        .last_news = combined_descriptions;

      info!("Posted combined news to Discord with {} items", valid_items.len());
      
      Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    };

    let result = gen_prompt().await;

    drop(generation_permit);
    result
  }

  async fn send_chunked_message(
    state: &State,
    channel_id: Id<ChannelMarker>,
    title: &str,
    description: &str,
    timestamp: Option<i64>
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
    
    match send_embed_message(state, channel_id, title, &first_description, timestamp).await {
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
        safe_truncate(&continuation_title, DISCORD_EMBED_TITLE_LIMIT)
      } else {
        continuation_title
      };

      info!("Sending continuation chunk {} with title length: {}, description length: {}", i + 2, safe_continuation_title.chars().count(), chunk.chars().count());
      
      match send_embed_message( state
                              , channel_id
                              , &safe_continuation_title
                              , chunk
                              , timestamp ).await {
        Ok(_) => info!("Continuation chunk {} sent successfully", i + 2),
        Err(e) => {
          error!("Failed to send continuation chunk {}: {}", i + 2, e);
          return Err(e);
        }
      }
    }

    Ok(())
  }
}
