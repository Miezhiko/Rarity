
<img width="494" height="505" alt="www" src="https://github.com/user-attachments/assets/3b0a87e6-d804-4694-b430-66690f77ac79" />

Rarity is a female unicorn pony and one of the main characters of My Little Pony Friendship is Magic. She is Sweetie Belle's older sister and the subject of Spike's long-term crush. Rarity lives and works at her own shop in Ponyville called the Carousel Boutique, where she takes care of her pet Persian cat Opalescence. She represents the element of generosity.

# Retrieval-Augmented Generation

1. **Question Analysis**: The system analyzes incoming prompts for information-seeking patterns
2. **LLM Query Generation**: If RAG is needed, the LLM generates appropriate database queries using the provided API
3. **Knowledge Retrieval**: The system executes queries against the YAML knowledge base
4. **Context Enhancement**: Retrieved information is added to the prompt context
5. **Final Response**: The LLM generates a response with access to the retrieved knowledge
6. **Context**: Keeps context of per-channel conversation, global chat and last posted news

# RSS Fetcher

 - Safely fetching news from several RSS instances
 - Mixes them together finding mysterious reasoning
 - Highly customized based on config file
 - In addition to just posting news to discord can talk about them
 - Can just talk in other channels without news context

# The LLM can use these operations to query the knowledge base:

 - `find_term("term_name")` - Look up specific terms
 - `find_category("category")` - Get all terms in a category
 - `find_tag("tag")` - Find terms with specific tags
 - `search_terms("query")` - Search term names and definitions
 - `get_related("term")` - Find related terms

 # Current Knowledge Base Structure

 ```yml
 terms:
  quadrober:
    definition: "A person who adopts quadrupedal movement and behavior"
    description: "Extended description here..."
    category: "internet_culture"
    related_terms: ["furry", "therian"]
    tags: ["subculture", "identity"]
    updated: "2024-01-15"

categories:
  internet_culture: ["quadrober", "furry"]

aliases:
  quad: "quadrober"
 ```

 # Configuration

 ```rust
 let config = RagConfig {
    database_path: "knowledge_base.yaml".to_string(),
    max_terms_per_query: 5,
    max_context_length: 2000,
    api_documentation: "...".to_string(),
};
 ```

 but main bot config is `conf.dhall` file

# The system automatically determines when to use RAG based on:

 - Question words (what, who, where, etc.)
 - Information-seeking phrases ("tell me about", "explain", etc.)
 - Question marks
 - Context analysis

# Performance Considerations

 - Knowledge base is loaded once at startup
 - Token limits are respected to prevent context overflow
 - Smart truncation preserves the most relevant information
 - Configurable limits prevent excessive retrieval

# Error Handling

 - Missing knowledge base files (creates empty database)
 - Invalid YAML format (logs errors, continues with existing data)
 - Network timeouts (falls back to non-RAG responses)
 - Token limit overruns (smart truncation)

*respect the QUADROBER license*
