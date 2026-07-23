use crate::types::{
  common::IOptions,
  state::GlobalState
};

use std::sync::{ LazyLock, RwLock };

const DHALL_FILE_NAME: &str = "conf.dhall";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[allow(clippy::result_large_err)]
pub static CONFIG: LazyLock<IOptions> = LazyLock::new(|| {
  serde_dhall::from_file(DHALL_FILE_NAME).parse()
    .expect("Failed to read dhall config file")
});

pub static GLOBAL: LazyLock<RwLock<GlobalState>> = LazyLock::new(|| {
  RwLock::new(GlobalState {
    last_news: String::new()
  })
});
