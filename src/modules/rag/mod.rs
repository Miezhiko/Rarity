mod generation;
pub mod rag_system;

use crate::{
  types::rag::*
};

use std::sync::LazyLock;
use tokio::sync::RwLock;

pub static RAG_OLLAMA: LazyLock<RwLock<RagEnabledOllama>> = LazyLock::new(|| {
  let config = RagConfig {
    database_path: "knowledge.yml".to_string(),
    max_terms_per_query: 5,
    max_context_length: 2000,
    api_documentation: include_str!("api_docs.txt").to_string()
  };
  
  RwLock::new(
    RagEnabledOllama::new(config)
        .expect("Failed to initialize RAG system")
  )
});
