use serde::Deserialize;

#[derive(Clone, Deserialize)]
pub struct NewsInstance {
  #[serde(rename = "type")]
  pub instance_type: String,
  pub url: String
}

#[derive(Clone, Deserialize)]
pub struct IOptions {
  pub discord: String,
  pub models: Vec<String>,
  pub bot_name: String,
  pub system_prompt: String,
  pub footer_text: String,
  pub allowed_guilds: Vec<u64>,
  pub restricted_channels: Vec<u64>,
  pub owner: u64,
  pub bot: u64,
  pub twitter_channel_id: u64,
  pub title_mod_msg: String,
  pub desc_mod_msg: String,
  pub test_subscriber: bool,
  pub news_instances: Vec<NewsInstance>
}
