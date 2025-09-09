use crate::{
  types::state::State,
  modules::state
};

pub const HISTORY_LIMIT: usize = 7;
pub const GLOBAL_HISTORY_LIMIT: usize = 25;

pub async fn update_global_state(
    text: String,
    author: String,
    state: State,
) -> anyhow::Result<()> {
  let mut history_lock = state.global_conversation_history.lock().await;

  let needs_drain = history_lock.messages.len() > state::GLOBAL_HISTORY_LIMIT;
  
  if needs_drain {
    let drain_amount = history_lock.messages.len() - state::GLOBAL_HISTORY_LIMIT;
    history_lock.messages.drain(0..drain_amount);
  }
  
  history_lock.messages.push((author.into(), text.into()));
  
  Ok(())
}
