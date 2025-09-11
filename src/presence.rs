use crate::types::state::State;

use std::error::Error;

use twilight_model::{
  gateway::presence::{ActivityType, MinimalActivity, Status},
  gateway::payload::outgoing::UpdatePresence
};

async fn get_ollama_stats(state: &State) -> Result<(f32, f32), Box<dyn Error + Send + Sync>> {
  let response = state.request_client
    .get("http://localhost:11434/api/ps")
    .send()
    .await?;

  if response.status().is_success() {
    let json: serde_json::Value = response.json().await?;
    
    if let Some(models) = json["models"].as_array() {
      if !models.is_empty() {
        let mut total_cpu = 0.0f32;
        let mut total_mem_gb = 0.0;
        
        for model in models {
          if let Some(size_vram) = model["size_vram"].as_u64() {
            total_mem_gb += size_vram as f32 / 1_073_741_824.0; // Convert bytes to GB
          }
          // Ollama API doesn't directly provide CPU usage, so we estimate based on active models
          if model["expires_at"].is_string() {
            total_cpu += 15.0; // Rough estimate per active model
          }
        }
        
        return Ok((total_cpu.min(100.0), total_mem_gb));
      }
    }
  }

  #[cfg(target_os = "linux")]
  {
    use std::process::Command;
    
    let output = Command::new("pgrep")
      .arg("-f")
      .arg("ollama")
      .output();
      
    if let Ok(output) = output {
      if !output.stdout.is_empty() {
        let pid_str = String::from_utf8_lossy(&output.stdout);
        let pid = pid_str.trim();
        let stats_output = Command::new("ps")
          .args(&["-p", pid, "-o", "pcpu,pmem", "--no-headers"])
          .output();
          
        if let Ok(stats) = stats_output {
          let stats_str = String::from_utf8_lossy(&stats.stdout);
          let parts: Vec<&str> = stats_str.trim().split_whitespace().collect();
          
          if parts.len() >= 2 {
            let cpu = parts[0].parse::<f32>().unwrap_or(0.0);
            let mem_percent = parts[1].parse::<f32>().unwrap_or(0.0);
            
            // Estimate memory in GB (assuming 16GB total system RAM as baseline)
            let mem_gb = (mem_percent / 100.0) * 16.0;
            
            return Ok((cpu, mem_gb));
          }
        }
      }
    }
  }
  
  Ok((0.0, 0.0))
}

pub async fn update_bot_status(state: &State) -> Result<(), Box<dyn Error + Send + Sync>> {
  let (cpu, mem_gb) = get_ollama_stats(state).await.unwrap_or((0.0, 0.0));
  
  let status_text = if cpu > 0.0 || mem_gb > 0.0 {
    format!("🧠: {:.1}% | 🪄: {:.1}GB", cpu, mem_gb)
  } else {
    "💤 Рарити спит".to_string()
  };
  
  let activity = MinimalActivity {
    kind: ActivityType::Custom,
    name: status_text,
    url: None
  };
  
  let presence = match UpdatePresence::new(
    vec![activity.into()],
    false,
    None,
    Status::Online,
  ) {
    Ok(p) => p,
    Err(e) => {
      tracing::warn!("Failed to create presence update: {}", e);
      return Ok(());
    }
  };

  state.shard_sender.command(&presence)
    .map_err(|e| {
      tracing::warn!("Failed to send presence update: {}", e);
      Box::<dyn Error + Send + Sync>::from(e.to_string())
    })?;
    
  tracing::debug!("Updated bot status: CPU {:.1}%, RAM {:.1}GB", cpu, mem_gb);
  Ok(())
}
