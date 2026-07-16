//! LLM-based tool-result summarization and memory extraction.

#[allow(clippy::wildcard_imports)]
use crate::*;

/// Fit tool results within a token budget using intelligent proportional allocation.
///
/// Key improvements over naive equal division:
/// - Proportional: larger results get proportionally more budget
/// Configuration for tool result summarization
#[derive(Clone)]
pub struct ToolSummarizationConfig {
    /// Model priority list (e.g., ["ollama/llama3.2", "anthropic/claude-haiku"])
    pub(crate) model_priority: Vec<String>,
    /// Ollama URL for local models
    pub(crate) ollama_url: String,
    /// Threshold (chars) above which to summarize
    pub(crate) threshold: usize,
    /// Reference to main config for creating clients
    pub(crate) config: Config,
}

/// Summarize a large tool result using configured summarization models
/// Falls back to truncation if all models fail or none are configured
/// Uses hierarchical summarization for very large content (chunks recursively)
pub(crate) async fn summarize_tool_result(
    content: &str,
    tool_name: &str,
    summarization_config: &ToolSummarizationConfig,
    target_chars: usize,
) -> String {
    // If content is small enough, return as-is
    if content.len() <= summarization_config.threshold {
        return content.to_string();
    }

    // If no summarization models configured, truncate
    if summarization_config.model_priority.is_empty() {
        info!(
            "No summarization models configured, truncating {} ({} chars)",
            tool_name, content.len()
        );
        return smart_truncate_tool_result(content, tool_name, target_chars);
    }

    // Hierarchical summarization handles any size - the summarizer will:
    // - Chunk content into model-sized pieces (max 20 chunks per level)
    // - Summarize each chunk
    // - Recursively summarize if combined result is still large

    // Try each model in priority order
    for model_spec in &summarization_config.model_priority {
        debug!("Attempting to summarize {} with model {}", tool_name, model_spec);

        match try_summarize_with_model(
            content,
            tool_name,
            model_spec,
            &summarization_config.ollama_url,
            &summarization_config.config,
            target_chars,
        )
        .await
        {
            Ok(summary) => {
                info!(
                    "Summarized tool result '{}': {} -> {} chars using {}",
                    tool_name,
                    content.len(),
                    summary.len(),
                    model_spec
                );
                return format!(
                    "[Summarized from {} chars using {}]\n\n{}",
                    content.len(),
                    model_spec,
                    summary
                );
            }
            Err(e) => {
                warn!(
                    "Summarization with {} failed for {}: {}",
                    model_spec, tool_name, e
                );
            }
        }
    }

    // All models failed, fall back to truncation
    warn!(
        "All summarization models failed for {}, truncating",
        tool_name
    );
    smart_truncate_tool_result(content, tool_name, target_chars)
}

/// Try to summarize content with a specific model via direct LLM call
pub(crate) async fn try_summarize_with_model(
    content: &str,
    tool_name: &str,
    model_spec: &str,
    ollama_url: &str,
    config: &Config,
    _target_chars: usize,
) -> Result<String, String> {
    use nanna_llm::{AnthropicMessage, AnthropicRequest, ContentBlock};

    // Parse model spec (provider/model or just model)
    let (client, model_name) = create_summarization_client(model_spec, ollama_url, config)?;

    // Get model's context window from cache or API
    let cache = ModelInfoCache::default_location();
    let model_info = client.get_model_info(&model_name, cache.as_ref()).await;
    let context_window = model_info.context_window;

    // Truncate content to fit the model's context window (leave room for prompt + output)
    let usable_tokens = context_window.saturating_sub(3000);
    let max_chars = (usable_tokens * 4).max(4000); // ~4 chars per token, min 4k chars

    info!(
        "Using model {} (context: {}) for summarization, max_chars: {}",
        model_name, context_window, max_chars
    );

    // Cut on a char boundary — a raw byte slice panics mid-codepoint.
    let truncated = if content.len() > max_chars {
        &content[..content.floor_char_boundary(max_chars)]
    } else {
        content
    };

    let prompt = format!(
        "Summarize the following output from a tool called '{}'. Preserve all important information \
         including file paths, code snippets, error messages, and key data.\n\n\
         ---\n{}\n---\n\nProvide a concise summary:",
        tool_name, truncated
    );

    let request = AnthropicRequest {
        model: model_name.clone(),
        messages: vec![AnthropicMessage::user_text(prompt)],
        max_tokens: 2048,
        temperature: Some(0.3),
        system: Some("You are a summarizer. Output only the summary, no preamble.".to_string()),
        tools: None,
        stream: None,
        thinking: None,
        cache_control: None,
    };

    let response = client.complete_anthropic(&request).await.map_err(|e| e.to_string())?;

    let mut summary = String::new();
    for block in &response.content {
        if let ContentBlock::Text { text } = block {
            summary.push_str(text);
        }
    }

    // Reject degenerate output (empty, "...", a bare title): accepting it
    // silently replaces real data — observed live as 80 KB → 17 chars.
    if nanna_agent::plausible_summary(&summary, truncated.len()) {
        Ok(summary)
    } else {
        Err(format!(
            "Implausible summary returned ({} chars for {} chars of input)",
            summary.trim().len(),
            truncated.len()
        ))
    }
}

