
use crate::{
  types::rag::*
};

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;
use tracing::{debug, info, warn};
use regex::Regex;

static RAG_OPERATION_REGEX: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r#"(?i)(find_term|find_category|find_tag|search_terms|get_related)\s*\(\s*["']([^"']+)["']\s*\)"#)
    .expect("Failed to compile RAG operation regex")
});

impl Default for RagConfig {
  fn default() -> Self {
    Self {
      database_path: "knowledge.yml".to_string(),
      max_terms_per_query: 5,
      max_context_length: 2000,
      api_documentation: include_str!("api_docs.txt").to_string()
    }
  }
}

impl RagSystem {
  pub fn new(config: RagConfig) -> Result<Self> {
    let knowledge_base = Self::load_knowledge_base(&config.database_path)?;
    info!("RAG system initialized with {} terms", knowledge_base.terms.len());
    Ok(Self {
      config,
      knowledge_base
    })
  }

  fn load_knowledge_base(path: &str) -> Result<KnowledgeBase> {
    if !Path::new(path).exists() {
      warn!("Knowledge base file not found at {}, creating empty database", path);
      return Ok(KnowledgeBase {
        terms: HashMap::new(),
        categories: HashMap::new(),
        aliases: HashMap::new(),
      });
    }

    let content = fs::read_to_string(path)
      .with_context(|| format!("Failed to read knowledge base file: {}", path))?;
    
    let knowledge_base: KnowledgeBase = serde_yaml_bw::from_str(&content)
      .with_context(|| format!("Failed to parse YAML knowledge base: {}", path))?;
    
    info!("Loaded knowledge base with {} terms from {}", knowledge_base.terms.len(), path);
    
    Ok(knowledge_base)
  }

  pub fn get_api_documentation(&self) -> &str {
    &self.config.api_documentation
  }

  pub fn parse_operations(&self, text: &str) -> Vec<RagOperation> {
    let mut operations = Vec::new();
    
    for caps in RAG_OPERATION_REGEX.captures_iter(text) {
      let operation_type = caps.get(1).unwrap().as_str().to_lowercase();
      let parameter = caps.get(2).unwrap().as_str();
      
      let operation = match operation_type.as_str() {
        "find_term" => RagOperation::FindTerm(parameter.to_string()),
        "find_category" => RagOperation::FindCategory(parameter.to_string()),
        "find_tag" => RagOperation::FindTag(parameter.to_string()),
        "search_terms" => RagOperation::SearchTerms(parameter.to_string()),
        "get_related" => RagOperation::GetRelated(parameter.to_string()),
        _ => continue,
      };
      
      operations.push(operation);
    }
    
    debug!("Parsed {} RAG operations from text", operations.len());
    operations
  }

  pub fn execute_operation(&self, operation: &RagOperation) -> RagResult {
    let terms = match operation {
      RagOperation::FindTerm(term) => self.find_term(term),
      RagOperation::FindCategory(category) => self.find_category(category),
      RagOperation::FindTag(tag) => self.find_tag(tag),
      RagOperation::SearchTerms(query) => self.search_terms(query),
      RagOperation::GetRelated(term) => self.get_related(term),
    };
    
    let context_length = self.calculate_context_length(&terms);
    
    RagResult {
      operation: operation.clone(),
      terms,
      context_length,
    }
  }

  pub fn execute_operations(&self, operations: &[RagOperation]) -> Vec<RagResult> {
    let mut results = Vec::new();
    let mut total_context_length = 0;
    
    for operation in operations {
      if results.len() >= self.config.max_terms_per_query {
        debug!("Reached max terms per query limit");
        break;
      }
      
      let result = self.execute_operation(operation);
      
      if total_context_length + result.context_length > self.config.max_context_length {
        debug!("Would exceed max context length, stopping operation execution");
        break;
      }
      
      total_context_length += result.context_length;
      results.push(result);
    }
    
    info!("Executed {} operations, total context length: {}", results.len(), total_context_length);
    results
  }

  fn find_term(&self, term: &str) -> Vec<(String, TermInfo)> {
    let normalized_term = term.to_lowercase();

    if let Some(term_info) = self.knowledge_base.terms.get(&normalized_term) {
      return vec![(normalized_term, term_info.clone())];
    }

    if let Some(actual_term) = self.knowledge_base.aliases.get(&normalized_term) {
      if let Some(term_info) = self.knowledge_base.terms.get(actual_term) {
        return vec![(actual_term.clone(), term_info.clone())];
      }
    }

    let mut matches = Vec::new();
    for (key, info) in &self.knowledge_base.terms {
      if key.contains(&normalized_term) {
        matches.push((key.clone(), info.clone()));
      }
    }
    
    matches
  }

