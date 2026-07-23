use crate::{
  types::state::State,
  types::rss::*,
  options,
  modules::discord::sanitize_discord_text
};

use std::time::{Duration, SystemTime, UNIX_EPOCH};
use feed_rs::parser;
use tracing::{info, warn, error};

pub struct RssFetcher;

impl RssFetcher {
  pub async fn fetch_all_feeds(state: &State) -> Result<Vec<FeedItem>, Box<dyn std::error::Error + Send + Sync>> {
    let mut all_items = Vec::new();
    let mut successful_fetches = 0;

    for instance in &options::CONFIG.news_instances {
      info!("Attempting to fetch from {} ({})", instance.url, instance.instance_type);
      match instance.instance_type.as_str() {
        "bing" => {
          match Self::try_fetch_from_bing(state, instance.url.as_str()).await {
            Ok(items) => {
              info!("Successfully fetched {} items from Bing: {}", items.len(), instance.url);
              all_items.extend(items);
              successful_fetches += 1;
            }
            Err(e) => {
              warn!("Failed to fetch from Bing {}: {}", instance.url, e);
            }
          }
        }
        _ => {
          match Self::try_fetch_standard(state, instance.url.as_str()).await {
            Ok(items) => {
              info!("Successfully fetched {} items from: {}", items.len(), instance.url);
              all_items.extend(items);
              successful_fetches += 1;
            }
            Err(e) => {
              warn!("Failed to fetch from {}: {}", instance.url, e);
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
      .header("Accept-Encoding", "identity")
      .timeout(Duration::from_secs(3))
      .send()
      .await?;
    
    if !response.status().is_success() {
      return Err(format!("HTTP {}", response.status()).into());
    }

    // cound cause decoding problem, Rarity 0.6.6 fixes this
    // let content = response.text().await?;
    // get content as plain bytes instead and manually decode
    let bytes = response.bytes().await?;

    // Extract encoding from XML declaration if present
    let declared_encoding = if bytes.starts_with(b"<?xml") {
      let header = String::from_utf8_lossy(&bytes[..bytes.len().min(200)]);
      header
        .split("encoding=")
        .nth(1)
        .and_then(|s| s.trim_start_matches(['"', '\''])
                       .split(['"', '\'']).next())
        .map(|s| s.to_ascii_lowercase())
    } else {
      None
    };

    let content = match declared_encoding.as_deref() {
      Some("windows-1251") | Some("cp1251") => {
        let (decoded, _, had_errors) = encoding_rs::WINDOWS_1251.decode(&bytes);
        if had_errors {
          warn!("Encoding errors while decoding feed as Windows-1251");
        }
        decoded.into_owned()
      }
      Some("iso-8859-1") | Some("latin-1") => {
        let (decoded, _, had_errors) = encoding_rs::WINDOWS_1251.decode(&bytes);
        if had_errors {
          warn!("Encoding errors while decoding feed as ISO-8859-1");
        }
        decoded.into_owned()
      }
      _ => {
        // Fall back to UTF-8, then Windows-1251 if that fails
        match String::from_utf8(bytes.to_vec()) {
          Ok(s) => s,
          Err(_) => {
            warn!("UTF-8 decoding failed, falling back to Windows-1251");
            let (decoded, _, _) = encoding_rs::WINDOWS_1251.decode(&bytes);
            decoded.into_owned()
          }
        }
      }
    };

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
}