/// Create an LLM client for the specified summarization model
pub(crate) fn create_summarization_client(
    model_spec: &str,
    ollama_url: &str,
    config: &Config,
) -> Result<(LlmClient, String), String> {
    if let Some((provider, model)) = model_spec.split_once('/') {
        match provider.to_lowercase().as_str() {
            "ollama" => Ok((LlmClient::ollama(ollama_url), model.to_string())),
            "anthropic" => {
                // Use existing Anthropic credentials
                if let Some(ref key) = config.llm.api_key {
                    Ok((LlmClient::anthropic(key), model.to_string()))
                } else if config.llm.anthropic_use_oauth {
                    if let Some(ref token) = config.llm.anthropic_oauth_token {
                        Ok((LlmClient::anthropic_oauth(token), model.to_string()))
                    } else {
                        Err("Anthropic OAuth enabled but no token available".to_string())
                    }
                } else {
                    Err("No Anthropic API key configured".to_string())
                }
            }
            "openai" => {
                if let Some(ref key) = config.llm.openai_api_key {
                    Ok((LlmClient::openai(key), model.to_string()))
                } else {
                    Err("No OpenAI API key configured".to_string())
                }
            }
            "openrouter" => {
                if let Some(ref key) = config.llm.openrouter_api_key {
                    Ok((LlmClient::openrouter(key), model.to_string()))
                } else {
                    Err("No OpenRouter API key configured".to_string())
                }
            }
            _ => Err(format!("Unknown provider: {}", provider)),
        }
    } else {
        // No provider prefix - assume ollama
        Ok((LlmClient::ollama(ollama_url), model_spec.to_string()))
    }
}

/// Fit tool results to budget with optional summarization
/// This is the async version that can summarize large results
pub(crate) async fn fit_tool_results_to_budget_with_summarization(
    tool_results: Vec<(String, String, String, bool)>, // (id, name, content, is_error)
    budget_tokens: usize,
    summarization_config: Option<&ToolSummarizationConfig>,
) -> Vec<(String, String, bool)> {
    if tool_results.is_empty() {
        return vec![];
    }

    // Build entries with metadata
    let entries: Vec<ToolResultEntry> = tool_results
        .into_iter()
        .enumerate()
        .map(|(idx, (id, name, content, is_error))| {
            let raw_tokens = estimate_tokens(&content);
            ToolResultEntry {
                id,
                name,
                content,
                is_error,
                raw_tokens,
                recency_index: idx,
            }
        })
        .collect();

    // Calculate total raw tokens
    let total_raw_tokens: usize = entries.iter().map(|e| e.raw_tokens).sum();

    // If within budget, return as-is
    if total_raw_tokens <= budget_tokens {
        return entries
            .into_iter()
            .map(|e| (e.id, e.content, e.is_error))
            .collect();
    }

    // Allocate budgets intelligently
    let allocations = allocate_tool_budgets(&entries, budget_tokens);

    info!(
        "Tool results over budget ({} > {} tokens, {} results). Processing with summarization.",
        total_raw_tokens,
        budget_tokens,
        entries.len()
    );

    // Process each entry with summarization or truncation
    let mut results = Vec::with_capacity(entries.len());

    for (entry, budget_chars) in entries.into_iter().zip(allocations) {
        let processed = if entry.content.len() <= budget_chars {
            // Within budget, keep as-is
            entry.content
        } else if let Some(config) = summarization_config {
            // Try summarization for large results
            if entry.content.len() > config.threshold {
                summarize_tool_result(&entry.content, &entry.name, config, budget_chars).await
            } else {
                smart_truncate_tool_result(&entry.content, &entry.name, budget_chars)
            }
        } else {
            // No summarization config, just truncate
            smart_truncate_tool_result(&entry.content, &entry.name, budget_chars)
        };

        results.push((entry.id, processed, entry.is_error));
    }

    results
}

/// Configuration for memory extraction
pub struct ExtractionConfig {
    pub(crate) embedding_enabled: bool,
    pub(crate) extraction_model: String,
    pub(crate) chat_model: String,
}

