use crate::{
  modules::{ ollama, discord, state },
  options,
  types::state::{ ConversationHistory, State }
};

use twilight_model::channel::Message;

use anyhow::Context;
use smallvec::SmallVec;

pub async fn reply(
  msg: Message,
  text: String,
  author: String,
  state: State
) -> anyhow::Result<()> {
  tracing::debug!("reply command in channel {} by {}", msg.channel_id
                                                     , msg.author.name);

  let permit = match discord::try_acquire_permit(&state, &msg).await? {
    Some(p) => p,
    None    => return Ok(())
  };

  let mut history = {
    let mut history_lock = state.conversation_history.lock().await;
    let entry = history_lock.entry(msg.channel_id.to_string())
      .or_insert_with(|| ConversationHistory { messages: SmallVec::new() });

    let mut cloned = entry.clone();

    if cloned.messages.len() > state::HISTORY_LIMIT {
      cloned.messages.drain(0..cloned.messages.len() - state::HISTORY_LIMIT);
    }
    cloned
  };

  if msg.channel_id == options::CONFIG.twitter_channel_id {
    let last_news = options::GLOBAL.read()
      .expect("GLOBAL lock poisoned")
      .last_news.clone();
    history.messages.push((
      author.clone().into(),
      "Рарити, расскажи новости".into(),
      last_news.into()
    ));
  }

  let response = ollama::generate_ollama_with_history(&text, &author, &history, &state)
    .await
    .context("Failed to generate response")?;

  history.messages.push((author.into(), text.into(), response.clone().into()));
  {
    let mut history_lock = state.conversation_history.lock().await;
    history_lock.insert(msg.channel_id.to_string(), history);
  }

  discord::send_response(&state, &msg, &response).await?;
  drop(permit);

  Ok(())
}
