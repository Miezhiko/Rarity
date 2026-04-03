use crate::{
  modules::discord,
  types::state::{ State }
};

use twilight_model::channel::Message;
use twilight_model::http::attachment::Attachment;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use base64::{Engine, engine::general_purpose::STANDARD};
use tracing::{info, error};

use std::time::Duration;

const IMAGE_GENERATION_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Available image generation models (all under 50GB)
/// Z-Image-Turbo: ~6GB, fast generation, good quality
/// FLUX.2 Klein (4B): ~8GB, higher quality, slower
/// FLUX.2 Klein (9B): ~18GB, best quality, slowest
const DEFAULT_IMAGE_MODEL: &str = "x/z-image-turbo";
const DEFAULT_WIDTH: u32 = 1024;
const DEFAULT_HEIGHT: u32 = 768;

#[derive(Serialize)]
struct ImageGenerationRequest {
  model: String,
  prompt: String,
  width: u32,
  height: u32,
  stream: bool,
}

#[derive(Deserialize)]
struct ImageGenerationResponse {
  image: Option<String>,
  done: bool,
  #[serde(default)]
  error: Option<String>,
}

/// Parse image dimensions from command arguments
/// Format: ~imagine [width]x[height] <prompt>
/// Examples:
///   ~imagine a cat -> 1024x768, "a cat"
///   ~imagine 512x512 a cat -> 512x512, "a cat"
///   ~imagine 1024 a cat -> 1024x1024, "a cat"
fn parse_imagine_command(content: &str) -> (Option<(u32, u32)>, String) {
  let parts: Vec<&str> = content.splitn(2, ' ').collect();
  
  if parts.len() < 2 {
    return (None, content.to_string());
  }

  let first_part = parts[0];
  
  // Check if first part is a dimension specification
  if let Some(dims) = parse_dimensions(first_part) {
    return (Some(dims), parts[1].to_string());
  }

  // Not a dimension, treat entire input as prompt
  (None, content.to_string())
}

fn parse_dimensions(dim_str: &str) -> Option<(u32, u32)> {
  if dim_str.contains('x') {
    let parts: Vec<&str> = dim_str.split('x').collect();
    if parts.len() == 2 {
      if let (Ok(w), Ok(h)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
        // Validate reasonable dimensions
        if w > 0 && w <= 2048 && h > 0 && h <= 2048 {
          return Some((w, h));
        }
      }
    }
  } else if let Ok(square) = dim_str.parse::<u32>() {
    if square > 0 && square <= 2048 {
      return Some((square, square));
    }
  }
  
  None
}

pub async fn imagine(
  msg: Message,
  prompt: String,
  state: State
) -> Result<()> {
  info!(
    "imagine command in channel {} by {} with prompt: {}",
    msg.channel_id, msg.author.name, prompt
  );

  let permit = match discord::try_acquire_permit(&state, &msg).await? {
    Some(p) => p,
    None    => return Ok(())
  };

  // Parse dimensions from prompt if present
  let (dims, clean_prompt) = parse_imagine_command(&prompt);
  let (width, height) = dims.unwrap_or((DEFAULT_WIDTH, DEFAULT_HEIGHT));
  
  let model = DEFAULT_IMAGE_MODEL;
  
  info!("Generating image with model: {}, size: {}x{}, prompt: {}", 
        model, width, height, clean_prompt);

  // Send initial acknowledgment
  let thinking_embed = discord::build_embed(
    "🎨 Generating Image...",
    &format!("Model: **{}**\nSize: {}×{}\nPrompt: {}",
             model, width, height, clean_prompt)
  )
  .timestamp(msg.timestamp)
  .build();

  let _thinking_msg = state.http
    .create_message(msg.channel_id)
    .embeds(&[thinking_embed])
    .reply(msg.id)
    .await
    .context("Failed to send thinking message")?;

  // Generate the image
  let image_bytes = match generate_image(&clean_prompt, width, height, &state).await {
    Ok(bytes) => bytes,
    Err(e) => {
      error!("Failed to generate image: {}", e);
      let error_embed = discord::build_embed_with_color(
        "❌ Generation Failed",
        &format!("Failed to generate image: {}", e),
        0xFF0000
      )
      .timestamp(msg.timestamp)
      .build();

      state.http
        .create_message(msg.channel_id)
        .embeds(&[error_embed])
        .await
        .context("Failed to send error message")?;

      drop(permit);
      return Ok(());
    }
  };

  info!("Image generated successfully ({} bytes), uploading to Discord", image_bytes.len());

  // Upload the image as an attachment
  let attachment_name = "generated_image.png".to_string();
  let response_content = format!("🎨 Here's your imagination of: **{}**", clean_prompt);

  let message_builder = state.http
    .create_message(msg.channel_id)
    .reply(msg.id)
    .content(&response_content);

  // Create attachment from image bytes with unique ID (using timestamp)
  let attachment_id = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_millis() as u64;
  
  let mut attachment = Attachment::from_bytes(
    attachment_name,
    image_bytes,
    attachment_id
  );
  attachment.description(format!("AI generated image: {}", clean_prompt));

  message_builder
    .attachments(&[attachment])
    .await
    .context("Failed to send image message")?;

  drop(permit);
  Ok(())
}

async fn generate_image(prompt: &str, width: u32, height: u32, state: &State) -> Result<Vec<u8>> {
  info!("Generating image with model: {}", DEFAULT_IMAGE_MODEL);

  let request_body = ImageGenerationRequest {
    model: DEFAULT_IMAGE_MODEL.to_string(),
    prompt: prompt.to_string(),
    width,
    height,
    stream: false,
  };

  let response = tokio::time::timeout(
    IMAGE_GENERATION_TIMEOUT,
    state.request_client
      .post("http://localhost:11434/api/generate")
      .json(&request_body)
      .send()
  )
  .await
  .context("Image generation timed out after 10 minutes")??;

  let status = response.status();
  if !status.is_success() {
    let error_text = response
      .text()
      .await
      .unwrap_or_else(|_| "Unknown error".to_string());
    
    // Check if it's a model not found error
    if status == 404 || error_text.contains("not found") {
      anyhow::bail!(
        "Model '{}' not found. Please download it first using: ollama pull {}",
        DEFAULT_IMAGE_MODEL,
        DEFAULT_IMAGE_MODEL
      );
    }
    
    anyhow::bail!("Ollama API returned error status {}: {}", status, error_text);
  }

  let response_json: ImageGenerationResponse = response
    .json()
    .await
    .context("Failed to parse Ollama JSON response")?;

  if let Some(error_msg) = response_json.error {
    anyhow::bail!("Ollama error: {}", error_msg);
  }

  if !response_json.done {
    anyhow::bail!("Image generation did not complete successfully");
  }

  let image_base64 = response_json
    .image
    .ok_or_else(|| anyhow::anyhow!("No image data in response"))?;

  // Decode base64 image
  let image_bytes = STANDARD
    .decode(&image_base64)
    .context("Failed to decode base64 image data")?;

  info!(
    "Image generated successfully: {} bytes ({}x{})",
    image_bytes.len(),
    width,
    height
  );

  Ok(image_bytes)
}