  fn find_category(&self, category: &str) -> Vec<(String, TermInfo)> {
    let normalized_category = category.to_lowercase();
    let mut results = Vec::new();
    
    if let Some(term_names) = self.knowledge_base.categories.get(&normalized_category) {
      for term_name in term_names {
        if let Some(term_info) = self.knowledge_base.terms.get(term_name) {
          results.push((term_name.clone(), term_info.clone()));
        }
      }
    }
    
    results
  }

  fn find_tag(&self, tag: &str) -> Vec<(String, TermInfo)> {
    let normalized_tag = tag.to_lowercase();
    let mut results = Vec::new();
    
    for (term_name, term_info) in &self.knowledge_base.terms {
      if let Some(tags) = &term_info.tags {
        if tags.iter().any(|t| t.to_lowercase().contains(&normalized_tag)) {
          results.push((term_name.clone(), term_info.clone()));
        }
      }
    }
    
    results
  }

  fn search_terms(&self, query: &str) -> Vec<(String, TermInfo)> {
    let normalized_query = query.to_lowercase();
    let mut results = Vec::new();
    
    for (term_name, term_info) in &self.knowledge_base.terms {
      if term_name.to_lowercase().contains(&normalized_query) {
        results.push((term_name.clone(), term_info.clone()));
        continue;
      }
      
      if term_info.definition.to_lowercase().contains(&normalized_query) {
        results.push((term_name.clone(), term_info.clone()));
        continue;
      }
      
      if let Some(description) = &term_info.description {
        if description.to_lowercase().contains(&normalized_query) {
          results.push((term_name.clone(), term_info.clone()));
        }
      }
    }
    
    results
  }

  fn get_related(&self, term: &str) -> Vec<(String, TermInfo)> {
    let normalized_term = term.to_lowercase();
    let mut results = Vec::new();
    
    if let Some(term_info) = self.knowledge_base.terms.get(&normalized_term) {
      if let Some(related_terms) = &term_info.related_terms {
        for related_term in related_terms {
          if let Some(related_info) = self.knowledge_base.terms.get(related_term) {
            results.push((related_term.clone(), related_info.clone()));
          }
        }
      }
    }
    
    results
  }

  fn calculate_context_length(&self, terms: &[(String, TermInfo)]) -> usize {
    terms.iter().map(|(name, info)| {
      let mut length = name.len() + info.definition.len();
      if let Some(description) = &info.description {
        length += description.len();
      }
      length
    }).sum()
  }

  pub fn format_retrieved_context(&self, results: &[RagResult]) -> String {
    if results.is_empty() {
      return String::new();
    }
    
    let mut context = String::from("RETRIEVED KNOWLEDGE:\n\n");
    
    for result in results {
      if result.terms.is_empty() {
        continue;
      }
      
      match &result.operation {
        RagOperation::FindTerm(term) => {
          context.push_str(&format!("Information about '{}':\n", term));
        }
        RagOperation::FindCategory(category) => {
          context.push_str(&format!("Terms in category '{}':\n", category));
        }
        RagOperation::FindTag(tag) => {
          context.push_str(&format!("Terms tagged with '{}':\n", tag));
        }
        RagOperation::SearchTerms(query) => {
          context.push_str(&format!("Search results for '{}':\n", query));
        }
        RagOperation::GetRelated(term) => {
          context.push_str(&format!("Terms related to '{}':\n", term));
        }
      }
      
      for (term_name, term_info) in &result.terms {
        context.push_str(&format!("- {}: {}", term_name, term_info.definition));
        
        if let Some(description) = &term_info.description {
          context.push_str(&format!(" {}", description));
        }
        
        if let Some(category) = &term_info.category {
          context.push_str(&format!(" [Category: {}]", category));
        }
        
        context.push('\n');
      }
      
      context.push('\n');
    }
    
    context
  }

  pub fn reload_knowledge_base(&mut self) -> Result<()> {
    self.knowledge_base = Self::load_knowledge_base(&self.config.database_path)?;
    info!("Knowledge base reloaded");
    Ok(())
  }

  pub fn get_statistics(&self) -> HashMap<String, usize> {
    let mut stats = HashMap::new();
    stats.insert("total_terms".to_string(), self.knowledge_base.terms.len());
    stats.insert("total_categories".to_string(), self.knowledge_base.categories.len());
    stats.insert("total_aliases".to_string(), self.knowledge_base.aliases.len());
    
    let terms_with_descriptions = self.knowledge_base.terms.values()
      .filter(|info| info.description.is_some())
      .count();
    stats.insert("terms_with_descriptions".to_string(), terms_with_descriptions);
    
    let terms_with_tags = self.knowledge_base.terms.values()
      .filter(|info| info.tags.is_some())
      .count();
    stats.insert("terms_with_tags".to_string(), terms_with_tags);
    
    stats
  }
}
