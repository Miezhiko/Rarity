
use crate::{
  types::state::State,
  types::rag::*,
  options,
  modules::ollama
};

use anyhow::{ Context, Result };
use tracing::{ debug, info, warn };

impl RagEnabledOllama {
  pub fn new(rag_config: RagConfig) -> Result<Self> {
    let rag_system = RagSystem::new(rag_config)
        .context("Failed to initialize RAG system")?;
    
    Ok(Self { rag_system })
  }

  pub async fn generate_with_rag(
      &self,
      prompt: &str,
      state: &State,
  ) -> Result<String> {
    let api_docs = self.rag_system.get_api_documentation();
    let initial_prompt = format!(
      "{}\n\nAPI DOCUMENTATION:\n{}\n\nUSER REQUEST:\n{}",
      options::CONFIG.system_prompt,
      api_docs,
      prompt
    );

    let llm_response = ollama::generate_ollama_response(&initial_prompt, state).await?;
    
    let operations = self.rag_system.parse_operations(&llm_response);
    
    if operations.is_empty() {
      debug!("No RAG operations found in LLM response");
      return Ok(llm_response);
    }

    info!("Found {} RAG operations in LLM response", operations.len());

    let rag_results = self.rag_system.execute_operations(&operations);
    let retrieved_context = self.rag_system.format_retrieved_context(&rag_results);

    if retrieved_context.trim().is_empty() {
      warn!("RAG operations executed but no context retrieved");
      return Ok(llm_response);
    }

    let enhanced_prompt = format!(
      "{}\n\n{}\n\nBased on the above information, please provide a comprehensive answer to: {}",
      options::CONFIG.system_prompt,
      retrieved_context,
      prompt
    );

    info!("Generating enhanced response with {} characters of retrieved context", 
          retrieved_context.len());

    ollama::generate_ollama_response(&enhanced_prompt, state).await
  }

  /// Returns the generated text along with the model that produced it, so
  /// callers can retry with a different model (via `exclude_models`) if the
  /// response turns out to be unusable (e.g. empty after sanitization).
  pub async fn generate_with_secondary_and_rag(
      &self,
      prompt: &str,
      secondary_prompt: &str,
      state: &State,
      exclude_models: &[String],
  ) -> Result<(String, String)> {
    let api_docs = self.rag_system.get_api_documentation();
    let initial_prompt = format!(
      "API DOCUMENTATION:\n{}\n\nUSER REQUEST:\n{}",
      api_docs,
      prompt
    );

    // `secondary_prompt` (e.g. desc_mod_msg) must be passed through as-is
    // here, not folded into `initial_prompt`: only the `prompt` argument to
    // generate_ollama_response_with_secondary is subject to truncation when
    // the assembled prompt is too long, and system_prompt is already added
    // automatically -- passing it again here as secondary_prompt duplicated
    // it and pushed the real secondary_prompt (the task instructions) into
    // the truncatable region instead.
    let (llm_response, model_used) = ollama::generate_ollama_response_with_secondary(
      &initial_prompt,
      secondary_prompt,
      state,
      exclude_models
    ).await?;

    let operations = self.rag_system.parse_operations(&llm_response);

    if operations.is_empty() {
      debug!("No RAG operations found in LLM response");
      return Ok((llm_response, model_used));
    }

    info!("Found {} RAG operations in LLM response", operations.len());

    let rag_results = self.rag_system.execute_operations(&operations);
    let retrieved_context = self.rag_system.format_retrieved_context(&rag_results);

    if retrieved_context.trim().is_empty() {
      warn!("RAG operations executed but no context retrieved");
      return Ok((llm_response, model_used));
    }

    let enhanced_prompt = format!(
      "{}\n\nBased on the above information, please provide a comprehensive answer to: {}",
      retrieved_context,
      prompt
    );

    info!("Generating enhanced response with retrieved context");

    ollama::generate_ollama_response_with_secondary(
      &enhanced_prompt,
      secondary_prompt,
      state,
      exclude_models
    ).await
  }

  // TODO
  #[allow(dead_code)]
  pub fn reload_knowledge_base(&mut self) -> Result<()> {
    self.rag_system.reload_knowledge_base()
  }

  // TODO
  #[allow(dead_code)]
  pub fn get_rag_statistics(&self) -> std::collections::HashMap<String, usize> {
    self.rag_system.get_statistics()
  }

  pub fn should_use_rag(&self, prompt: &str) -> bool {
    let prompt_lower = prompt.to_lowercase();
    
    let info_keywords = [
      "что такое", "что такое", "кто такой", "кто такая", "расскажи о",
      "расскажи про", "объясни", "определение", "значение", "опиши",
      "как работает", "как сделать", "почему", "зачем", "информация о",
      "подробности о", "знать о", "знаком с", "как ты относишься"
    ];

    let question_words = ["что", "кто", "где", "когда", "почему", "как"];

    let has_info_keywords = info_keywords.iter().any(|keyword| prompt_lower.contains(keyword));
    let has_questions = question_words.iter().any(|word| prompt_lower.starts_with(word));
    let has_question_mark = prompt.contains('?');
    
    has_info_keywords || (has_questions && has_question_mark)
  }

  pub async fn generate_smart(
    &self,
    prompt: &str,
    state: &State
  ) -> Result<String> {
    if self.should_use_rag(prompt) {
      info!("Using RAG for information-seeking prompt");
      self.generate_with_rag(prompt, state).await
    } else {
      debug!("Using standard generation for conversational prompt");
      ollama::generate_ollama_response(prompt, state).await
    }
  }

  pub fn reload(&mut self) -> Result<()> {
    self.rag_system.reload_knowledge_base()
  }

  pub fn get_stats(&self) -> String {
    let stats = self.rag_system.get_statistics();
    let labels = [
      ("total_terms", "Total Terms"),
      ("total_categories", "Total Categories"),
      ("total_aliases", "Total Aliases"),
      ("terms_with_descriptions", "Terms with Descriptions"),
      ("terms_with_tags", "Terms with Tags")
    ];

    let mut output = String::from("📊 RAG Statistics:\n");    
    for (key, label) in &labels {
      if let Some(value) = stats.get(*key) {
        output.push_str(&format!("• {}: {}\n", label, value));
      }
    }

    output
  }
}
