use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagConfig {
  pub database_path:        String,
  pub max_terms_per_query:  usize,
  pub max_context_length:   usize,
  pub api_documentation:    String
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeBase {
  pub terms:      HashMap<String, TermInfo>,
  pub categories: HashMap<String, Vec<String>>,
  pub aliases:    HashMap<String, String>
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermInfo {
  pub definition: String,
  pub description: Option<String>,
  pub category: Option<String>,
  pub related_terms: Option<Vec<String>>,
  pub tags: Option<Vec<String>>,
  pub updated: Option<String>
}

#[derive(Debug, Clone, PartialEq)]
pub enum RagOperation {
  FindTerm(String),
  FindCategory(String),
  FindTag(String),
  SearchTerms(String),
  GetRelated(String)
}

#[derive(Debug, Clone)]
pub struct RagResult {
  pub operation: RagOperation,
  pub terms: Vec<(String, TermInfo)>,
  pub context_length: usize
}

pub struct RagSystem {
  pub config: RagConfig,
  pub knowledge_base: KnowledgeBase
}

pub struct RagEnabledOllama {
  pub rag_system: RagSystem
}
