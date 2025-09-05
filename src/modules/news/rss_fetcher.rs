use crate::{
  types::state::State,
  types::rss::*
};

use std::time::{Duration, SystemTime, UNIX_EPOCH};
use feed_rs::parser;
use tracing::{info, warn, error};

pub struct RssFetcher;

impl RssFetcher {
  pub async fn fetch_all_feeds(state: &State) -> Result<Vec<FeedItem>, Box<dyn std::error::Error + Send + Sync>> {
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
            .map(|t| Self::sanitize_discord_text(&t.content))
            .unwrap_or_else(|| "No title".to_string());
          
          let link = entry.links.first()
            .map(|l| l.href.clone())
            .unwrap_or_else(|| "No link".to_string());
          
          let description = entry.content
            .and_then(|c| c.body)
            .or_else(|| entry.summary.map(|s| s.content))
            .map(|d| Self::sanitize_discord_text(&d))
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
            .map(|t| Self::sanitize_discord_text(&t.content))
            .unwrap_or_else(|| "No title".to_string());
          
          let link = entry.links.iter()
            .find(|l| !l.href.is_empty())
            .map(|l| l.href.clone())
            .unwrap_or_else(|| "No link".to_string());
          
          let description = entry.summary
            .map(|s| Self::sanitize_discord_text(&s.content))
            .or_else(|| entry.content.and_then(|c| c.body.map(|b| Self::sanitize_discord_text(&b))))
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
}
