use crate::types::{
  common::IOptions,
  state::GlobalState
};

use once_cell::sync::Lazy;

const DHALL_FILE_NAME: &str = "conf.dhall";

#[allow(clippy::result_large_err)]
pub static CONFIG: Lazy<IOptions> = Lazy::new(|| {
  serde_dhall::from_file(DHALL_FILE_NAME).parse()
    .expect("Failed to read dhall config file")
});

pub static mut GLOBAL: Lazy<GlobalState> = Lazy::new(|| {
  GlobalState {
    last_news: String::new()
  }
});