/// Extract memories from a conversation turn and store them
///
/// Skips extraction if embeddings are disabled (recall won't work anyway).
/// Uses configurable extraction model (falls back to chat model if empty).
/// Includes importance scoring (1-5) for FSRS prioritization.
/// Memories are scoped to the provided workspace_id (None = global).
pub(crate) async fn extract_and_store_memories(
    llm: &LlmClient,
    memory: &MemoryService,
    memory_path: &std::path::Path,
    user_message: &str,
    assistant_response: &str,
    session_id: &str,
    config: ExtractionConfig,
    workspace_id: Option<String>,
) {
    // Skip extraction if embeddings are disabled - recall won't work anyway
    if !config.embedding_enabled {
        debug!("Skipping memory extraction: embeddings disabled");
        return;
    }

    // Determine which model to use for extraction
    let model = if config.extraction_model.is_empty() {
        &config.chat_model
    } else {
        &config.extraction_model
    };

    let extraction_prompt = format!(
        r#"Analyze this conversation turn and extract important facts worth remembering about the user.

User said: "{}"

Assistant replied: "{}"

Extract facts in two categories:

**STATED** - Things the user explicitly said about themselves:
- Their name, location, job
- Preferences they directly expressed
- Projects/goals they mentioned
- Family/relationships they described

**OBSERVED** - Your observations/inferences about the user (use sparingly):
- Patterns in their behavior or interests
- Implicit preferences based on their questions
- Expertise level you've noticed

Rules:
- STATED facts must be directly from the user's words
- OBSERVED facts are your synthesis - be conservative, only note strong patterns
- Rate importance 1-5 (5 = critical identity, 1 = minor detail)
- Skip generic conversation
- If nothing memorable, output NONE

Output format (one per line, or NONE):
STATED|importance: [fact the user explicitly said]
OBSERVED|importance: [your observation about the user]

Examples:
STATED|5: The user's name is Justin
STATED|4: User is working on rewriting Clawdbot in Rust
OBSERVED|3: User values performance and prefers Rust over higher-level languages"#,
        user_message.chars().take(500).collect::<String>(),
        assistant_response.chars().take(500).collect::<String>(),
    );

    let request = nanna_llm::CompletionRequest::default()
        .with_model(model)
        .with_message(nanna_llm::Message::user(&extraction_prompt));

    match llm.complete(&request).await {
        Ok(response) => {
            let mut stored_count = 0;

            // Parse extracted facts with importance and source type
            for line in response.lines() {
                let line = line.trim();

                // Determine fact type: STATED (user said) or OBSERVED (model inferred)
                let (fact_type, rest) = if line.starts_with("STATED|") {
                    ("stated", line.strip_prefix("STATED|"))
                } else if line.starts_with("OBSERVED|") {
                    ("observed", line.strip_prefix("OBSERVED|"))
                } else if line.starts_with("FACT|") {
                    // Legacy format - treat as stated for backwards compatibility
                    ("stated", line.strip_prefix("FACT|"))
                } else {
                    continue;
                };

                if let Some(rest) = rest {
                    // Parse "importance: content"
                    if let Some((importance_str, fact)) = rest.split_once(':') {
                        let importance: f32 = importance_str
                            .trim()
                            .parse()
                            .unwrap_or(3.0);
                        let fact = fact.trim();

                        if !fact.is_empty() && fact.len() > 5 {
                            // Store the memory with importance and fact type
                            let mut metadata = std::collections::HashMap::new();
                            metadata.insert("session_id".to_string(), session_id.to_string());
                            metadata.insert("source".to_string(), "extraction".to_string());
                            metadata.insert("importance".to_string(), importance.to_string());
                            metadata.insert("fact_type".to_string(), fact_type.to_string());

                            // Use scoped remember - memory is tied to current workspace (or global)
                            match memory.remember_scoped(fact, metadata, importance, workspace_id.clone()).await {
                                Ok((id, action)) => {
                                    info!("Memory {} [{}]: {} (id: {}, importance: {}, workspace: {:?})",
                                        match action {
                                            nanna_memory::IngestAction::Create => "stored",
                                            nanna_memory::IngestAction::Reinforce => "reinforced",
                                            nanna_memory::IngestAction::Update => "updated",
                                        },
                                        fact_type,
                                        fact.chars().take(40).collect::<String>(),
                                        id,
                                        importance,
                                        workspace_id);
                                    stored_count += 1;
                                }
                                Err(e) => {
                                    debug!("Failed to store memory: {}", e);
                                }
                            }
                        }
                    }
                }
            }

            // Auto-save memories after extraction if any were stored
            if stored_count > 0 {
                if let Err(e) = memory.save(memory_path).await {
                    debug!("Failed to auto-save memories: {}", e);
                } else {
                    debug!("Auto-saved {} memories", stored_count);
                }
            }
        }
        Err(e) => {
            debug!("Memory extraction failed: {}", e);
        }
    }
}
