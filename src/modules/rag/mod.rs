mod generation;
pub mod rag_system;

use crate::{
  types::rag::*
};

use once_cell::sync::Lazy;

pub static RAG_OLLAMA: Lazy<RagEnabledOllama> = Lazy::new(|| {
  let config = RagConfig {
    database_path: "knowledge.yml".to_string(),
    max_terms_per_query: 5,
    max_context_length: 2000,
    api_documentation: include_str!("api_docs.txt").to_string()
  };
  
  RagEnabledOllama::new(config)
      .expect("Failed to initialize RAG system")
});
