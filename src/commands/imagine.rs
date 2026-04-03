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

/// x/z-image-turbo:fp8 - FP8 quantized, fits ~8GB VRAM
/// x/flux2-klein:4b - crashes with text encoder bug in current Ollama versions
const DEFAULT_IMAGE_MODEL: &str = "x/z-image-turbo:fp8";
const DEFAULT_WIDTH: u32 = 512;
const DEFAULT_HEIGHT: u32 = 512;

#[derive(Serialize)]
struct ImageGenerationRequest {
  model: String,
  prompt: String,
  width: u32,
  height: u32,
  stream: bool,
}

#[derive(Deserialize, Debug)]
struct ImageGenerationResponse {
  image: Option<String>,
  response: Option<String>,
  done: bool,
  #[serde(default)]
  error: Option<String>,
  #[serde(default)]
  done_reason: Option<String>,
}

/// Parse image dimensions from command arguments
/// Format: ~imagine [width]x[height] <prompt>
fn parse_imagine_command(content: &str) -> (Option<(u32, u32)>, String) {
  let parts: Vec<&str> = content.splitn(2, ' ').collect();

  if parts.len() < 2 {
    return (None, content.to_string());
  }

  let first_part = parts[0];
  if let Some(dims) = parse_dimensions(first_part) {
    return (Some(dims), parts[1].to_string());
  }

  (None, content.to_string())
}

fn parse_dimensions(dim_str: &str) -> Option<(u32, u32)> {
  if dim_str.contains('x') {
    let parts: Vec<&str> = dim_str.split('x').collect();
    if parts.len() == 2 {
      if let (Ok(w), Ok(h)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
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

  let (dims, clean_prompt) = parse_imagine_command(&prompt);
  let (width, height) = dims.unwrap_or((DEFAULT_WIDTH, DEFAULT_HEIGHT));

  let model = DEFAULT_IMAGE_MODEL;

  info!("Generating image with model: {}, size: {}x{}, prompt: {}",
        model, width, height, clean_prompt);

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

  let attachment_name = "generated_image.png".to_string();
  let response_content = format!("🎨 Here's your imagination of: **{}**", clean_prompt);

  let message_builder = state.http
    .create_message(msg.channel_id)
    .reply(msg.id)
    .content(&response_content);

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

    if status == 404 || error_text.contains("not found") {
      anyhow::bail!(
        "Model '{}' not found. Download it with: ollama pull {}",
        DEFAULT_IMAGE_MODEL,
        DEFAULT_IMAGE_MODEL
      );
    }

    if status == 500 && error_text.contains("index out of range") {
      anyhow::bail!(
        "Ollama has a known bug in the text encoder for image generation models.\n\
         The model crashes with: 'runtime error: index out of range [0] with length 0'\n\
         This is an Ollama internal bug, not a configuration issue.\n\
         Possible workarounds:\n\
         1. Update Ollama to the latest version\n\
         2. Use ComfyUI with z-image-turbo-fp8 weights instead\n\
         3. Try running ollama with OLLAMA_NUM_GPU=0 to force CPU-only"
      );
    }

    if error_text.contains("GiB") || error_text.contains("memory") || error_text.contains("VRAM") {
      anyhow::bail!(
        "Not enough GPU memory. Ollama image models need 12+ GB VRAM.\n\
         For low VRAM (~8 GB), try ComfyUI with FP8 quantized weights instead.\n\
         Error: {}",
        error_text
      );
    }

    anyhow::bail!("Ollama API returned error status {}: {}", status, error_text);
  }

  let response_json: ImageGenerationResponse = response
    .json()
    .await
    .context("Failed to parse Ollama JSON response")?;

  info!("Ollama response: done={}, done_reason={:?}, has_image={}, has_response={}",
        response_json.done,
        response_json.done_reason,
        response_json.image.is_some(),
        response_json.response.is_some()
  );

  if let Some(error_msg) = response_json.error {
    anyhow::bail!("Ollama error: {}", error_msg);
  }

  if !response_json.done {
    anyhow::bail!("Image generation did not complete successfully");
  }

  let image_base64 = response_json.image
    .or(response_json.response.clone())
    .ok_or_else(|| anyhow::anyhow!("No image data in response (done={}, done_reason={:?})",
                                    response_json.done,
                                    response_json.done_reason))?;

  let trimmed = image_base64.trim();
  let has_spaces = trimmed.contains(' ');
  let has_common_words = trimmed.contains(" the ") || trimmed.contains(" a ") || trimmed.contains(" и ");
  if has_spaces || has_common_words {
    let preview: String = trimmed.chars().take(150).collect();
    anyhow::bail!(
      "Model '{}' returned text instead of image data.\n\
       Use an image generation model: x/z-image-turbo\n\
       Pull with: ollama pull x/z-image-turbo\n\
       Response preview: {}",
      DEFAULT_IMAGE_MODEL,
      preview
    );
  }

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
