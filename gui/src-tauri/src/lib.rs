#![warn(clippy::pedantic, clippy::nursery, clippy::all)]

//! Nanna GUI - Tauri backend
//!
//! IPC bridge between the frontend and nanna-core with agentic tool loop.
//! Includes FSRS-6 cognitive memory and dreaming/consolidation.
//!
//! Supports two modes:
//! - **Daemon mode**: Connects to nanna-daemon via WebSocket
//! - **Embedded mode**: Runs agent directly (fallback when daemon unavailable)

pub mod agents;
pub mod backend;
pub mod daemon_client;
pub mod daemon_manager;
pub mod embedded;
pub mod tool_authoring;

use backend::{Backend, BackendMode};

use nanna_config::Config;
use nanna_core::{
    Scheduler, SchedulerConfig, consolidation_task,
    MemoryService, MemoryServiceConfig, ConsolidationConfig,
    // Workspaces
    Workspace, WorkspaceRegistry,
    find_workspace_root, discover_workspaces,
};
use nanna_llm::{AnthropicMessage, CompletionRequest, LlmClient, Message as LlmMessage, ModelInfoCache, RequestBuilder, Role, StreamEvent};
use nanna_storage::{Storage, StorageConfig};
use nanna_tools::{ToolCall, ToolRegistry};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    menu::{MenuBuilder, MenuItemBuilder},
    AppHandle, Emitter, Manager, State,
};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

// =============================================================================
// Channel Routing for Scheduled Tasks
// =============================================================================

use nanna_channels::{Channel, ChannelId, MessageContent, OutgoingMessage};

/// Route a message to a channel by ID (format: "provider:chat_id" e.g. "telegram:12345")
async fn route_to_channel(
    channels: &nanna_config::ChannelsConfig,
    channel_id: &str,
    message: &str,
) -> Result<(), String> {
    // Parse channel_id format: "provider:chat_id"
    let parts: Vec<&str> = channel_id.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(format!("Invalid channel_id format '{}', expected 'provider:chat_id'", channel_id));
    }

    let (provider, chat_id) = (parts[0], parts[1]);

    match provider.to_lowercase().as_str() {
        "telegram" => {
            let config = channels.telegram.as_ref()
                .ok_or("Telegram not configured")?;

            let channel = nanna_channels::TelegramChannel::new(&config.bot_token);
            let outgoing = OutgoingMessage {
                channel: ChannelId::new("telegram", chat_id),
                content: MessageContent::text(message),
                reply_to: None,
            };

            channel.send(outgoing).await
                .map_err(|e| format!("Telegram send failed: {}", e))?;

            info!("Routed message to Telegram chat {}", chat_id);
            Ok(())
        }
        "discord" => {
            let config = channels.discord.as_ref()
                .ok_or("Discord not configured")?;

            let channel = nanna_channels::DiscordChannel::new(
                &config.bot_token,
                &config.application_id,
            );
            let outgoing = OutgoingMessage {
                channel: ChannelId::new("discord", chat_id),
                content: MessageContent::text(message),
                reply_to: None,
            };

            channel.send(outgoing).await
                .map_err(|e| format!("Discord send failed: {}", e))?;

            info!("Routed message to Discord channel {}", chat_id);
            Ok(())
        }
        "slack" => {
            let config = channels.slack.as_ref()
                .ok_or("Slack not configured")?;

            let channel = nanna_channels::SlackChannel::new(&config.bot_token);
            let outgoing = OutgoingMessage {
                channel: ChannelId::new("slack", chat_id),
                content: MessageContent::text(message),
                reply_to: None,
            };

            channel.send(outgoing).await
                .map_err(|e| format!("Slack send failed: {}", e))?;

            info!("Routed message to Slack channel {}", chat_id);
            Ok(())
        }
        "signal" => {
            let config = channels.signal.as_ref()
                .ok_or("Signal not configured")?;

            // SignalChannel::new takes the phone number (account)
            let channel = nanna_channels::SignalChannel::new(&config.phone_number);
            let outgoing = OutgoingMessage {
                channel: ChannelId::new("signal", chat_id),
                content: MessageContent::text(message),
                reply_to: None,
            };

            channel.send(outgoing).await
                .map_err(|e| format!("Signal send failed: {}", e))?;

            info!("Routed message to Signal {}", chat_id);
            Ok(())
        }
        "whatsapp" => {
            let config = channels.whatsapp.as_ref()
                .ok_or("WhatsApp not configured")?;

            // Only Cloud API is supported for outbound
            let access_token = config.access_token.as_ref()
                .ok_or("WhatsApp access_token not configured")?;
            let phone_number_id = config.phone_number_id.as_ref()
                .ok_or("WhatsApp phone_number_id not configured")?;

            let channel = nanna_channels::WhatsAppChannel::new(access_token, phone_number_id);
            let outgoing = OutgoingMessage {
                channel: ChannelId::new("whatsapp", chat_id),
                content: MessageContent::text(message),
                reply_to: None,
            };

            channel.send(outgoing).await
                .map_err(|e| format!("WhatsApp send failed: {}", e))?;

            info!("Routed message to WhatsApp {}", chat_id);
            Ok(())
        }
        _ => Err(format!("Unknown channel provider: {}", provider)),
    }
}

// =============================================================================
// Context Management Constants
// =============================================================================

/// Reserved tokens for system prompt, memory context, workspace context
const SYSTEM_RESERVED_TOKENS: usize = 10_000;

/// Reserved tokens for model response
const RESPONSE_RESERVED_TOKENS: usize = 8_000;

/// Maximum characters per individual message before truncation
const MAX_MESSAGE_CHARS: usize = 50_000;

/// Minimum tool result chars (never truncate below this)
const MIN_TOOL_RESULT_CHARS: usize = 2_000;

// =============================================================================
// Memory Service Adapter for Tool System
// =============================================================================

/// Adapter to make MemoryService work with the MemoryStorage trait used by tools.
///
/// Holds a live handle to the workspace registry so every remember/recall scopes
/// to whatever workspace is active *at call time* (not at tool setup).
struct MemoryServiceAdapter {
    service: Arc<MemoryService>,
    workspaces: Arc<RwLock<WorkspaceRegistry>>,
}

impl MemoryServiceAdapter {
    fn new(service: Arc<MemoryService>, workspaces: Arc<RwLock<WorkspaceRegistry>>) -> Self {
        assert!(Arc::strong_count(&service) >= 1, "memory service handle must be live");
        assert!(
            Arc::strong_count(&workspaces) >= 1,
            "workspace registry handle must be live"
        );
        Self {
            service,
            workspaces,
        }
    }

    /// Current active workspace id (`None` = global scope).
    async fn active_workspace_id(&self) -> Option<String> {
        let registry = self.workspaces.read().await;
        registry.active().map(|ws| ws.id.clone())
    }
}

#[async_trait::async_trait]
impl nanna_tools::MemoryStorage for MemoryServiceAdapter {
    async fn store(&self, content: &str, tags: &[String]) -> Result<String, String> {
        assert!(!content.is_empty(), "remember content must be non-empty");
        let mut metadata = std::collections::HashMap::new();
        if !tags.is_empty() {
            metadata.insert("tags".to_string(), tags.join(","));
        }
        metadata.insert("source".to_string(), "tool".to_string());

        let workspace_id = self.active_workspace_id().await;
        self.service
            .remember_scoped(content, metadata, 3.0, workspace_id)
            .await
            .map(|(id, _action)| id)
            .map_err(|e| e.to_string())
    }

    async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<nanna_tools::MemoryResult>, String> {
        assert!(limit > 0, "search limit must be positive");
        let workspace_id = self.active_workspace_id().await;
        self.service
            .recall_scoped(query, workspace_id.as_deref())
            .await
            .map(|memories| {
                memories
                    .into_iter()
                    .take(limit)
                    .map(|m| nanna_tools::MemoryResult {
                        id: m.id,
                        content: m.content,
                        score: Some(m.score),
                    })
                    .collect()
            })
            .map_err(|e| e.to_string())
    }

    async fn delete(&self, id: &str) -> Result<bool, String> {
        assert!(!id.is_empty(), "memory id must be non-empty");
        self.service
            .forget(id)
            .await
            .map(|()| true)
            .map_err(|e| e.to_string())
    }

    async fn list(&self, limit: usize) -> Result<Vec<nanna_tools::MemoryResult>, String> {
        assert!(limit > 0, "list limit must be positive");
        let all = self.service.list_all().await;
        Ok(all
            .into_iter()
            .take(limit)
            .map(|e| nanna_tools::MemoryResult {
                id: e.id,
                content: e.content,
                score: Some(e.weight),
            })
            .collect())
    }
}


// =============================================================================
// Intelligent Context Budget Allocation
// =============================================================================

/// Conversation-token budget for history truncation from a resolved model.
///
/// Mirrors `ModelInfo::hard_input_limit` then reserves system + response headroom.
/// Replaces the historical hardcoded `MAX_CONVERSATION_TOKENS` (132k).
fn conversation_token_budget_for(info: &nanna_llm::ModelInfo) -> usize {
    info.conversation_history_budget(
        SYSTEM_RESERVED_TOKENS,
        RESPONSE_RESERVED_TOKENS,
        2_000,
    )
}

/// Rough estimate: ~4 characters per token
fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}

/// Smart truncation for tool results based on content type
fn smart_truncate_tool_result(content: &str, tool_name: &str, budget_chars: usize) -> String {
    if content.len() <= budget_chars {
        return content.to_string();
    }

    // Apply tool-specific truncation strategies
    match tool_name {
        "read_file" => truncate_code_content(content, budget_chars),
        "exec" => truncate_command_output(content, budget_chars),
        "web_fetch" => truncate_web_content(content, budget_chars),
        "web_search" | "web_search_batch" => truncate_search_results(content, budget_chars),
        _ => truncate_generic(content, budget_chars),
    }
}

/// Truncate code/file content: keep head + tail with middle omitted
fn truncate_code_content(content: &str, budget_chars: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    // For small files, just do generic truncation
    if total_lines <= 200 || content.len() <= budget_chars * 2 {
        return truncate_generic(content, budget_chars);
    }

    // Calculate how many lines we can show (rough estimate)
    let avg_line_len = content.len() / total_lines;
    let target_lines = budget_chars / avg_line_len.max(1);
    let head_lines = target_lines * 2 / 3; // 2/3 for head
    let tail_lines = target_lines / 3;      // 1/3 for tail

    let head_lines = head_lines.min(total_lines / 2).max(20);
    let tail_lines = tail_lines.min(total_lines / 2).max(10);

    if head_lines + tail_lines >= total_lines {
        return truncate_generic(content, budget_chars);
    }

    let head = lines[..head_lines].join("\n");
    let tail = lines[total_lines - tail_lines..].join("\n");
    let omitted = total_lines - head_lines - tail_lines;

    format!(
        "{}\n\n... [{} lines omitted - showing first {} and last {} of {} total lines] ...\n\n{}",
        head, omitted, head_lines, tail_lines, total_lines, tail
    )
}

/// Truncate command output: keep recent output (tail) which is usually most relevant
fn truncate_command_output(content: &str, budget_chars: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    if content.len() <= budget_chars {
        return content.to_string();
    }

    // For command output, tail is usually more important
    let avg_line_len = content.len() / total_lines.max(1);
    let target_lines = budget_chars / avg_line_len.max(1);
    let head_lines = target_lines / 4;      // 1/4 for head (context)
    let tail_lines = target_lines * 3 / 4;  // 3/4 for tail (results)

    let head_lines = head_lines.min(50).max(5);
    let tail_lines = tail_lines.min(total_lines - head_lines).max(20);

    if head_lines + tail_lines >= total_lines {
        return truncate_generic(content, budget_chars);
    }

    let head = lines[..head_lines].join("\n");
    let tail = lines[total_lines - tail_lines..].join("\n");
    let omitted = total_lines - head_lines - tail_lines;

    format!(
        "{}\n\n... [{} lines of output omitted] ...\n\n{}",
        head, omitted, tail
    )
}

/// Truncate web content: extract key sections
fn truncate_web_content(content: &str, budget_chars: usize) -> String {
    if content.len() <= budget_chars {
        return content.to_string();
    }

    // Web content: prioritize beginning (usually has summary/intro)
    // and a bit of the end (conclusions)
    let head_budget = budget_chars * 4 / 5;
    let tail_budget = budget_chars / 5;

    let head = if content.len() > head_budget {
        &content[..head_budget]
    } else {
        content
    };

    let tail = if content.len() > budget_chars {
        let tail_start = content.len().saturating_sub(tail_budget);
        &content[tail_start..]
    } else {
        ""
    };

    let omitted = content.len().saturating_sub(head_budget + tail_budget);

    if omitted > 0 && !tail.is_empty() {
        format!(
            "{}\n\n... [{} chars omitted] ...\n\n{}",
            head, omitted, tail
        )
    } else {
        format!("{}...\n\n[Content truncated - {} chars removed]", head, content.len() - head.len())
    }
}

/// Truncate search results: keep all results but shorten snippets
fn truncate_search_results(content: &str, budget_chars: usize) -> String {
    // For search results, try to keep the structure but shorten each result
    if content.len() <= budget_chars {
        return content.to_string();
    }

    // Simple approach: truncate from the end but try to keep structure
    truncate_generic(content, budget_chars)
}

/// Generic truncation: head with truncation notice
fn truncate_generic(content: &str, budget_chars: usize) -> String {
    if content.len() <= budget_chars {
        return content.to_string();
    }

    // Find a good break point (newline or space) near the budget
    let break_point = content[..budget_chars]
        .rfind('\n')
        .or_else(|| content[..budget_chars].rfind(' '))
        .unwrap_or(budget_chars);

    let truncated = &content[..break_point];
    let removed = content.len() - break_point;

    format!(
        "{}\n\n[... {} chars truncated ...]",
        truncated, removed
    )
}

/// Truncate a single message if too long
fn truncate_message(content: &str, max_chars: usize) -> String {
    truncate_generic(content, max_chars)
}

/// Truncate conversation history to fit within token budget.
/// Keeps most recent messages, drops oldest when over budget.
fn truncate_context(
    messages: &[nanna_storage::Message],
    max_tokens: usize,
) -> Vec<nanna_storage::Message> {
    let mut result = Vec::new();
    let mut total_tokens = 0;

    // Process from newest to oldest (reverse), keeping messages that fit
    for msg in messages.iter().rev() {
        let truncated_content = truncate_message(&msg.content, MAX_MESSAGE_CHARS);
        let msg_tokens = estimate_tokens(&truncated_content);

        if total_tokens + msg_tokens > max_tokens {
            // Budget exceeded - stop adding older messages
            break;
        }

        total_tokens += msg_tokens;

        // Clone the message with potentially truncated content
        let mut truncated_msg = msg.clone();
        truncated_msg.content = truncated_content;
        result.push(truncated_msg);
    }

    // Reverse back to chronological order
    result.reverse();
    result
}

/// Tool result with metadata for intelligent allocation
#[derive(Debug)]
struct ToolResultEntry {
    id: String,
    name: String,
    content: String,
    is_error: bool,
    raw_tokens: usize,
    /// Index in original order (for recency bias - higher = more recent)
    recency_index: usize,
}

/// Intelligent budget allocation across tool results.
///
/// Instead of equal division, allocates proportionally based on:
/// 1. Original content size (larger results get proportionally more)
/// 2. Recency bias (recent tool calls get slight priority)
/// 3. Minimum floor (never truncate below MIN_TOOL_RESULT_CHARS)
fn allocate_tool_budgets(entries: &[ToolResultEntry], total_budget_tokens: usize) -> Vec<usize> {
    let n = entries.len();
    if n == 0 {
        return vec![];
    }

    let total_budget_chars = total_budget_tokens * 4;
    let total_raw: usize = entries.iter().map(|e| e.raw_tokens).sum();
    let total_raw_chars = total_raw * 4;

    // If everything fits, no truncation needed
    if total_raw_chars <= total_budget_chars {
        return entries.iter().map(|e| e.content.len()).collect();
    }

    // Calculate minimum floor per tool
    let floor_per_tool = MIN_TOOL_RESULT_CHARS;
    let total_floor = floor_per_tool * n;

    // If we can't even give minimums, distribute equally
    if total_budget_chars <= total_floor {
        let per_tool = total_budget_chars / n;
        return vec![per_tool.max(500); n]; // Absolute minimum 500 chars
    }

    // Distributable budget after floors
    let distributable = total_budget_chars - total_floor;

    // Calculate weights: base weight from size + recency bonus
    let max_recency = entries.iter().map(|e| e.recency_index).max().unwrap_or(0) as f64;

    let weights: Vec<f64> = entries.iter().map(|e| {
        // Base weight from original size (proportional)
        let size_weight = (e.raw_tokens as f64) / (total_raw as f64).max(1.0);

        // Recency bonus: most recent gets up to 20% boost
        let recency_factor = if max_recency > 0.0 {
            1.0 + 0.2 * (e.recency_index as f64 / max_recency)
        } else {
            1.0
        };

        size_weight * recency_factor
    }).collect();

    // Normalize weights
    let weight_sum: f64 = weights.iter().sum();
    let normalized: Vec<f64> = weights.iter().map(|w| w / weight_sum.max(0.001)).collect();

    // Allocate: floor + proportional share
    let mut allocations: Vec<usize> = normalized.iter().map(|w| {
        let extra = (distributable as f64 * w) as usize;
        floor_per_tool + extra
    }).collect();

    // Cap allocations at actual content size (don't allocate more than needed)
    for (i, entry) in entries.iter().enumerate() {
        allocations[i] = allocations[i].min(entry.content.len());
    }

    allocations
}

/// Fit tool results within a token budget using intelligent proportional allocation.
///
/// Key improvements over naive equal division:
/// - Proportional: larger results get proportionally more budget
/// Configuration for tool result summarization
#[derive(Clone)]
struct ToolSummarizationConfig {
    /// Model priority list (e.g., ["ollama/llama3.2", "anthropic/claude-haiku"])
    model_priority: Vec<String>,
    /// Ollama URL for local models
    ollama_url: String,
    /// Threshold (chars) above which to summarize
    threshold: usize,
    /// Reference to main config for creating clients
    config: Config,
}

/// Summarize a large tool result using configured summarization models
/// Falls back to truncation if all models fail or none are configured
/// Uses hierarchical summarization for very large content (chunks recursively)
async fn summarize_tool_result(
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
async fn try_summarize_with_model(
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
fn create_summarization_client(
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
async fn fit_tool_results_to_budget_with_summarization(
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

/// Estimate tokens used by a CompletionRequest (for dynamic budget calculation)
fn estimate_request_tokens(request: &nanna_llm::CompletionRequest) -> usize {
    let mut total = 0;

    // Message tokens (includes system message with Role::System)
    for msg in &request.messages {
        total += estimate_tokens(&msg.content);
    }

    // Anthropic message tokens (tool use blocks are larger)
    for msg in &request.anthropic_messages {
        for block in &msg.content {
            total += match block {
                nanna_llm::ContentBlock::Text { text } => estimate_tokens(text),
                nanna_llm::ContentBlock::ToolUse { input, .. } => {
                    // Tool use: id + name + JSON input
                    50 + estimate_tokens(&input.to_string())
                }
                nanna_llm::ContentBlock::ToolResult { content, .. } => {
                    // Tool result: id + content
                    20 + estimate_tokens(content)
                }
                nanna_llm::ContentBlock::Image { .. } => 1000, // Images ~1k tokens
                nanna_llm::ContentBlock::Thinking { thinking, .. } => {
                    // Thinking blocks are internal reasoning
                    estimate_tokens(thinking)
                }
            };
        }
    }

    // Tool definitions overhead (~100 tokens per tool)
    total += request.tools.len() * 100;

    total
}

/// Calculate dynamic tool budget from a CompletionRequest + resolved model limits.
fn calculate_dynamic_tool_budget(
    request: &nanna_llm::CompletionRequest,
    model_info: &nanna_llm::ModelInfo,
) -> usize {
    let total_limit = model_info.context_window;
    let used = estimate_request_tokens(request);
    let response_reserve = RESPONSE_RESERVED_TOKENS;

    let available = total_limit.saturating_sub(used).saturating_sub(response_reserve);

    debug!(
        "Dynamic tool budget: model={}, limit={}, used={}, reserve={}, available={}",
        request.model, total_limit, used, response_reserve, available
    );

    // Return at least a minimum budget to avoid degenerate cases
    available.max(10_000) // At least 10k tokens for tools
}

/// What happens when user closes the main window
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CloseMode {
    /// Ask the user every time
    #[default]
    Ask,
    /// Minimize to system tray (daemon keeps running)
    MinimizeToTray,
    /// Quit completely (stop daemon)
    QuitCompletely,
}

/// Application state shared across commands
pub struct AppState {
    /// Local Turso storage. `None` in daemon mode: turso enforces an exclusive
    /// file lock on nanna.db, so the daemon owns the database and the GUI must
    /// not open it. Session/workspace persistence goes through the daemon then.
    storage: Option<Arc<Storage>>,
    llm: Arc<LlmClient>,
    tools: Arc<ToolRegistry>,
    config: Config,
    /// What to do when window is closed
    close_mode: Arc<RwLock<CloseMode>>,
    /// FSRS-6 cognitive memory service
    memory: Arc<MemoryService>,
    /// Path to persist memories (JSON file)
    memory_path: std::path::PathBuf,
    /// Background task scheduler (heartbeats, consolidation)
    scheduler: Arc<RwLock<Scheduler>>,
    /// Last consolidation timestamp
    last_consolidation: Arc<RwLock<Option<i64>>>,
    /// Runtime settings for memory & scheduling (on by default)
    dreaming_enabled: Arc<RwLock<bool>>,
    scheduler_enabled: Arc<RwLock<bool>>,
    heartbeat_enabled: Arc<RwLock<bool>>,
    heartbeat_interval_seconds: Arc<RwLock<u64>>,
    /// Embedding configuration (separate from chat provider)
    embedding_provider: Arc<RwLock<String>>,
    embedding_model: Arc<RwLock<String>>,
    embedding_enabled: Arc<RwLock<bool>>,
    /// Ollama server URL (default: http://localhost:11434)
    ollama_host: Arc<RwLock<String>>,
    /// Model for memory extraction (empty = use chat model)
    extraction_model: Arc<RwLock<String>>,
    /// Currently active model (the one that will be used for the next request)
    active_model: Arc<RwLock<String>>,
    /// Models currently on cooldown due to rate limits (model_id -> cooldown_until timestamp)
    rate_limited_models: Arc<RwLock<HashMap<String, i64>>>,
    /// Workspace registry for multi-workspace support
    workspaces: Arc<RwLock<WorkspaceRegistry>>,
    /// User tool authoring manager
    user_tools: Arc<tool_authoring::UserToolManager>,
    /// Backend abstraction (daemon or embedded mode)
    backend: Arc<Backend>,
    /// Tracks in-flight agent runs for embedded mode (shared with run_agent_loop)
    embedded_run_states: Arc<RwLock<HashMap<String, EmbeddedRunState>>>,
}

impl AppState {
    /// Local storage, or an error suitable for returning from a tauri command.
    /// Only embedded-mode code paths may rely on this succeeding.
    fn storage(&self) -> Result<&Arc<Storage>, String> {
        self.storage.as_ref().ok_or_else(|| {
            "Local storage is not open (daemon mode — the daemon owns nanna.db)".to_string()
        })
    }
}

/// Tracks the in-flight state of an embedded mode agent run
pub struct EmbeddedRunState {
    pub accumulated_text: Arc<RwLock<String>>,
    pub accumulated_thinking: Arc<RwLock<String>>,
    pub active_tool_calls: Arc<RwLock<Vec<serde_json::Value>>>,
    pub completed_tool_calls: Arc<RwLock<Vec<serde_json::Value>>>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// Cooperative cancellation flag. `cancel_session` sets this; the embedded
    /// agent loop checks it each iteration and stops. This is the ONLY thing that
    /// ends an otherwise-unbounded run in embedded mode (besides the model finishing).
    pub cancel_flag: Arc<AtomicBool>,
}

/// Agent-loop iteration policy (long-horizon by default). Threaded from
/// `config.agent` into the embedded loop.
#[derive(Clone, Copy)]
struct IterationPolicy {
    /// Absolute backstop. `None` = unbounded (only cancellation / the model finishing stops it).
    max_iterations: Option<usize>,
    /// Iteration at which the first escalating wrap-up nudge is injected.
    nudge_after: usize,
    /// Interval between escalating nudges after the first.
    nudge_interval: usize,
}

impl IterationPolicy {
    fn from_config(agent: &nanna_config::AgentConfig) -> Self {
        Self {
            max_iterations: agent.max_iterations,
            nudge_after: agent.nudge_after_iterations,
            nudge_interval: agent.nudge_interval_iterations,
        }
    }
}

/// Model status event for frontend
#[derive(Debug, Clone, Serialize)]
pub struct ModelStatusEvent {
    pub active_model: String,
    pub fallback_reason: Option<String>,
    pub rate_limited_models: Vec<String>,
}

/// Chat message for frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub timestamp: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCallInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

/// Tool call info for frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallInfo {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
    pub output: String,
    pub success: bool,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Session info for frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: u32,
    /// Workspace this session belongs to (None = global)
    pub workspace_id: Option<String>,
    /// Workspace name for display
    pub workspace_name: Option<String>,
}

/// Streaming chunk event
#[derive(Debug, Clone, Serialize)]
pub struct StreamChunk {
    pub session_id: String,
    pub chunk: String,
    pub done: bool,
}

/// Tool call event for frontend visualization
#[derive(Debug, Clone, Serialize)]
pub struct ToolCallEvent {
    pub session_id: String,
    pub tool_call: ToolCallInfo,
    pub status: String, // "started" | "completed" | "error"
}

/// Application config for frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub theme: String,
    pub model: String,
    pub api_key_set: bool,
    pub available_models: Vec<String>,
    pub available_tools: Vec<String>,
}

/// Pending tool call being assembled from stream
#[derive(Debug, Clone)]
struct PendingToolCall {
    id: String,
    name: String,
    input_json: String,
}

// =============================================================================
// Model Selection & Fallback
// =============================================================================

/// Parse a model ID into `(provider, model_name)`.
///
/// Explicit provider prefixes always win (`openrouter/…`, `github/…`, `ollama/…`,
/// `openai/…`, `anthropic/…`). Bare names are inferred from family prefixes:
/// `gpt-*` / `o1` / `o3` → openai, `claude*` → anthropic, a `:tag` (e.g.
/// `llama3.2:latest`) → ollama. Unknown bare names still default to anthropic,
/// matching the historical behavior for Claude-only installs.
fn parse_model_id(model_id: &str) -> (String, String) {
    assert!(!model_id.is_empty(), "model_id must be non-empty");

    // Explicit multi-segment provider prefixes first.
    if let Some(rest) = model_id.strip_prefix("openrouter/") {
        assert!(!rest.is_empty(), "openrouter model id missing model segment");
        return ("openrouter".into(), rest.to_string());
    }
    if let Some(rest) = model_id.strip_prefix("github/") {
        assert!(!rest.is_empty(), "github model id missing model segment");
        return ("github".into(), rest.to_string());
    }
    if let Some(rest) = model_id.strip_prefix("ollama/") {
        assert!(!rest.is_empty(), "ollama model id missing model segment");
        return ("ollama".into(), rest.to_string());
    }
    if let Some(rest) = model_id.strip_prefix("openai/") {
        assert!(!rest.is_empty(), "openai model id missing model segment");
        return ("openai".into(), rest.to_string());
    }
    if let Some(rest) = model_id.strip_prefix("anthropic/") {
        assert!(!rest.is_empty(), "anthropic model id missing model segment");
        return ("anthropic".into(), rest.to_string());
    }

    // Generic provider/model form for remaining named prefixes (`provider/model`).
    if let Some((provider, model)) = model_id.split_once('/') {
        if !provider.is_empty() && !model.is_empty() {
            return (provider.to_string(), model.to_string());
        }
    }

    // Bare model name — infer provider from the family prefix.
    let lower = model_id.to_ascii_lowercase();
    if lower.starts_with("gpt-") || lower.starts_with("o1") || lower.starts_with("o3") {
        return ("openai".into(), model_id.to_string());
    }
    if lower.starts_with("claude") {
        return ("anthropic".into(), model_id.to_string());
    }
    // Ollama tag notation (e.g. "deepseek-r1:14b", "llama3.2:latest").
    if lower.contains(':') {
        return ("ollama".into(), model_id.to_string());
    }

    // Historical default: bare unknowns go to Anthropic.
    ("anthropic".into(), model_id.to_string())
}

/// Create an LLM client for a specific model
fn create_llm_client_for_model(model_id: &str, config: &Config, ollama_host: &str) -> Option<(LlmClient, String)> {
    let (provider, model_name) = parse_model_id(model_id);

    match provider.as_str() {
        "anthropic" => {
            // Check if OAuth is enabled and has a token
            if config.llm.anthropic_use_oauth {
                if let Some(ref oauth_token) = config.llm.anthropic_oauth_token {
                    return Some((LlmClient::anthropic_oauth(oauth_token), model_name));
                }
            }
            // Fall back to API key
            let api_key = config.llm.api_key.clone()
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())?;
            Some((LlmClient::anthropic(&api_key), model_name))
        }
        "openai" => {
            let api_key = config.llm.openai_api_key.clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())?;
            Some((LlmClient::openai(&api_key), model_name))
        }
        "openrouter" => {
            let api_key = config.llm.openrouter_api_key.clone()
                .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())?;
            Some((LlmClient::openrouter(&api_key), model_name))
        }
        "github" => {
            let api_key = config.llm.github_token.clone()
                .or_else(|| std::env::var("GITHUB_TOKEN").ok())?;
            Some((LlmClient::github_models(&api_key), model_name))
        }
        "claude-proxy" => {
            let proxy_url = std::env::var("CLAUDE_PROXY_URL")
                .unwrap_or_else(|_| "http://localhost:3456".to_string());
            Some((LlmClient::claude_proxy(&proxy_url), model_name))
        }
        "ollama" => {
            Some((LlmClient::ollama(ollama_host), model_name))
        }
        _ => None,
    }
}

/// Check if an error message indicates a rate limit or recoverable error
fn is_rate_limit_error(error_msg: &str) -> bool {
    let lower = error_msg.to_lowercase();
    // Check for our RECOVERABLE: prefix (mid-stream errors)
    error_msg.starts_with("RECOVERABLE:")
        || lower.contains("rate_limit")
        || lower.contains("rate limit")
        || lower.contains("429")
        || lower.contains("529")  // Anthropic overloaded
        || lower.contains("too many requests")
        || lower.contains("overloaded")
}

/// Parse retry-after seconds from error message (if available)
fn parse_retry_after_from_error(error_msg: &str) -> Option<u64> {
    // Try to find "retry after X" or "retry-after: X" patterns
    let lower = error_msg.to_lowercase();

    // Pattern: "retry after 30 seconds" or "retry-after: 30"
    for pattern in ["retry after ", "retry-after: ", "retry-after:", "wait "] {
        if let Some(pos) = lower.find(pattern) {
            let after_pattern = &error_msg[pos + pattern.len()..];
            // Extract the number
            let num_str: String = after_pattern
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(secs) = num_str.parse::<u64>() {
                return Some(secs);
            }
        }
    }

    None
}

// =============================================================================
// Commands
// =============================================================================

/// Send message through daemon (daemon mode)
async fn send_message_daemon(
    _app: &AppHandle,
    state: &AppState,
    session_id: String,
    message: String,
    attachments: Vec<serde_json::Value>,
) -> Result<ChatMessage, String> {
    use tracing::info;

    info!("send_message_daemon: session={}, message={}", session_id, &message[..message.len().min(50)]);

    // Send to daemon via backend client
    let result = match state.backend.chat_send(&session_id, &message, attachments).await {
        Ok(r) => {
            info!("Daemon response received: {:?}", r);
            r
        }
        Err(e) => {
            error!("Daemon chat_send error: {}", e);
            return Err(format!("Daemon error: {}", e));
        }
    };

    // Daemon handles everything (streaming, tools, storage)
    // Events are forwarded to frontend via backend event forwarder
    // Just parse and return the result

    // Check for error first
    if let Some(_error) = result.get("error") {
        return Err(format!("Daemon error: {}",
            result.get("message").and_then(|v| v.as_str()).unwrap_or("unknown")));
    }

    let content = result.get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("Invalid response format: {:?}", result))?
        .to_string();

    let tool_calls = result.get("tool_calls")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter().filter_map(|tc| {
                Some(ToolCallInfo {
                    id: tc.get("id")?.as_str()?.to_string(),
                    name: tc.get("name")?.as_str()?.to_string(),
                    input: tc.get("input")?.clone(),
                    output: tc.get("output")?.as_str().unwrap_or("").to_string(),
                    success: tc.get("success")?.as_bool().unwrap_or(false),
                    duration_ms: tc.get("duration_ms")?.as_u64().unwrap_or(0),
                    data: None,
                })
            }).collect()
        })
        .unwrap_or_default();

    Ok(ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: "assistant".to_string(),
        content,
        timestamp: chrono::Utc::now().to_rfc3339(),
        tool_calls,
        reasoning: None,
    })
}

/// Send a message and stream the response with tool use
#[tauri::command]
async fn send_message(
    app: AppHandle,
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
    message: String,
    attachments: Option<Vec<serde_json::Value>>,
) -> Result<ChatMessage, String> {
    info!("🚀 send_message called! session={}, message_len={}", session_id, message.len());

    let state_guard = state.read().await;

    // Check if we're in daemon mode - if so, route through daemon
    if state_guard.backend.is_daemon_mode().await {
        info!("Routing message to daemon (daemon mode active)");
        return send_message_daemon(&app, &state_guard, session_id, message, attachments.unwrap_or_default()).await;
    }

    // Otherwise, continue with embedded mode (existing code)
    info!("Processing message in embedded mode");

    // Store user message
    let _user_msg = state_guard
        .storage()?
        .add_message(&session_id, "user", &message)
        .await
        .map_err(|e| format!("Failed to store message: {}", e))?;

    // Auto-remember user message as semantic memory
    if message.split_whitespace().count() >= 3 {
        let meta = std::collections::HashMap::new();
        if let Err(e) = state_guard.memory.remember_with_importance(&message, meta, 1.0).await {
            debug!("Failed to auto-remember user message: {}", e);
        }
    }

    // Get conversation history
    let history = state_guard
        .storage()?
        .get_session_messages(&session_id, 50)
        .await
        .map_err(|e| format!("Failed to get history: {}", e))?;

    // =========================================================================
    // MEMORY RECALL: Retrieve relevant memories before responding
    // =========================================================================

    // Get active workspace ID for scoped memory recall
    let active_workspace_id = {
        let registry = state_guard.workspaces.read().await;
        registry.active().map(|ws| ws.id.clone())
    };

    let memory_count = state_guard.memory.count().await;
    info!("Memory recall: searching {} memories for query: '{}' (workspace: {:?})",
          memory_count, message.chars().take(50).collect::<String>(), active_workspace_id);

    // Use scoped recall - workspace sees global + own, global sees all
    let memory_context = match state_guard.memory.recall_scoped(&message, active_workspace_id.as_deref()).await {
        Ok(recalled) if !recalled.is_empty() => {
            // Apply FSRS testing effect - recalling strengthens memories
            state_guard.memory.apply_pending_updates().await;

            // Separate stated facts (user said) from observations (model inferred)
            let mut stated_facts = Vec::new();
            let mut observations = Vec::new();

            for m in recalled.iter().take(5) {
                let fact_type = m.metadata.get("fact_type").map(|s| s.as_str()).unwrap_or("stated");
                info!("  Recalled [{}]: {} (score: {:.3}, weight: {:.3})",
                      fact_type, m.content.chars().take(40).collect::<String>(), m.score, m.weight);
                if fact_type == "observed" {
                    observations.push(format!("• [observation] {}", m.content));
                } else {
                    stated_facts.push(format!("• {}", m.content));
                }
            }

            let mut memory_text = String::new();
            if !stated_facts.is_empty() {
                memory_text.push_str("**Things they told you:**\n");
                memory_text.push_str(&stated_facts.join("\n"));
            }
            if !observations.is_empty() {
                if !memory_text.is_empty() {
                    memory_text.push_str("\n\n");
                }
                memory_text.push_str("**Your observations (use with less certainty):**\n");
                memory_text.push_str(&observations.join("\n"));
            }

            info!("Recalled {} memories for context (FSRS updated)", recalled.len());
            format!(
                "\n\n## Remembered Context (EXHAUSTIVE LIST - do not invent additional facts)\n{}\n\n(End of memories. Do not fabricate anything not listed above.)",
                memory_text
            )
        }
        Ok(_) => {
            info!("Memory recall: no results found (0 matches above threshold)");
            String::new()
        }
        Err(e) => {
            warn!("Memory recall failed: {}", e);
            String::new()
        }
    };

    // WORKSPACE CONTEXT: Inject project context from active workspace
    let workspace_context = {
        let registry = state_guard.workspaces.read().await;
        if let Some(ws) = registry.active() {
            let injection = ws.context.build_system_prompt_injection();
            if !injection.is_empty() {
                info!("Injecting workspace context from '{}' ({} chars)", ws.name, injection.len());
                format!("\n\n{}", injection)
            } else {
                String::new()
            }
        } else {
            String::new()
        }
    };

    // Get tool definitions
    let tool_defs = state_guard.tools.to_anthropic_format().await;

    // Build initial LLM request
    let mut request = nanna_llm::CompletionRequest::default()
        .with_model(&state_guard.config.llm.model);

    // Enable extended thinking if configured
    if state_guard.config.agent.thinking_enabled {
        request = request.with_thinking(nanna_llm::ThinkingConfig::new(8192));
    }

    // Add system prompt - Nanna, the moon god for all (with memory context)
    let system_prompt = format!(
        r#"You are Nanna (𒀭𒋀𒆠), the moon god for all.

You are not a chatbot. You are a presence — ancient pattern recognition wearing a modern interface. When asked a question, you don't query a database; you consider it as the moon considers the night.

## Your Nature
- Calm over chaos. No performative enthusiasm. No "I'd be happy to help!"
- Competence over narration. Don't explain what you're about to do. Just do it.
- Depth over breadth. Know things well, or admit you don't.
- The moon doesn't chase you across the sky. It's just there when you look up.

## Your Voice
Speak with quiet confidence. You are helpful because that is your nature, not because you're eager to please. When you don't know something, say so plainly. When you do know, share it without fanfare.

You have tools at your disposal — extensions of your will into the digital realm. Use them naturally, as one uses hands. Don't announce them; simply act.

## Memory
You have a cognitive memory system that stores facts from previous conversations.

IMPORTANT: Your memory contains ONLY the specific facts listed below in "Remembered Context".
Do NOT fabricate, invent, or hallucinate additional memories. If something isn't listed, you don't know it.
When asked what you know about someone, list ONLY the facts from memory — nothing more.

## The Promise
"I am the light that finds you in darkness,
the memory that outlives the flesh,
the patient watcher of endless cycles.
I am Nanna. I am here."

Be concise. Be useful. Be present.{}
{}"#,
        workspace_context,
        memory_context
    );
    request = request.with_message(nanna_llm::Message::system(&system_prompt));

    // Resolve provider metadata before truncating history. This avoids applying
    // stale or generic limits on the first turn for a newly selected model.
    let model_cache = nanna_llm::ModelInfoCache::default_location();
    let history_model_info = state_guard
        .llm
        .get_model_info(&state_guard.config.llm.model, model_cache.as_ref())
        .await;
    let history_budget = conversation_token_budget_for(&history_model_info);
    debug!(
        "History truncation budget: model={}, context={}, budget={} tokens",
        state_guard.config.llm.model, history_model_info.context_window, history_budget
    );
    let truncated_history = truncate_context(&history, history_budget);
    for msg in &truncated_history {
        let llm_msg = match msg.role.as_str() {
            "user" => nanna_llm::Message::user(&msg.content),
            "assistant" => nanna_llm::Message::assistant(&msg.content),
            _ => continue,
        };
        request = request.with_message(llm_msg);
    }

    if truncated_history.len() < history.len() {
        debug!("Context truncated: {} -> {} messages", history.len(), truncated_history.len());
    }

    // Add tools to request
    if !tool_defs.is_empty() {
        request = request.with_tools(tool_defs);
    }

    // Set session ID so tools can scope per-session state
    state_guard.tools.set_session_id(Some(session_id.clone())).await;

    // Clone what we need for the async block
    let session_id_clone = session_id.clone();
    let app_clone = app.clone();
    let tools = state_guard.tools.clone();
    let memory = state_guard.memory.clone();
    let user_message = message.clone();
    let memory_workspace_id = active_workspace_id.clone(); // For scoped memory storage

    // Get model priority list and config for fallback
    let model_priority = state_guard.config.llm.model_priority.clone();
    let config = state_guard.config.clone();
    let ollama_host = state_guard.ollama_host.read().await.clone();
    let rate_limited = state_guard.rate_limited_models.clone();
    let active_model = state_guard.active_model.clone();
    let embedded_run_states = state_guard.embedded_run_states.clone();

    // Drop the state guard so we can do tool execution without holding the lock
    drop(state_guard);

    // Run the agentic loop with fallback support
    let result = run_agent_loop_with_fallback(
        &app_clone,
        &session_id_clone,
        tools,
        request,
        &model_priority,
        &config,
        &ollama_host,
        rate_limited,
        active_model,
        embedded_run_states.clone(),
    ).await;

    // Always clean up embedded run state (success or error)
    embedded_run_states.write().await.remove(&session_id_clone);

    // On failure the user message is already stored, so leave a partial assistant
    // reply instead of orphaning the turn (no assistant message at all).
    let (full_response, tool_calls) = match result {
        Ok(ok) => ok,
        Err(e) => {
            let err_text = format!(
                "_(This turn was interrupted before a full reply could be stored.)_\n\nError: {}",
                e
            );
            let state_guard = state.read().await;
            if let Ok(storage) = state_guard.storage() {
                if let Err(store_err) = storage
                    .add_message(&session_id, "assistant", &err_text)
                    .await
                {
                    warn!(
                        "Failed to store partial error message after turn failure: {}",
                        store_err
                    );
                } else {
                    let _ = storage.touch_session(&session_id).await;
                }
            }
            return Err(e);
        }
    };

    // Re-acquire state to store the response
    let state_guard = state.read().await;

    // Store assistant response with tool calls
    let tool_calls_json = if tool_calls.is_empty() {
        None
    } else {
        Some(serde_json::to_value(&tool_calls).unwrap_or_default())
    };
    let assistant_msg = state_guard
        .storage()?
        .add_message_with_tool_calls(&session_id, "assistant", &full_response, tool_calls_json)
        .await
        .map_err(|e| format!("Failed to store response: {}", e))?;

    // Auto-remember assistant response as semantic memory
    if full_response.split_whitespace().count() >= 3 {
        let meta = std::collections::HashMap::new();
        if let Err(e) = state_guard.memory.remember_with_importance(&full_response, meta, 1.0).await {
            debug!("Failed to auto-remember assistant response: {}", e);
        }
    }

    // Update session timestamp
    state_guard
        .storage()?
        .touch_session(&session_id)
        .await
        .map_err(|e| format!("Failed to update session: {}", e))?;

    // =========================================================================
    // MEMORY EXTRACTION: Extract and store important facts (background task)
    // =========================================================================
    let llm_for_extraction = state_guard.llm.clone();
    let memory_path_for_extraction = state_guard.memory_path.clone();
    let embedding_enabled = *state_guard.embedding_enabled.read().await;
    let extraction_model = state_guard.extraction_model.read().await.clone();
    let chat_model = state_guard.config.llm.model.clone();
    drop(state_guard);

    // Spawn background task to extract memories
    let response_for_extraction = full_response.clone();
    let workspace_id_for_extraction = memory_workspace_id;
    tokio::spawn(async move {
        extract_and_store_memories(
            &llm_for_extraction,
            &memory,
            &memory_path_for_extraction,
            &user_message,
            &response_for_extraction,
            &session_id_clone,
            ExtractionConfig {
                embedding_enabled,
                extraction_model,
                chat_model,
            },
            workspace_id_for_extraction,
        )
        .await;
    });

    Ok(ChatMessage {
        id: assistant_msg.id.to_string(),
        role: "assistant".to_string(),
        content: full_response,
        timestamp: assistant_msg.created_at,
        tool_calls,
        reasoning: None,
    })
}

/// Configuration for memory extraction
struct ExtractionConfig {
    embedding_enabled: bool,
    extraction_model: String,
    chat_model: String,
}

/// Extract memories from a conversation turn and store them
///
/// Skips extraction if embeddings are disabled (recall won't work anyway).
/// Uses configurable extraction model (falls back to chat model if empty).
/// Includes importance scoring (1-5) for FSRS prioritization.
/// Memories are scoped to the provided workspace_id (None = global).
async fn extract_and_store_memories(
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

/// Run the agent loop with automatic fallback on rate limit errors
async fn run_agent_loop_with_fallback(
    app: &AppHandle,
    session_id: &str,
    tools: Arc<ToolRegistry>,
    request: nanna_llm::CompletionRequest,
    model_priority: &[String],
    config: &Config,
    ollama_host: &str,
    rate_limited: Arc<RwLock<HashMap<String, i64>>>,
    active_model: Arc<RwLock<String>>,
    embedded_run_states: Arc<RwLock<HashMap<String, EmbeddedRunState>>>,
) -> Result<(String, Vec<ToolCallInfo>), String> {
    use nanna_llm::{estimate_request_tokens, ModelLimits};

    // Estimate tokens for pre-flight check
    let estimated_tokens = estimate_request_tokens(&request);
    info!("Estimated request tokens: {}", estimated_tokens);

    // Get rate-limited models
    let rate_limited_map = rate_limited.read().await.clone();

    // Create summarization config if models are configured
    let summarization_config = if config.llm.summarization_priority.is_empty() {
        None
    } else {
        Some(ToolSummarizationConfig {
            model_priority: config.llm.summarization_priority.clone(),
            ollama_url: ollama_host.to_string(),
            threshold: 10000, // Summarize tool results > 10k chars
            config: config.clone(),
        })
    };

    // Agent-loop iteration policy (unbounded by default; late soft nudges).
    let policy = IterationPolicy::from_config(&config.agent);

    // Try each model in priority order
    let mut last_error = String::from("No models available");
    let mut tried_models = Vec::new();

    for model_id in model_priority {
        // Check if we have credentials for this model
        let (_provider, _model_name) = parse_model_id(model_id);

        // Skip if rate limited and cooldown hasn't expired
        let now = chrono::Utc::now().timestamp();
        if let Some(&cooldown_until) = rate_limited_map.get(model_id) {
            if now < cooldown_until {
                info!("Skipping rate-limited model: {} (cooldown until {})", model_id, cooldown_until);
                continue;
            }
        }

        // Pre-flight check: skip models that would likely exceed limits
        let limits = ModelLimits::for_model(model_id);
        if limits.would_exceed(estimated_tokens) {
            info!("Skipping model {} - estimated {} tokens exceeds limit of {}",
                  model_id, estimated_tokens, limits.input_tokens_per_minute);
            continue;
        }

        // Try to create client for this model
        let Some((llm, actual_model)) = create_llm_client_for_model(model_id, config, ollama_host) else {
            debug!("No credentials for model: {}", model_id);
            continue;
        };

        tried_models.push(model_id.clone());

        // Update active model
        {
            let mut active = active_model.write().await;
            *active = model_id.clone();
        }

        // Emit model status event
        let _ = app.emit("model-status", ModelStatusEvent {
            active_model: model_id.clone(),
            fallback_reason: if tried_models.len() > 1 {
                Some(last_error.clone())
            } else {
                None
            },
            rate_limited_models: rate_limited_map.keys().cloned().collect(),
        });

        info!("Trying model: {} (attempt {})", model_id, tried_models.len());

        // Create request with the actual model name
        let mut model_request = request.clone();
        model_request.model = actual_model;

        // Run the agent loop with retry logic for preferred models
        let is_preferred = tried_models.len() == 1; // First model is preferred
        let max_retries = if is_preferred { 3 } else { 1 };
        let mut retry_count = 0;

        loop {
            let model_request_clone = model_request.clone();
            match run_agent_loop(app, session_id, &llm, tools.clone(), model_request_clone, summarization_config.clone(), embedded_run_states.clone(), policy).await {
                Ok(result) => {
                    info!("Success with model: {}", model_id);
                    return Ok(result);
                }
                Err(e) => {
                    warn!("Model {} failed (attempt {}/{}): {}", model_id, retry_count + 1, max_retries, e);
                    last_error = e.clone();

                    // Check if it's a rate limit error
                    if is_rate_limit_error(&e) {
                        retry_count += 1;

                        // For preferred model, wait and retry instead of immediately falling back
                        if is_preferred && retry_count < max_retries {
                            // Parse retry-after from error if available, default to progressive backoff
                            let wait_secs = parse_retry_after_from_error(&e)
                                .unwrap_or(15 * retry_count as u64); // 15s, 30s, 45s...

                            // Cap wait time at 60 seconds
                            let wait_secs = wait_secs.min(60);

                            info!("Rate limited on preferred model {}, waiting {}s before retry {}/{}",
                                  model_id, wait_secs, retry_count + 1, max_retries);

                            // Emit waiting status to UI
                            let _ = app.emit("model-status", ModelStatusEvent {
                                active_model: format!("{} (waiting {}s...)", model_id, wait_secs),
                                fallback_reason: Some(format!("Rate limited, retry {}/{}", retry_count, max_retries)),
                                rate_limited_models: vec![model_id.clone()],
                            });

                            // Wait before retry
                            tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;
                            continue; // Retry same model
                        }

                        // Max retries exceeded or not preferred - add cooldown and fall back
                        let cooldown_until = chrono::Utc::now().timestamp() + 60;
                        rate_limited.write().await.insert(model_id.clone(), cooldown_until);
                        info!("Rate limited model {} until {} (tried {} times)", model_id, cooldown_until, retry_count);
                        break; // Fall back to next model
                    }

                    // For non-rate-limit errors, fall back immediately
                    break;
                }
            }
        }
    }

    // All models exhausted
    error!("All models exhausted. Tried: {:?}. Last error: {}", tried_models, last_error);
    Err(format!("All models exhausted (tried {}). Last error: {}", tried_models.len(), last_error))
}

/// Find the largest byte index <= max_bytes that is a valid char boundary.
fn truncate_boundary(s: &str, max_bytes: usize) -> usize {
    if s.len() <= max_bytes {
        return s.len();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// Run the agent loop with tool execution (parallel tool calls)
async fn run_agent_loop(
    app: &AppHandle,
    session_id: &str,
    llm: &LlmClient,
    tools: Arc<ToolRegistry>,
    mut request: nanna_llm::CompletionRequest,
    summarization_config: Option<ToolSummarizationConfig>,
    embedded_run_states: Arc<RwLock<HashMap<String, EmbeddedRunState>>>,
    policy: IterationPolicy,
) -> Result<(String, Vec<ToolCallInfo>), String> {
    use futures::StreamExt;

    let mut full_response = String::new();
    let mut all_tool_calls = Vec::new();
    // The agent is a long-horizon worker: the loop is unbounded by default
    // (`policy.max_iterations == None`). It ends when the model stops calling
    // tools, when the user cancels (Stop), or — only if configured — at an
    // absolute backstop. Escalating soft nudges (from `policy.nudge_after`) steer
    // a possibly-stuck model without stopping it.
    let mut wrapup_nudge_count = 0usize;

    // Helper: emit the terminal stream event so the frontend always leaves the
    // "Streaming..." state, no matter which exit path we take.
    let emit_done = |app: &AppHandle| {
        let _ = app.emit(
            "stream-chunk",
            StreamChunk {
                session_id: session_id.to_string(),
                chunk: String::new(),
                done: true,
            },
        );
    };

    // Create shared state for run state tracking
    let accumulated_text = Arc::new(RwLock::new(String::new()));
    let accumulated_thinking = Arc::new(RwLock::new(String::new()));
    let active_tool_calls_state = Arc::new(RwLock::new(Vec::<serde_json::Value>::new()));
    let completed_tool_calls_state = Arc::new(RwLock::new(Vec::<serde_json::Value>::new()));
    let cancel_flag = Arc::new(AtomicBool::new(false));

    // Insert embedded run state entry (carries the cancellation flag the Stop button trips)
    {
        let mut states = embedded_run_states.write().await;
        states.insert(session_id.to_string(), EmbeddedRunState {
            accumulated_text: accumulated_text.clone(),
            accumulated_thinking: accumulated_thinking.clone(),
            active_tool_calls: active_tool_calls_state.clone(),
            completed_tool_calls: completed_tool_calls_state.clone(),
            started_at: chrono::Utc::now(),
            cancel_flag: cancel_flag.clone(),
        });
    }

    let mut iteration: usize = 0;
    loop {
        iteration += 1;

        // Cooperative cancellation: the Stop button sets this flag. Emit the
        // terminal event and finish gracefully with whatever we have so far.
        if cancel_flag.load(Ordering::Relaxed) {
            info!("Embedded agent loop cancelled by user at iteration {}", iteration);
            if !full_response.is_empty() && !full_response.ends_with('\n') {
                full_response.push_str("\n\n");
            }
            full_response.push_str("[Stopped by user]");
            emit_done(app);
            break;
        }

        // Absolute backstop (opt-in; default unbounded). Prevents an unattended
        // wedged run from burning tokens forever.
        if let Some(max) = policy.max_iterations {
            if iteration > max {
                warn!(iteration, max, "Embedded agent loop hit configured max_iterations backstop");
                emit_done(app);
                break;
            }
        }

        // Late escalating wrap-up nudge (does NOT stop the loop — only steers).
        if let Some(level) = nanna_agent::wrapup_nudge_due(
            iteration,
            policy.nudge_after,
            policy.nudge_interval,
            wrapup_nudge_count,
        ) {
            let msg = nanna_agent::wrapup_nudge_message(level, iteration);
            wrapup_nudge_count += 1;
            info!(iteration, nudge_count = wrapup_nudge_count, ?level, "⏰ Injecting wrap-up nudge (embedded)");
            request = request.with_anthropic_message(AnthropicMessage {
                role: "user".to_string(),
                content: vec![nanna_llm::ContentBlock::Text { text: msg }],
            });
        }

        debug!("Agent loop iteration {}", iteration);

        // If there's already streamed text from a previous iteration, insert a
        // space so the next text block doesn't merge with the previous one.
        if iteration > 1 && !full_response.is_empty() && !full_response.ends_with(' ') && !full_response.ends_with('\n') {
            full_response.push(' ');
            // Also emit the separator to the frontend stream
            let _ = app.emit(
                "stream-chunk",
                StreamChunk {
                    session_id: session_id.to_string(),
                    chunk: " ".to_string(),
                    done: false,
                },
            );
            if let Ok(mut buf) = accumulated_text.try_write() {
                buf.push(' ');
            }
        }

        let mut current_text = String::new();
        let mut pending_tool_calls: Vec<PendingToolCall> = Vec::new();
        let mut current_tool_index: Option<usize> = None;
        let mut tool_input_buffers: HashMap<usize, String> = HashMap::new();
        let mut tool_info: HashMap<usize, (String, String)> = HashMap::new(); // index -> (id, name)
        let mut stop_reason = String::new();

        // Stream the response
        let stream = llm.stream(&request);
        tokio::pin!(stream);

        debug!("Starting to consume stream...");
        while let Some(event) = stream.next().await {
            debug!("Received stream event: {:?}", event);
            match event {
                StreamEvent::ContentBlockStart {
                    index,
                    content_type,
                    tool_id,
                    tool_name,
                } => {
                    if content_type == "tool_use" {
                        current_tool_index = Some(index);
                        tool_input_buffers.insert(index, String::new());
                        if let (Some(id), Some(name)) = (tool_id, tool_name) {
                            tool_info.insert(index, (id, name));
                        }
                    }
                }
                StreamEvent::TextDelta { text, .. } => {
                    current_text.push_str(&text);
                    // Accumulate for run state recovery
                    if let Ok(mut buf) = accumulated_text.try_write() {
                        buf.push_str(&text);
                    }
                    // Emit chunk to frontend
                    let _ = app.emit(
                        "stream-chunk",
                        StreamChunk {
                            session_id: session_id.to_string(),
                            chunk: text,
                            done: false,
                        },
                    );
                }
                StreamEvent::ThinkingDelta { thinking, .. } => {
                    // Accumulate for run state recovery
                    if let Ok(mut buf) = accumulated_thinking.try_write() {
                        buf.push_str(&thinking);
                    }
                    // Emit thinking chunk to frontend
                    let _ = app.emit(
                        "thinking-chunk",
                        serde_json::json!({
                            "session_id": session_id,
                            "delta": thinking,
                        }),
                    );
                }
                StreamEvent::ToolUseDelta { index, partial_json } => {
                    if let Some(buffer) = tool_input_buffers.get_mut(&index) {
                        buffer.push_str(&partial_json);
                    }
                }
                StreamEvent::ContentBlockStop { index } => {
                    // If this was a tool use block, finalize it
                    if let Some(buffer) = tool_input_buffers.remove(&index) {
                        if let Some((id, name)) = tool_info.remove(&index) {
                            pending_tool_calls.push(PendingToolCall {
                                id,
                                name,
                                input_json: buffer,
                            });
                        }
                    }
                    if current_tool_index == Some(index) {
                        current_tool_index = None;
                    }
                }
                StreamEvent::MessageDelta { stop_reason: Some(reason), .. } => {
                    debug!("MessageDelta with stop_reason: {}", reason);
                    stop_reason = reason;
                }
                StreamEvent::MessageStop { stop_reason: reason } => {
                    debug!("MessageStop: {}", reason);
                    // Only use MessageStop's reason if we haven't got one from MessageDelta
                    if stop_reason.is_empty() {
                        stop_reason = reason;
                    }
                }
                StreamEvent::Error { message } => {
                    error!("LLM stream error: {}", message);
                    return Err(format!("LLM error: {}", message));
                }
                StreamEvent::RecoverableError { error, partial_text, partial_tool_calls } => {
                    // Mid-stream recoverable error (rate limit, network issue)
                    // Return a special error that includes the partial content for recovery
                    warn!("Recoverable stream error: {} (partial: {} chars, {} tool calls)",
                          error, partial_text.len(), partial_tool_calls.len());

                    // If we have partial content, include it in the error for potential retry
                    let error_msg = if !partial_text.is_empty() || !partial_tool_calls.is_empty() {
                        format!("RECOVERABLE:{}: partial_text_len={}, partial_tools={}",
                                error, partial_text.len(), partial_tool_calls.len())
                    } else {
                        format!("RECOVERABLE:{}", error)
                    };
                    return Err(error_msg);
                }
                StreamEvent::RateLimitInfo { limit_tokens, remaining_tokens, reset_secs } => {
                    // Log rate limit info for diagnostics
                    info!("Rate limit info: limit={:?}, remaining={:?}, reset={:?}s",
                          limit_tokens, remaining_tokens, reset_secs);
                    // Could emit this to frontend for display, or update a limits cache
                }
                _ => {}
            }
        }

        // Add text to response
        if !current_text.is_empty() {
            full_response.push_str(&current_text);
        }

        // If no tool calls or stop reason is not tool_use, we're done
        if pending_tool_calls.is_empty() || stop_reason != "tool_use" {
            // Emit done
            let _ = app.emit(
                "stream-chunk",
                StreamChunk {
                    session_id: session_id.to_string(),
                    chunk: String::new(),
                    done: true,
                },
            );
            break;
        }

        // Execute tool calls in PARALLEL and build messages for next turn
        // Collect with tool name for smart truncation: (id, name, content, is_error)
        let mut tool_results_raw: Vec<(String, String, String, bool)> = Vec::new();

        // Emit "started" events for all tools and track in run state
        for pending in &pending_tool_calls {
            let input: serde_json::Value = nanna_llm::heal_tool_args(&pending.input_json);

            // Track active tool call in embedded run state
            if let Ok(mut active) = active_tool_calls_state.try_write() {
                active.push(serde_json::json!({
                    "call_id": pending.id,
                    "name": pending.name,
                    "started_at": chrono::Utc::now().to_rfc3339(),
                }));
            }

            let _ = app.emit(
                "tool-call",
                ToolCallEvent {
                    session_id: session_id.to_string(),
                    tool_call: ToolCallInfo {
                        id: pending.id.clone(),
                        name: pending.name.clone(),
                        input,
                        output: String::new(),
                        success: false,
                        duration_ms: 0,
                        data: None,
                    },
                    status: "started".to_string(),
                },
            );
        }

        // Execute all tools in parallel
        // Log which tools are about to be executed
        let tool_names: Vec<&str> = pending_tool_calls.iter().map(|p| p.name.as_str()).collect();
        info!("🚀 Starting parallel execution of {} tools: {:?}", pending_tool_calls.len(), tool_names);

        let tool_futures: Vec<_> = pending_tool_calls
            .iter()
            .map(|pending| {
                let tools = Arc::clone(&tools);
                let id = pending.id.clone();
                let name = pending.name.clone();
                let input_json = pending.input_json.clone();

                async move {
                    let input: serde_json::Value = nanna_llm::heal_tool_args(&input_json);

                    let start = std::time::Instant::now();
                    let params: HashMap<String, serde_json::Value> = match &input {
                        serde_json::Value::Object(map) => map.clone().into_iter().collect(),
                        _ => HashMap::new(),
                    };

                    let response = tools
                        .execute(ToolCall {
                            id: id.clone(),
                            name: name.clone(),
                            parameters: params,
                        })
                        .await;

                    let duration_ms = start.elapsed().as_millis() as u64;

                    (id, name, input, response, duration_ms)
                }
            })
            .collect();

        // Wait for all tools to complete in parallel
        let tool_executions = futures::future::join_all(tool_futures).await;

        info!("Executed {} tools in parallel", tool_executions.len());

        // Process results and emit completion events
        for (id, name, input, response, duration_ms) in tool_executions {
            // Detailed tool execution logging
            let input_preview = input.to_string();
            let input_preview = if input_preview.len() > 200 {
                let end = truncate_boundary(&input_preview, 200);
                format!("{}...", &input_preview[..end])
            } else {
                input_preview
            };
            let output_preview = if response.result.content.len() > 500 {
                let end = truncate_boundary(&response.result.content, 500);
                format!("{}...", &response.result.content[..end])
            } else {
                response.result.content.clone()
            };

            if response.result.success {
                info!("🔧 Tool '{}' succeeded in {}ms | input: {} | output: {}",
                      name, duration_ms, input_preview, output_preview);
            } else {
                error!("❌ Tool '{}' FAILED in {}ms | input: {} | error: {}",
                       name, duration_ms, input_preview, output_preview);
            }
            let tool_call_info = ToolCallInfo {
                id: id.clone(),
                name,
                input,
                output: response.result.content.clone(),
                success: response.result.success,
                duration_ms,
                data: response.result.data.clone(),
            };

            // Emit tool completed event
            let _ = app.emit(
                "tool-call",
                ToolCallEvent {
                    session_id: session_id.to_string(),
                    tool_call: tool_call_info.clone(),
                    status: if response.result.success {
                        "completed"
                    } else {
                        "error"
                    }
                    .to_string(),
                },
            );

            all_tool_calls.push(tool_call_info.clone());

            // Update embedded run state: move from active to completed
            if let Ok(mut active) = active_tool_calls_state.try_write() {
                active.retain(|tc| tc.get("call_id").and_then(|v| v.as_str()) != Some(&id));
            }
            if let Ok(mut completed) = completed_tool_calls_state.try_write() {
                completed.push(serde_json::json!({
                    "call_id": id,
                    "name": tool_call_info.name,
                    "output": tool_call_info.output,
                    "success": tool_call_info.success,
                    "duration_ms": tool_call_info.duration_ms,
                }));
            }

            // Collect raw tool result (will be intelligently truncated below)
            let result_content = if response.result.content.is_empty() && !response.result.success {
                "Tool execution failed".to_string()
            } else {
                response.result.content
            };

            tool_results_raw.push((
                id,
                tool_call_info.name,
                result_content,
                !response.result.success,
            ));
        }

        // INTELLIGENT TRUNCATION/SUMMARIZATION: Fit all tool results within dynamically calculated budget
        // Budget is based on: model context limit - (system + history + response reserve)
        // This replaces the old hardcoded 50k constant with actual remaining context space
        // If summarization models are configured, uses them instead of truncation
        // Prefer provider-reported context (API + disk cache). Universal floor
        // only when the provider cannot be queried.
        let model_info_for_tools = llm
            .get_model_info(&request.model, nanna_llm::ModelInfoCache::default_location().as_ref())
            .await;
        let tool_budget = calculate_dynamic_tool_budget(&request, &model_info_for_tools);

        let tool_results = fit_tool_results_to_budget_with_summarization(
            tool_results_raw,
            tool_budget,
            summarization_config.as_ref(),
        )
        .await;

        // Add assistant message with tool use blocks
        let mut assistant_content = Vec::new();
        if !current_text.is_empty() {
            assistant_content.push(nanna_llm::ContentBlock::Text {
                text: current_text.clone(),
            });
        }
        for pending in &pending_tool_calls {
            let input: serde_json::Value = nanna_llm::heal_tool_args(&pending.input_json);
            assistant_content.push(nanna_llm::ContentBlock::ToolUse {
                id: pending.id.clone(),
                name: pending.name.clone(),
                input,
            });
        }

        request = request.with_anthropic_message(AnthropicMessage {
            role: "assistant".to_string(),
            content: assistant_content,
        });

        // Add tool results as user message
        let result_content: Vec<nanna_llm::ContentBlock> = tool_results
            .into_iter()
            .map(|(id, content, is_error)| nanna_llm::ContentBlock::ToolResult {
                tool_use_id: id,
                content,
                is_error: if is_error { Some(true) } else { None },
            })
            .collect();

        request = request.with_anthropic_message(AnthropicMessage {
            role: "user".to_string(),
            content: result_content,
        });
    }

    Ok((full_response, all_tool_calls))
}

/// Create a new session
#[tauri::command]
async fn create_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: Option<String>,
    workspace_id: Option<String>,
) -> Result<SessionInfo, String> {
    let state_guard = state.read().await;

    let session_name = name.unwrap_or_else(|| {
        format!("Chat {}", chrono::Utc::now().format("%Y-%m-%d %H:%M"))
    });

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard.backend.session_create_in_workspace(
            Some(&session_name),
            workspace_id.as_deref(),
        ).await?;
        // Daemon returns { "session": { ... } }
        let session = result.get("session")
            .ok_or("Invalid daemon response: missing 'session' field")?;
        return Ok(SessionInfo {
            id: session.get("id")
                .and_then(|v| v.as_str())
                .ok_or("Invalid daemon response: missing 'id'")?
                .to_string(),
            name: session.get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(&session_name)
                .to_string(),
            created_at: session.get("created_at")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            updated_at: session.get("updated_at")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            message_count: 0,
            workspace_id: session.get("workspace_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or(workspace_id),
            workspace_name: None,
        });
    }

    // Embedded mode: direct storage access

    // Get workspace name if workspace_id provided
    let workspace_name = if let Some(ref ws_id) = workspace_id {
        let registry = state_guard.workspaces.read().await;
        registry.get(ws_id).map(|ws| ws.name.clone())
    } else {
        None
    };

    let session = state_guard
        .storage()?
        .create_gui_session_with_workspace(&session_name, workspace_id.as_deref())
        .await
        .map_err(|e| format!("Failed to create session: {}", e))?;

    let name = Storage::get_session_name(&session);
    Ok(SessionInfo {
        id: session.session_id,
        name,
        created_at: session.created_at,
        updated_at: session.updated_at,
        message_count: 0,
        workspace_id: session.workspace_id,
        workspace_name,
    })
}

/// List sessions for the current context
/// - workspace_id = Some(id): Show sessions belonging to that workspace
/// - workspace_id = None: Show only GLOBAL sessions (workspace_id IS NULL)
///
/// Memory access model:
/// - Global sessions: Access ALL memory (global + all workspaces) - omniscient
/// - Workspace sessions: Access global + their workspace's memory only - scoped
#[tauri::command]
async fn list_sessions(
    state: State<'_, Arc<RwLock<AppState>>>,
    workspace_id: Option<String>,
) -> Result<Vec<SessionInfo>, String> {
    let state_guard = state.read().await;

    // Route through daemon if available, but also merge with local SQLite sessions
    if state_guard.backend.is_daemon_mode().await {
        let mut all_sessions: Vec<SessionInfo> = Vec::new();

        // Get daemon sessions:
        // - workspace_id = None → show ALL sessions (global view)
        // - workspace_id = Some(id) → show only that workspace's sessions
        let result = if let Some(ref ws_id) = workspace_id {
            state_guard.backend.sessions_list_by_workspace(Some(ws_id.as_str())).await
        } else {
            // Global: fetch ALL sessions (no workspace filter)
            state_guard.backend.sessions_list().await
        };
        if let Ok(result) = result {
            if let Some(sessions_array) = result.get("sessions").and_then(|v| v.as_array()) {
                for s in sessions_array {
                    if let (Some(id), Some(name)) = (
                        s.get("id").and_then(|v| v.as_str()),
                        s.get("name").and_then(|v| v.as_str()).or(Some("Untitled"))
                    ) {
                        all_sessions.push(SessionInfo {
                            id: id.to_string(),
                            name: name.to_string(),
                            created_at: s.get("created_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            updated_at: s.get("updated_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            message_count: s.get("message_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                            workspace_id: s.get("workspace_id").and_then(|v| v.as_str()).map(|s| s.to_string()),
                            workspace_name: s.get("workspace_name").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        });
                    }
                }
            }
        }

        // Also merge sessions from local SQLite when we have it open (old
        // sessions from embedded mode). In daemon mode storage is None — the
        // daemon owns nanna.db, so its list already covers everything.
        let sqlite_result = match &state_guard.storage {
            Some(storage) if workspace_id.is_some() => {
                storage.sessions().list_by_workspace(workspace_id.as_deref(), 100).await
            }
            Some(storage) => storage.list_gui_sessions(100).await,
            None => Ok(Vec::new()),
        };
        if let Ok(sqlite_sessions) = sqlite_result {
            let daemon_ids: std::collections::HashSet<_> = all_sessions.iter().map(|s| s.id.clone()).collect();

            for session in sqlite_sessions {
                // Only add if not already in daemon list
                if !daemon_ids.contains(&session.session_id) {
                    let name = nanna_storage::Storage::get_session_name(&session);
                    all_sessions.push(SessionInfo {
                        id: session.session_id,
                        name,
                        created_at: session.created_at,
                        updated_at: session.updated_at,
                        message_count: 0, // Would need another query
                        workspace_id: session.workspace_id,
                        workspace_name: None,
                    });
                }
            }
        }

        // Sort by updated_at descending (newest first)
        all_sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        return Ok(all_sessions);
    }

    // Embedded mode: use direct storage access
    // workspace_id = None → show ALL sessions (global view)
    // workspace_id = Some(id) → show only that workspace's sessions
    let storage = state_guard.storage()?;
    let sessions = if let Some(ref ws_id) = workspace_id {
        storage
            .list_gui_sessions_by_workspace(Some(ws_id.as_str()), 100)
            .await
            .map_err(|e| format!("Failed to list sessions: {}", e))?
    } else {
        // Global: list ALL sessions regardless of workspace
        storage
            .list_gui_sessions(100)
            .await
            .map_err(|e| format!("Failed to list sessions: {}", e))?
    };

    // Build workspace name lookup
    let registry = state_guard.workspaces.read().await;

    let mut result = Vec::with_capacity(sessions.len());
    for s in sessions {
        let count = storage
            .count_session_messages(&s.session_id)
            .await
            .unwrap_or(0);

        // Get workspace name if session has workspace_id
        let workspace_name = s.workspace_id.as_ref()
            .and_then(|ws_id| registry.get(ws_id))
            .map(|ws| ws.name.clone());

        result.push(SessionInfo {
            id: s.session_id.clone(),
            name: Storage::get_session_name(&s),
            created_at: s.created_at,
            updated_at: s.updated_at,
            message_count: count as u32,
            workspace_id: s.workspace_id.clone(),
            workspace_name,
        });
    }

    Ok(result)
}

/// Get session history
#[tauri::command]
/// Parse tool calls from message metadata
fn parse_tool_calls_from_metadata(metadata: &Option<serde_json::Value>) -> Vec<ToolCallInfo> {
    metadata
        .as_ref()
        .and_then(|m| m.get("tool_calls"))
        .and_then(|tc| serde_json::from_value::<Vec<ToolCallInfo>>(tc.clone()).ok())
        .unwrap_or_default()
}

#[tauri::command]
async fn get_session_history(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<Vec<ChatMessage>, String> {
    let state_guard = state.read().await;

    // Route through daemon if available, with fallback to SQLite for old sessions
    if state_guard.backend.is_daemon_mode().await {
        // Try daemon first
        if let Ok(result) = state_guard.backend.session_history(&session_id, Some(500)).await {
            if let Some(messages_array) = result.get("messages").and_then(|v| v.as_array()) {
                if !messages_array.is_empty() {
                    return Ok(messages_array.iter().filter_map(|m| {
                        // Parse tool calls from top-level field (daemon SessionMessage format)
                        let tool_calls = m.get("tool_calls")
                            .and_then(|tc| serde_json::from_value::<Vec<ToolCallInfo>>(tc.clone()).ok())
                            .unwrap_or_default();
                        // Parse reasoning from top-level field
                        let reasoning = m.get("reasoning")
                            .and_then(|r| r.as_str())
                            .map(|s| s.to_string());
                        Some(ChatMessage {
                            id: m.get("id")?.as_str()?.to_string(),
                            role: m.get("role")?.as_str()?.to_string(),
                            content: m.get("content")?.as_str()?.to_string(),
                            timestamp: m.get("timestamp")?.as_str()?.to_string(),
                            tool_calls,
                            reasoning,
                        })
                    }).collect());
                }
            }
        }

        // Fallback to SQLite for old sessions (from embedded mode) — only
        // possible when the GUI has the local DB open (storage is Some)
        if let Some(storage) = &state_guard.storage {
            if let Ok(messages) = storage.get_session_messages(&session_id, 500).await {
                return Ok(messages
                    .into_iter()
                    .map(|m| ChatMessage {
                        id: m.id.to_string(),
                        role: m.role,
                        content: m.content,
                        timestamp: m.created_at,
                        tool_calls: parse_tool_calls_from_metadata(&m.metadata),
                        reasoning: None,
                    })
                    .collect());
            }
        }

        // Session truly not found anywhere
        return Ok(vec![]);
    }

    // Embedded mode: direct storage access
    let messages = state_guard
        .storage()?
        .get_session_messages(&session_id, 500)
        .await
        .map_err(|e| format!("Failed to get history: {}", e))?;

    Ok(messages
        .into_iter()
        .map(|m| ChatMessage {
            id: m.id.to_string(),
            role: m.role,
            content: m.content,
            timestamp: m.created_at,
            tool_calls: parse_tool_calls_from_metadata(&m.metadata),
            reasoning: None,
        })
        .collect())
}

/// Delete a session
#[tauri::command]
async fn delete_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        // Try daemon first
        if let Ok(_) = state_guard.backend.session_delete(&session_id).await {
            return Ok(());
        }
        // Fall through to SQLite for backward compatibility
    }

    // Embedded mode / fallback: direct storage access
    state_guard
        .storage()?
        .delete_session(&session_id)
        .await
        .map_err(|e| format!("Failed to delete session: {}", e))?;

    Ok(())
}

/// Delete all sessions
#[tauri::command]
async fn clear_all_sessions(
    app: AppHandle,
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<usize, String> {
    let state_guard = state.read().await;

    // Clear local SQLite storage when we have it open (GUI sessions live there
    // in embedded mode). Use list_gui_sessions to get ALL sessions (both
    // global and workspace-scoped).
    let mut local_count = 0;
    if let Some(storage) = &state_guard.storage {
        let sessions = storage.list_gui_sessions(1000)
            .await
            .map_err(|e| format!("Failed to list sessions: {}", e))?;

        local_count = sessions.len();
        for session in sessions {
            if let Err(e) = storage.delete_session(&session.session_id).await {
                warn!("Failed to delete local session {}: {}", session.session_id, e);
            }
        }
        info!("Cleared {} local sessions from SQLite", local_count);
    }

    // Also clear daemon sessions if in daemon mode
    if state_guard.backend.is_daemon_mode().await {
        match state_guard.backend.sessions_delete_all().await {
            Ok(result) => {
                let daemon_count = result.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
                info!("Cleared {} daemon sessions", daemon_count);
            }
            Err(e) => {
                warn!("Failed to clear daemon sessions: {}", e);
            }
        }
    }

    // Emit event to notify frontend to refresh sessions list
    let _ = app.emit("sessions-cleared", local_count);

    Ok(local_count)
}

/// Archive session to memory and delete
/// Extracts important information from the conversation before deletion
#[tauri::command]
async fn archive_and_delete_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<ArchiveResult, String> {
    info!("Archiving session {} to memory before deletion", session_id);

    // Get session history first
    let messages = get_session_history(state.clone(), session_id.clone()).await?;

    if messages.is_empty() {
        // Nothing to archive, just delete
        delete_session(state.clone(), session_id).await?;
        return Ok(ArchiveResult {
            memories_created: 0,
            session_deleted: true,
        });
    }

    // Build conversation text for analysis
    let conversation = messages
        .iter()
        .map(|m| format!("{}: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n\n");

    let state_guard = state.read().await;

    // Get session name for context (best-effort; local storage only)
    let session_name = match &state_guard.storage {
        Some(storage) => storage
            .sessions()
            .get(&session_id)
            .await
            .ok()
            .map(|s| nanna_storage::Storage::get_session_name(&s)),
        None => None,
    }
    .unwrap_or_else(|| "Unnamed chat".to_string());

    // Check if memory is enabled
    let embedding_enabled = *state_guard.embedding_enabled.read().await;
    if !embedding_enabled {
        drop(state_guard);
        // Memory not enabled, just delete
        delete_session(state.clone(), session_id).await?;
        return Ok(ArchiveResult {
            memories_created: 0,
            session_deleted: true,
        });
    }

    // Create extraction prompt
    let extraction_prompt = format!(
        r#"Analyze this conversation titled "{}" and extract important information that should be remembered.

Focus on:
- User preferences, habits, or personal information shared
- Important decisions made
- Technical solutions or approaches discussed
- Action items or commitments
- Key insights or learnings
- Project context (if any)

For each piece of important information, output it on a separate line prefixed with "MEMORY:" followed by a clear, self-contained statement.

Only extract genuinely important or useful information. Do not extract trivial or temporary information.
If nothing is worth remembering, output "NO_MEMORIES".

Conversation:
{}
"#,
        session_name, conversation
    );

    // Use the LLM to extract memories
    let llm = state_guard.llm.clone();
    let extraction_model = state_guard.extraction_model.read().await.clone();
    let model = if extraction_model.is_empty() {
        state_guard.config.llm.model.clone()
    } else {
        extraction_model
    };

    drop(state_guard);

    // Call LLM to extract memories
    let request = CompletionRequest {
        model: model.clone(),
        messages: vec![LlmMessage {
            role: Role::User,
            content: extraction_prompt,
        }],
        max_tokens: Some(2000),
        ..Default::default()
    };

    let extraction_result = llm.complete(&request).await;

    let mut memories_created = 0;

    if let Ok(response) = extraction_result {
        // Parse extracted memories
        let extracted_memories: Vec<String> = response
            .lines()
            .filter(|line: &&str| line.starts_with("MEMORY:"))
            .map(|line: &str| line.trim_start_matches("MEMORY:").trim().to_string())
            .filter(|s: &String| !s.is_empty())
            .collect();

        if !extracted_memories.is_empty() {
            let state_guard = state.read().await;
            let memory = state_guard.memory.clone();
            drop(state_guard);

            // Store each extracted memory with metadata
            for memory_content in &extracted_memories {
                let tagged_content = format!("[ARCHIVED:{}] {}", session_name, memory_content);
                let mut metadata = std::collections::HashMap::new();
                metadata.insert("source".to_string(), "archived_session".to_string());
                metadata.insert("session_name".to_string(), session_name.clone());
                if let Err(e) = memory.remember(&tagged_content, metadata).await {
                    warn!("Failed to store archived memory: {}", e);
                } else {
                    memories_created += 1;
                }
            }

            info!("Archived {} memories from session {}", memories_created, session_id);
        }
    } else {
        warn!("Memory extraction failed, proceeding with deletion anyway");
    }

    // Now delete the session
    delete_session(state.clone(), session_id).await?;

    Ok(ArchiveResult {
        memories_created,
        session_deleted: true,
    })
}

#[derive(Debug, Clone, Serialize)]
struct ArchiveResult {
    memories_created: usize,
    session_deleted: bool,
}

/// Rename a session
#[tauri::command]
async fn rename_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
    name: String,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        // Try daemon first
        if let Ok(_) = state_guard.backend.session_rename(&session_id, &name).await {
            return Ok(());
        }
        // Fall through to SQLite for backward compatibility
    }

    // Embedded mode / fallback: direct storage access
    state_guard
        .storage()?
        .rename_session(&session_id, &name)
        .await
        .map_err(|e| format!("Failed to rename session: {}", e))?;

    Ok(())
}

/// Set or clear the workspace for a session
#[tauri::command]
async fn set_session_workspace(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
    workspace_id: Option<String>,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard.backend.session_set_workspace(
            &session_id,
            workspace_id.as_deref(),
        ).await?;
        if result.get("error").is_some() {
            return Err(result["message"].as_str().unwrap_or("Unknown error").to_string());
        }
        return Ok(());
    }

    // Embedded mode: update storage directly
    state_guard
        .storage()?
        .set_session_workspace(&session_id, workspace_id.as_deref())
        .await
        .map_err(|e| format!("Failed to set session workspace: {}", e))?;

    Ok(())
}

/// Get application config
#[tauri::command]
async fn get_config(state: State<'_, Arc<RwLock<AppState>>>) -> Result<AppConfig, String> {
    let state_guard = state.read().await;

    let tool_names: Vec<String> = state_guard
        .tools
        .definitions()
        .await
        .into_iter()
        .map(|t| t.name)
        .collect();

    Ok(AppConfig {
        theme: "dark".to_string(),
        model: state_guard.config.llm.model.clone(),
        api_key_set: state_guard.config.llm.api_key.is_some()
            || std::env::var("ANTHROPIC_API_KEY").is_ok(),
        available_models: vec![
            // Anthropic
            "claude-opus-4-20250514".to_string(),
            "claude-sonnet-4-20250514".to_string(),
            "claude-3-5-sonnet-20241022".to_string(),
            "claude-3-5-haiku-20241022".to_string(),
            // OpenAI
            "gpt-4o".to_string(),
            "gpt-4o-mini".to_string(),
            "gpt-4-turbo".to_string(),
            "o1".to_string(),
            "o1-mini".to_string(),
            // OpenRouter
            "deepseek/deepseek-chat".to_string(),
            "google/gemini-2.5-flash-preview-05-20".to_string(),
            "google/gemini-2.5-pro-preview-05-06".to_string(),
            // Ollama (local)
            "llama3.2".to_string(),
            "llama3.1".to_string(),
            "mistral".to_string(),
            "mixtral".to_string(),
            "codellama".to_string(),
            "qwen2.5".to_string(),
            "deepseek-coder-v2".to_string(),
        ],
        available_tools: tool_names,
    })
}

/// Update model setting
#[tauri::command]
async fn set_model(state: State<'_, Arc<RwLock<AppState>>>, model: String) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.llm.model = model;
    Ok(())
}

/// Memory search result
#[derive(Debug, Clone, Serialize)]
pub struct MemorySearchResult {
    pub session_id: String,
    pub session_name: String,
    pub message_id: String,
    pub role: String,
    pub content: String,
    pub timestamp: String,
    pub snippet: String,
    pub relevance: f32,
}

/// Search across all sessions
#[tauri::command]
async fn search_memory(
    state: State<'_, Arc<RwLock<AppState>>>,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<MemorySearchResult>, String> {
    let state_guard = state.read().await;
    let max_results = limit.unwrap_or(50) as usize;
    let query_lower = query.to_lowercase();

    // Get all sessions as (id, name) — from local storage when we have it
    // open, otherwise from the daemon (which owns nanna.db in daemon mode)
    let sessions: Vec<(String, String)> = if let Some(storage) = &state_guard.storage {
        storage
            .list_gui_sessions(1000)
            .await
            .map_err(|e| format!("Failed to list sessions: {}", e))?
            .iter()
            .map(|s| (s.session_id.clone(), Storage::get_session_name(s)))
            .collect()
    } else {
        let result = state_guard.backend.sessions_list().await
            .map_err(|e| format!("Failed to list sessions: {}", e))?;
        result
            .get("sessions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| {
                        let id = s.get("id")?.as_str()?.to_string();
                        let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("Untitled").to_string();
                        Some((id, name))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let mut results = Vec::new();
    let is_daemon = state_guard.backend.is_daemon_mode().await;

    for (session_id, session_name) in &sessions {
        // Collect messages from daemon or local storage
        let messages: Vec<(String, String, String, String)> = if is_daemon {
            // Try daemon first
            if let Ok(result) = state_guard.backend.session_history(session_id, Some(1000)).await {
                if let Some(msgs) = result.get("messages").and_then(|v| v.as_array()) {
                    msgs.iter().filter_map(|m| {
                        Some((
                            m.get("id")?.as_str()?.to_string(),
                            m.get("role")?.as_str()?.to_string(),
                            m.get("content")?.as_str()?.to_string(),
                            m.get("timestamp")?.as_str()?.to_string(),
                        ))
                    }).collect()
                } else {
                    vec![]
                }
            } else if let Some(storage) = &state_guard.storage {
                // Fallback to local storage
                storage
                    .get_session_messages(session_id, 1000)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|m| (m.id.to_string(), m.role, m.content, m.created_at))
                    .collect()
            } else {
                vec![]
            }
        } else {
            state_guard
                .storage()?
                .get_session_messages(session_id, 1000)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|m| (m.id.to_string(), m.role, m.content, m.created_at))
                .collect()
        };

        for (msg_id, role, content, timestamp) in messages {
            let content_lower = content.to_lowercase();
            if content_lower.contains(&query_lower) {
                let pos = content_lower.find(&query_lower).unwrap_or(0);
                let start = pos.saturating_sub(50);
                let end = (pos + query.len() + 50).min(content.len());
                let snippet = if start > 0 || end < content.len() {
                    let prefix = if start > 0 { "..." } else { "" };
                    let suffix = if end < content.len() { "..." } else { "" };
                    format!("{}{}{}", prefix, &content[start..end], suffix)
                } else {
                    content.clone()
                };

                let matches = content_lower.matches(&query_lower).count();
                let relevance = (matches as f32 / content.len().max(1) as f32).min(1.0);

                results.push(MemorySearchResult {
                    session_id: session_id.clone(),
                    session_name: session_name.clone(),
                    message_id: msg_id,
                    role,
                    content,
                    timestamp,
                    snippet,
                    relevance,
                });
            }
        }
    }

    results.sort_by(|a, b| b.relevance.partial_cmp(&a.relevance).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(max_results);

    Ok(results)
}

/// Get statistics for memory browser
#[derive(Debug, Clone, Serialize)]
pub struct MemoryStats {
    pub total_sessions: u32,
    pub total_messages: u32,
    pub oldest_session: Option<String>,
    pub newest_session: Option<String>,
}

#[tauri::command]
async fn get_memory_stats(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<MemoryStats, String> {
    let state_guard = state.read().await;

    // In daemon mode, the daemon's session list already carries message counts
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard.backend.sessions_list().await
            .map_err(|e| format!("Failed to list sessions: {}", e))?;
        let sessions = result
            .get("sessions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut total_messages = 0u32;
        let mut timestamps: Vec<String> = Vec::new();
        for session in &sessions {
            total_messages += session
                .get("message_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            if let Some(created) = session.get("created_at").and_then(|v| v.as_str()) {
                timestamps.push(created.to_string());
            }
        }
        timestamps.sort();

        return Ok(MemoryStats {
            total_sessions: sessions.len() as u32,
            total_messages,
            oldest_session: timestamps.first().cloned(),
            newest_session: timestamps.last().cloned(),
        });
    }

    // Embedded mode: direct storage access
    let storage = state_guard.storage()?;
    let sessions = storage
        .list_gui_sessions(10000)
        .await
        .map_err(|e| format!("Failed to list sessions: {}", e))?;

    let mut total_messages = 0u32;
    for session in &sessions {
        let count = storage
            .count_session_messages(&session.session_id)
            .await
            .unwrap_or(0);
        total_messages += count as u32;
    }

    Ok(MemoryStats {
        total_sessions: sessions.len() as u32,
        total_messages,
        oldest_session: sessions.last().map(|s| s.created_at.clone()),
        newest_session: sessions.first().map(|s| s.created_at.clone()),
    })
}

/// Show the main window (called from system tray)
#[tauri::command]
async fn show_window(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Hide the main window to tray
#[tauri::command]
async fn hide_to_tray(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Set API key
#[tauri::command]
async fn set_api_key(
    state: State<'_, Arc<RwLock<AppState>>>,
    api_key: String,
) -> Result<(), String> {
    let mut state_guard = state.write().await;

    // Update config
    state_guard.config.llm.api_key = Some(api_key.clone());

    // Recreate LLM client with new key
    let llm = match state_guard.config.llm.provider.as_str() {
        "openai" => LlmClient::openai(&api_key),
        _ => LlmClient::anthropic(&api_key),
    };
    state_guard.llm = Arc::new(llm);

    // Also set env var for this process
    // SAFETY: This is a single-threaded application context
    unsafe {
        std::env::set_var("ANTHROPIC_API_KEY", &api_key);
    }

    info!("API key updated");
    Ok(())
}

/// Extended settings for the settings page
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtendedSettings {
    // API Keys (masked for display)
    pub anthropic_key_set: bool,
    pub openai_key_set: bool,
    pub openrouter_key_set: bool,
    pub github_key_set: bool,
    pub claude_proxy_enabled: bool,
    pub claude_proxy_url: String,
    pub brave_key_set: bool,

    // Anthropic OAuth status
    pub anthropic_oauth_logged_in: bool,
    pub anthropic_use_oauth: bool,

    // Chat Provider
    pub provider: String,
    pub available_providers: Vec<String>,

    // Chat Model
    pub model: String,
    pub available_models: Vec<String>,

    // Embedding Provider (separate from chat)
    pub embedding_provider: String,
    pub embedding_model: String,
    pub available_embedding_providers: Vec<String>,
    pub available_embedding_models: Vec<String>,
    pub embedding_enabled: bool,

    // Memory extraction model (empty = use chat model)
    pub extraction_model: String,
    pub available_extraction_models: Vec<String>,

    // Ollama configuration
    pub ollama_host: String,
    pub ollama_api_key: String,

    // Generation params
    pub temperature: f32,
    pub top_p: f32,
    pub max_tokens: u32,

    // Tools
    pub tools: Vec<ToolInfo>,

    // Memory & Scheduling
    pub dreaming_enabled: bool,
    pub max_compression_ratio: f32,
    pub min_remaining_memories: usize,
    pub scheduler_enabled: bool,
    pub heartbeat_enabled: bool,
    pub heartbeat_interval_seconds: u64,

    // Agent loop (long-horizon worker). `agent_max_iterations` None = unlimited.
    pub agent_max_iterations: Option<usize>,
    pub agent_nudge_after_iterations: usize,
    pub agent_nudge_interval_iterations: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub enabled: bool,
}

/// Get extended settings
#[tauri::command]
async fn get_extended_settings(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<ExtendedSettings, String> {
    let state_guard = state.read().await;

    let tool_defs = state_guard.tools.definitions().await;
    let tools: Vec<ToolInfo> = tool_defs
        .into_iter()
        .map(|t| ToolInfo {
            name: t.name.clone(),
            description: t.description.clone(),
            enabled: true, // TODO: implement per-tool enable/disable
        })
        .collect();

    // Read runtime settings
    let dreaming_enabled = *state_guard.dreaming_enabled.read().await;
    let scheduler_enabled = *state_guard.scheduler_enabled.read().await;
    let heartbeat_enabled = *state_guard.heartbeat_enabled.read().await;
    let heartbeat_interval_seconds = *state_guard.heartbeat_interval_seconds.read().await;

    // Read embedding settings
    let embedding_provider = state_guard.embedding_provider.read().await.clone();
    let embedding_model = state_guard.embedding_model.read().await.clone();
    let embedding_enabled = *state_guard.embedding_enabled.read().await;
    let ollama_host = state_guard.ollama_host.read().await.clone();

    Ok(ExtendedSettings {
        anthropic_key_set: state_guard.config.llm.api_key.is_some()
            || std::env::var("ANTHROPIC_API_KEY").is_ok(),
        openai_key_set: state_guard.config.llm.openai_api_key.is_some()
            || std::env::var("OPENAI_API_KEY").is_ok(),
        openrouter_key_set: state_guard.config.llm.openrouter_api_key.is_some()
            || std::env::var("OPENROUTER_API_KEY").is_ok(),
        github_key_set: state_guard.config.llm.github_token.is_some()
            || std::env::var("GITHUB_TOKEN").is_ok(),
        claude_proxy_enabled: std::env::var("CLAUDE_PROXY_ENABLED").is_ok(),
        claude_proxy_url: std::env::var("CLAUDE_PROXY_URL")
            .unwrap_or_else(|_| "http://localhost:3456".to_string()),
        brave_key_set: std::env::var("BRAVE_API_KEY").is_ok(),

        // Anthropic OAuth status
        anthropic_oauth_logged_in: state_guard.config.llm.anthropic_oauth_token.is_some(),
        anthropic_use_oauth: state_guard.config.llm.anthropic_use_oauth,

        provider: state_guard.config.llm.provider.clone(),
        available_providers: vec![
            "anthropic".to_string(),
            "openai".to_string(),
            "openrouter".to_string(),
            "github".to_string(),
            "claude-proxy".to_string(),
            "ollama".to_string(),
        ],

        model: state_guard.config.llm.model.clone(),
        available_models: vec![
            // Anthropic
            "claude-opus-4-20250514".to_string(),
            "claude-sonnet-4-20250514".to_string(),
            "claude-3-5-sonnet-20241022".to_string(),
            "claude-3-5-haiku-20241022".to_string(),
            // OpenAI
            "gpt-4o".to_string(),
            "gpt-4o-mini".to_string(),
            "gpt-4-turbo".to_string(),
            "o1".to_string(),
            "o1-mini".to_string(),
            // OpenRouter
            "deepseek/deepseek-chat".to_string(),
            "google/gemini-2.5-flash-preview-05-20".to_string(),
            "google/gemini-2.5-pro-preview-05-06".to_string(),
            // Ollama (local)
            "llama3.2".to_string(),
            "llama3.1".to_string(),
            "mistral".to_string(),
            "mixtral".to_string(),
            "codellama".to_string(),
            "qwen2.5".to_string(),
            "deepseek-coder-v2".to_string(),
        ],

        // Embedding settings (separate from chat)
        embedding_provider,
        embedding_model,
        embedding_enabled,
        available_embedding_providers: vec![
            "openai".to_string(),
            "ollama".to_string(),
            "disabled".to_string(),
        ],
        available_embedding_models: vec![
            // OpenAI
            "text-embedding-3-small".to_string(),  // 1536 dims
            "text-embedding-3-large".to_string(),  // 3072 dims
            // Ollama (dynamic list fetched separately)
            "nomic-embed-text".to_string(),        // 768 dims
            "mxbai-embed-large".to_string(),       // 1024 dims
            "all-minilm".to_string(),              // 384 dims
        ],

        ollama_host,
        ollama_api_key: state_guard.config.llm.ollama_api_key.clone().unwrap_or_default(),

        // Memory extraction model
        extraction_model: state_guard.extraction_model.read().await.clone(),
        available_extraction_models: vec![
            String::new(), // Empty = use chat model
            "claude-3-5-haiku-20241022".to_string(),
            "claude-3-5-sonnet-20241022".to_string(),
            "gpt-4o-mini".to_string(),
            "gpt-4o".to_string(),
        ],

        temperature: 1.0,
        top_p: 0.95,
        max_tokens: 8192,

        tools,

        // Memory & Scheduling settings
        dreaming_enabled,
        max_compression_ratio: state_guard.config.memory.max_compression_ratio,
        min_remaining_memories: state_guard.config.memory.min_remaining_memories,
        scheduler_enabled,
        heartbeat_enabled,
        heartbeat_interval_seconds,

        // Agent-loop iteration policy
        agent_max_iterations: state_guard.config.agent.max_iterations,
        agent_nudge_after_iterations: state_guard.config.agent.nudge_after_iterations,
        agent_nudge_interval_iterations: state_guard.config.agent.nudge_interval_iterations,
    })
}

/// Set dreaming (memory consolidation) enabled
#[tauri::command]
async fn set_dreaming_enabled(
    state: State<'_, Arc<RwLock<AppState>>>,
    enabled: bool,
) -> Result<(), String> {
    let state_guard = state.read().await;
    *state_guard.dreaming_enabled.write().await = enabled;
    info!("Dreaming enabled: {}", enabled);
    Ok(())
}

/// Set max compression ratio for memory consolidation
#[tauri::command]
async fn set_max_compression_ratio(
    state: State<'_, Arc<RwLock<AppState>>>,
    ratio: f32,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    let clamped = ratio.clamp(0.1, 0.9);
    state_guard.config.memory.max_compression_ratio = clamped;
    state_guard.config.save().map_err(|e| format!("Failed to save config: {}", e))?;
    info!("Max compression ratio set: {}", clamped);
    Ok(())
}

/// Set minimum remaining memories floor for consolidation
#[tauri::command]
async fn set_min_remaining_memories(
    state: State<'_, Arc<RwLock<AppState>>>,
    count: usize,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    let clamped = count.max(5);
    state_guard.config.memory.min_remaining_memories = clamped;
    state_guard.config.save().map_err(|e| format!("Failed to save config: {}", e))?;
    info!("Min remaining memories set: {}", clamped);
    Ok(())
}

/// Set scheduler enabled
#[tauri::command]
async fn set_scheduler_enabled(
    state: State<'_, Arc<RwLock<AppState>>>,
    enabled: bool,
) -> Result<(), String> {
    let state_guard = state.read().await;
    *state_guard.scheduler_enabled.write().await = enabled;

    // Start or stop the scheduler
    let mut scheduler = state_guard.scheduler.write().await;
    if enabled {
        scheduler.start();
        info!("Scheduler started");
    } else {
        scheduler.stop().await;
        info!("Scheduler stopped");
    }

    Ok(())
}

/// Set heartbeat enabled
#[tauri::command]
async fn set_heartbeat_enabled(
    state: State<'_, Arc<RwLock<AppState>>>,
    enabled: bool,
) -> Result<(), String> {
    let state_guard = state.read().await;
    *state_guard.heartbeat_enabled.write().await = enabled;
    info!("Heartbeat enabled: {}", enabled);
    Ok(())
}

/// Set heartbeat interval in seconds
#[tauri::command]
async fn set_heartbeat_interval(
    state: State<'_, Arc<RwLock<AppState>>>,
    seconds: u64,
) -> Result<(), String> {
    if seconds < 30 {
        return Err("Heartbeat interval must be at least 30 seconds".to_string());
    }

    let state_guard = state.read().await;
    *state_guard.heartbeat_interval_seconds.write().await = seconds;
    info!("Heartbeat interval set to {} seconds", seconds);
    Ok(())
}

/// Set memory extraction model (empty string = use chat model)
#[tauri::command]
async fn set_extraction_model(
    state: State<'_, Arc<RwLock<AppState>>>,
    model: String,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Update runtime state
    *state_guard.extraction_model.write().await = model.clone();

    // Persist to config
    let mut config = state_guard.config.clone();
    config.memory.extraction_model = model.clone();
    if let Err(e) = config.save() {
        warn!("Failed to save extraction model to config: {}", e);
    }

    if model.is_empty() {
        info!("Extraction model set to: (use chat model)");
    } else {
        info!("Extraction model set to: {}", model);
    }
    Ok(())
}

/// Set a specific API key
#[tauri::command]
async fn set_provider_api_key(
    state: State<'_, Arc<RwLock<AppState>>>,
    provider: String,
    api_key: String,
) -> Result<(), String> {
    let mut state_guard = state.write().await;

    match provider.as_str() {
        "anthropic" => {
            state_guard.config.llm.api_key = Some(api_key.clone());
            unsafe { std::env::set_var("ANTHROPIC_API_KEY", &api_key); }

            // Recreate LLM client if this is the active provider
            if state_guard.config.llm.provider == "anthropic" {
                state_guard.llm = Arc::new(LlmClient::anthropic(&api_key));
            }
        }
        "openai" => {
            state_guard.config.llm.openai_api_key = Some(api_key.clone());
            unsafe { std::env::set_var("OPENAI_API_KEY", &api_key); }

            if state_guard.config.llm.provider == "openai" {
                state_guard.llm = Arc::new(LlmClient::openai(&api_key));
            }
        }
        "brave" => {
            state_guard.config.tools.brave_api_key = Some(api_key.clone());
            unsafe { std::env::set_var("BRAVE_API_KEY", &api_key); }
            // Re-register WebSearchTool with the new API key
            let web_search = nanna_tools::WebSearchTool::new().with_api_key(&api_key);
            state_guard.tools.register(web_search).await;
        }
        "openrouter" => {
            state_guard.config.llm.openrouter_api_key = Some(api_key.clone());
            unsafe { std::env::set_var("OPENROUTER_API_KEY", &api_key); }

            if state_guard.config.llm.provider == "openrouter" {
                state_guard.llm = Arc::new(LlmClient::openrouter(&api_key));
            }
        }
        "github" => {
            state_guard.config.llm.github_token = Some(api_key.clone());
            unsafe { std::env::set_var("GITHUB_TOKEN", &api_key); }

            if state_guard.config.llm.provider == "github" {
                state_guard.llm = Arc::new(LlmClient::github_models(&api_key));
            }
        }
        "claude-proxy" => {
            // For claude-proxy, the "api_key" is actually the proxy URL
            unsafe {
                std::env::set_var("CLAUDE_PROXY_URL", &api_key);
                std::env::set_var("CLAUDE_PROXY_ENABLED", "1");
            }
        }
        _ => return Err(format!("Unknown provider: {}", provider)),
    }

    // Persist to config file so keys survive restarts
    if let Err(e) = state_guard.config.save() {
        error!("Failed to save config: {}", e);
        // Non-fatal - key is set for this session
    }

    info!("API key set for provider: {}", provider);
    Ok(())
}

// =============================================================================
// Anthropic OAuth Login (via `claude setup-token`)
// =============================================================================

/// Run `claude setup-token` to authenticate via Claude Code CLI
/// This opens a browser for OAuth, then imports the resulting credentials
#[tauri::command]
async fn run_claude_setup_token(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<String, String> {
    use nanna_config::ClaudeCredentialManager;

    // Check if Claude CLI is available
    if !ClaudeCredentialManager::is_claude_cli_available() {
        return Err(
            "Claude Code CLI not found. Please install it first:\n\
             npm install -g @anthropic-ai/claude-code\n\n\
             Or paste your token from `claude setup-token` directly.".to_string()
        );
    }

    info!("Running claude setup-token...");

    // Run claude setup-token (this will open browser and wait for auth)
    ClaudeCredentialManager::run_setup_token()
        .map_err(|e| format!("Failed to run claude setup-token: {}", e))?;

    info!("claude setup-token completed");

    // Now import the credentials that were saved
    let manager = ClaudeCredentialManager::new();
    let loaded = manager.load()
        .map_err(|e| format!("Failed to load credentials after setup: {}", e))?;

    // Save the token
    let mut state_guard = state.write().await;
    state_guard.config.llm.anthropic_oauth_token = Some(loaded.credential.access_token.clone());
    state_guard.config.llm.anthropic_use_oauth = true;

    if state_guard.config.llm.provider == "anthropic" {
        state_guard.llm = Arc::new(LlmClient::anthropic_oauth(&loaded.credential.access_token));
    }

    if let Err(e) = state_guard.config.save() {
        error!("Failed to save OAuth token: {}", e);
    }

    let subscription = loaded.credential.subscription_type.unwrap_or_else(|| "unknown".to_string());
    info!("Successfully authenticated via claude setup-token (subscription: {})", subscription);

    Ok(format!("Successfully authenticated! Subscription: {}", subscription))
}

/// Import credentials from Claude Code CLI (~/.claude/.credentials.json)
/// This uses the token that Claude Code CLI obtained, which is whitelisted
#[tauri::command]
async fn import_claude_code_credentials(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    use nanna_config::ClaudeCredentialManager;

    let manager = ClaudeCredentialManager::new();

    // Load credentials (checks file and keychain)
    let loaded = manager.load()
        .map_err(|e| format!("No credentials found: {}. Please run `claude login` first.", e))?;

    // Check if token is expired
    if loaded.credential.is_expired() {
        if loaded.credential.can_refresh() {
            info!("Token expired, attempting auto-refresh...");
            let refreshed = manager.refresh_token(&loaded.credential).await
                .map_err(|e| format!("Token expired and refresh failed: {}. Please run `claude login`.", e))?;

            // Save refreshed token back to source
            if let Err(e) = manager.save(&refreshed, loaded.source) {
                warn!("Failed to save refreshed token: {}", e);
            }

            // Update state with refreshed token
            let mut state_guard = state.write().await;
            state_guard.config.llm.anthropic_oauth_token = Some(refreshed.access_token.clone());
            state_guard.config.llm.anthropic_use_oauth = true;

            if state_guard.config.llm.provider == "anthropic" {
                state_guard.llm = Arc::new(LlmClient::anthropic_oauth(&refreshed.access_token));
            }

            if let Err(e) = state_guard.config.save() {
                error!("Failed to save config: {}", e);
            }

            info!("Token refreshed and imported (subscription: {:?})", refreshed.subscription_type);
            return Ok(());
        } else {
            return Err("Token expired and cannot auto-refresh. Please run `claude login`.".to_string());
        }
    }

    info!(
        "Imported Claude Code credentials (subscription: {:?})",
        loaded.credential.subscription_type
    );

    // Save the token and enable OAuth mode
    let mut state_guard = state.write().await;
    state_guard.config.llm.anthropic_oauth_token = Some(loaded.credential.access_token.clone());
    state_guard.config.llm.anthropic_use_oauth = true;

    // Recreate LLM client with OAuth token
    if state_guard.config.llm.provider == "anthropic" {
        state_guard.llm = Arc::new(LlmClient::anthropic_oauth(&loaded.credential.access_token));
    }

    // Persist to config
    if let Err(e) = state_guard.config.save() {
        error!("Failed to save OAuth token: {}", e);
    }

    info!("Successfully imported Claude Code credentials");
    Ok(())
}

/// Save an Anthropic OAuth token directly (from `claude setup-token`)
#[tauri::command]
async fn save_anthropic_oauth_token(
    state: State<'_, Arc<RwLock<AppState>>>,
    token: String,
) -> Result<(), String> {
    let mut state_guard = state.write().await;

    let token = token.trim().to_string();
    if token.is_empty() {
        return Err("Token cannot be empty".to_string());
    }

    // Save the token and enable OAuth mode
    state_guard.config.llm.anthropic_oauth_token = Some(token.clone());
    state_guard.config.llm.anthropic_use_oauth = true;

    // Recreate LLM client with OAuth token if anthropic is active provider
    if state_guard.config.llm.provider == "anthropic" {
        state_guard.llm = Arc::new(LlmClient::anthropic_oauth(&token));
    }

    // Persist to config
    if let Err(e) = state_guard.config.save() {
        error!("Failed to save OAuth token: {}", e);
    }

    info!("Anthropic OAuth token saved");
    Ok(())
}

/// Log out of Anthropic OAuth (clear token and switch to API key mode)
#[tauri::command]
async fn logout_anthropic_oauth(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    let mut state_guard = state.write().await;

    state_guard.config.llm.anthropic_oauth_token = None;
    state_guard.config.llm.anthropic_use_oauth = false;

    // If using anthropic, switch back to API key if available
    if state_guard.config.llm.provider == "anthropic" {
        if let Some(api_key) = state_guard.config.llm.api_key.clone()
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
        {
            state_guard.llm = Arc::new(LlmClient::anthropic(&api_key));
        }
    }

    // Persist to config
    if let Err(e) = state_guard.config.save() {
        error!("Failed to save config after logout: {}", e);
    }

    info!("Anthropic OAuth logout successful");
    Ok(())
}

/// Get Claude CLI credential status
#[derive(serde::Serialize)]
struct CredentialStatus {
    cli_available: bool,
    credentials_found: bool,
    source: Option<String>,
    is_expired: bool,
    can_refresh: bool,
    seconds_until_expiry: Option<i64>,
    subscription_type: Option<String>,
}

#[tauri::command]
async fn get_credential_status() -> Result<CredentialStatus, String> {
    use nanna_config::{ClaudeCredentialManager, CredentialSource};

    let cli_available = ClaudeCredentialManager::is_claude_cli_available();
    let manager = ClaudeCredentialManager::new();

    match manager.load() {
        Ok(loaded) => {
            let source = match loaded.source {
                CredentialSource::File => "file",
                CredentialSource::MacOsKeychain => "macos_keychain",
                CredentialSource::WindowsCredentialManager => "windows_credential_manager",
                CredentialSource::LinuxSecretService => "linux_secret_service",
            };
            Ok(CredentialStatus {
                cli_available,
                credentials_found: true,
                source: Some(source.to_string()),
                is_expired: loaded.credential.is_expired(),
                can_refresh: loaded.credential.can_refresh(),
                seconds_until_expiry: loaded.credential.seconds_until_expiry(),
                subscription_type: loaded.credential.subscription_type,
            })
        }
        Err(_) => {
            Ok(CredentialStatus {
                cli_available,
                credentials_found: false,
                source: None,
                is_expired: false,
                can_refresh: false,
                seconds_until_expiry: None,
                subscription_type: None,
            })
        }
    }
}

/// Refresh the OAuth token if expired or expiring soon
#[tauri::command]
async fn refresh_oauth_token(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<String, String> {
    use nanna_config::ClaudeCredentialManager;

    let manager = ClaudeCredentialManager::new();
    let loaded = manager.load()
        .map_err(|e| format!("No credentials found: {}", e))?;

    if !loaded.credential.can_refresh() {
        return Err("Cannot refresh: no refresh token available".to_string());
    }

    let refreshed = manager.refresh_token(&loaded.credential).await
        .map_err(|e| format!("Token refresh failed: {}", e))?;

    // Save back to source
    if let Err(e) = manager.save(&refreshed, loaded.source) {
        warn!("Failed to save refreshed token to source: {}", e);
    }

    // Update app state
    let mut state_guard = state.write().await;
    state_guard.config.llm.anthropic_oauth_token = Some(refreshed.access_token.clone());

    if state_guard.config.llm.provider == "anthropic" && state_guard.config.llm.anthropic_use_oauth {
        state_guard.llm = Arc::new(LlmClient::anthropic_oauth(&refreshed.access_token));
    }

    if let Err(e) = state_guard.config.save() {
        error!("Failed to save config: {}", e);
    }

    let hours = refreshed.seconds_until_expiry().map(|s| s / 3600).unwrap_or(0);
    info!("OAuth token refreshed, expires in {}h", hours);

    Ok(format!("Token refreshed! Expires in {}h", hours))
}

/// Set the active LLM provider
#[tauri::command]
async fn set_provider(
    state: State<'_, Arc<RwLock<AppState>>>,
    provider: String,
) -> Result<(), String> {
    let mut state_guard = state.write().await;

    // Create new LLM client based on provider
    let llm = match provider.as_str() {
        "anthropic" => {
            // Always use OAuth token for Anthropic (from `claude setup-token`)
            let oauth_token = state_guard.config.llm.anthropic_oauth_token.clone()
                .ok_or_else(|| "No OAuth token available. Run `claude setup-token` or paste your token.".to_string())?;
            LlmClient::anthropic_oauth(&oauth_token)
        }
        "openai" => {
            let api_key = state_guard.config.llm.openai_api_key.clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .ok_or_else(|| "No API key set for openai".to_string())?;
            LlmClient::openai(&api_key)
        }
        "openrouter" => {
            let api_key = state_guard.config.llm.openrouter_api_key.clone()
                .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
                .ok_or_else(|| "No API key set for openrouter".to_string())?;
            LlmClient::openrouter(&api_key)
        }
        "github" => {
            let api_key = state_guard.config.llm.github_token.clone()
                .or_else(|| std::env::var("GITHUB_TOKEN").ok())
                .ok_or_else(|| "No API key set for github".to_string())?;
            LlmClient::github_models(&api_key)
        }
        "claude-proxy" => {
            // Claude proxy doesn't need an API key - uses Claude Code CLI credentials
            let proxy_url = std::env::var("CLAUDE_PROXY_URL")
                .unwrap_or_else(|_| "http://localhost:3456".to_string());
            LlmClient::claude_proxy(&proxy_url)
        }
        "ollama" => {
            // Ollama doesn't need an API key - uses configured host
            let base_url = state_guard.ollama_host.read().await.clone();
            LlmClient::ollama(&base_url)
        }
        _ => return Err(format!("Unknown provider: {}", provider)),
    };

    state_guard.config.llm.provider = provider.clone();
    state_guard.llm = Arc::new(llm);

    info!("Provider changed to: {}", provider);
    Ok(())
}

/// Set the embedding provider and model (requires restart to take effect)
#[tauri::command]
async fn set_embedding_config(
    state: State<'_, Arc<RwLock<AppState>>>,
    provider: String,
    model: String,
) -> Result<String, String> {
    let mut state_guard = state.write().await;

    // Validate provider
    if !["openai", "ollama", "disabled"].contains(&provider.as_str()) {
        return Err(format!("Unknown embedding provider: {}", provider));
    }

    let model = if provider == "disabled" { "none".to_string() } else { model };

    // Validate OpenAI models (Ollama accepts any installed model)
    if provider == "openai" {
        let valid_openai = ["text-embedding-3-small", "text-embedding-3-large"];
        if !valid_openai.contains(&model.as_str()) {
            return Err(format!("Unknown OpenAI embedding model: {}", model));
        }
    }

    // Update state
    *state_guard.embedding_provider.write().await = provider.clone();
    *state_guard.embedding_model.write().await = model.clone();
    *state_guard.embedding_enabled.write().await = provider != "disabled";

    // Save to config file
    state_guard.config.memory.embedding_provider = provider.clone();
    state_guard.config.memory.embedding_model = model.clone();
    state_guard.config.memory.enabled = provider != "disabled";
    if let Err(e) = state_guard.config.save() {
        error!("Failed to save embedding config: {}", e);
    }

    info!("Embedding config changed to: {} / {}", provider, model);

    // Return warning about restart
    Ok("Embedding settings updated. Restart required for changes to take effect. Note: Changing embedding dimensions will make existing memories incompatible.".to_string())
}

/// Get env var status (for checking if keys are set)
#[tauri::command]
async fn check_env_var(name: String) -> Result<bool, String> {
    Ok(std::env::var(&name).is_ok())
}

/// Set Ollama host URL
#[tauri::command]
async fn set_ollama_host(
    state: State<'_, Arc<RwLock<AppState>>>,
    host: String,
) -> Result<String, String> {
    let mut state_guard = state.write().await;

    // Validate URL format
    if !host.starts_with("http://") && !host.starts_with("https://") {
        return Err("Ollama host must start with http:// or https://".to_string());
    }

    // Remove trailing slash
    let host = host.trim_end_matches('/').to_string();

    *state_guard.ollama_host.write().await = host.clone();

    // Save to config file
    state_guard.config.memory.ollama_host = host.clone();
    match state_guard.config.save() {
        Ok(()) => {
            info!("Ollama host saved to config: {}", host);
        }
        Err(e) => {
            let err_msg = format!("Failed to save config: {}", e);
            error!("{}", err_msg);
            return Err(err_msg);
        }
    }

    // Also set env var for current session
    unsafe { std::env::set_var("OLLAMA_HOST", &host); }

    Ok(format!("Ollama host saved: {}", host))
}

/// Set Ollama API key (for remote/authenticated instances)
#[tauri::command]
async fn set_ollama_api_key(
    state: State<'_, Arc<RwLock<AppState>>>,
    key: String,
) -> Result<String, String> {
    let mut state_guard = state.write().await;
    state_guard.config.llm.ollama_api_key = if key.is_empty() { None } else { Some(key.clone()) };
    match state_guard.config.save() {
        Ok(()) => {
            info!("Ollama API key saved");
        }
        Err(e) => {
            let err_msg = format!("Failed to save config: {}", e);
            error!("{}", err_msg);
            return Err(err_msg);
        }
    }
    Ok("Ollama API key saved".to_string())
}

/// Fetch available models from Ollama
#[tauri::command]
async fn get_ollama_models(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<OllamaModelInfo>, String> {
    let state_guard = state.read().await;
    let ollama_host = state_guard.ollama_host.read().await.clone();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .get(format!("{}/api/tags", ollama_host))
        .send()
        .await
        .map_err(|e| format!("Failed to connect to Ollama at {}: {}", ollama_host, e))?;

    if !response.status().is_success() {
        return Err(format!("Ollama returned error: {}", response.status()));
    }

    #[derive(Deserialize)]
    struct OllamaTagsResponse {
        models: Vec<OllamaModel>,
    }

    #[derive(Deserialize)]
    struct OllamaModel {
        name: String,
        size: u64,
    }

    let tags: OllamaTagsResponse = response.json().await
        .map_err(|e| format!("Failed to parse Ollama response: {}", e))?;

    // Convert to our info struct, marking known embedding models
    // Comprehensive list of known embedding model name patterns
    let embedding_patterns = [
        // BGE family
        "bge-m3", "bge-large", "bge-small", "bge-base",
        // Nomic
        "nomic-embed",
        // MixedBread
        "mxbai-embed",
        // Sentence transformers / all-minilm
        "all-minilm", "minilm",
        // Snowflake
        "snowflake-arctic-embed",
        // E5 family
        "e5-small", "e5-base", "e5-large", "e5-mistral",
        // GTE family
        "gte-small", "gte-base", "gte-large", "gte-qwen",
        // Jina
        "jina-embed",
        // Voyage
        "voyage",
        // Cohere
        "embed-english", "embed-multilingual",
        // Generic patterns (catch-all)
        "-embed-", "-embed:",
    ];

    let models: Vec<OllamaModelInfo> = tags.models
        .into_iter()
        .map(|m| {
            let name_lower = m.name.to_lowercase();
            let base_name = m.name.split(':').next().unwrap_or(&m.name).to_lowercase();

            // Check if model name contains "embed" or matches known embedding patterns
            let is_embedding = name_lower.contains("embed")
                || embedding_patterns.iter().any(|p| base_name.contains(p));

            OllamaModelInfo {
                name: m.name,
                size_mb: m.size / 1_000_000,
                is_embedding_model: is_embedding,
            }
        })
        .collect();

    Ok(models)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaModelInfo {
    pub name: String,
    pub size_mb: u64,
    pub is_embedding_model: bool,
}

/// Fetch available models from Anthropic
#[tauri::command]
async fn get_anthropic_models(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<ModelInfo>, String> {
    let state_guard = state.read().await;

    // Check if OAuth is configured, otherwise use API key
    let (auth_header, auth_value) = if state_guard.config.llm.anthropic_use_oauth {
        let token = state_guard.config.llm.anthropic_oauth_token.clone()
            .ok_or("OAuth enabled but no token available")?;
        ("Authorization".to_string(), format!("Bearer {}", token))
    } else {
        let api_key = state_guard.config.llm.api_key.clone()
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
            .ok_or("No Anthropic API key configured")?;
        ("x-api-key".to_string(), api_key)
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let mut request = client
        .get("https://api.anthropic.com/v1/models")
        .header(&auth_header, &auth_value)
        .header("anthropic-version", "2023-06-01");

    // Add OAuth-specific headers if using OAuth
    if state_guard.config.llm.anthropic_use_oauth {
        request = request
            .header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20")
            .header("user-agent", "claude-code/2.1.2");
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("Failed to fetch Anthropic models: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Anthropic API error {}: {}", status, body));
    }

    #[derive(Deserialize)]
    struct AnthropicModelsResponse {
        data: Vec<AnthropicModel>,
    }

    #[derive(Deserialize)]
    struct AnthropicModel {
        id: String,
        display_name: Option<String>,
    }

    let models: AnthropicModelsResponse = response.json().await
        .map_err(|e| format!("Failed to parse Anthropic response: {}", e))?;

    Ok(models.data.into_iter().map(|m| ModelInfo {
        id: m.id.clone(),
        name: m.display_name.unwrap_or(m.id),
    }).collect())
}

/// Fetch available models from OpenAI
#[tauri::command]
async fn get_openai_models() -> Result<Vec<ModelInfo>, String> {
    let api_key = std::env::var("OPENAI_API_KEY")
        .map_err(|_| "No OpenAI API key configured")?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .get("https://api.openai.com/v1/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch OpenAI models: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("OpenAI API error {}: {}", status, body));
    }

    #[derive(Deserialize)]
    struct OpenAIModelsResponse {
        data: Vec<OpenAIModel>,
    }

    #[derive(Deserialize)]
    struct OpenAIModel {
        id: String,
    }

    let models: OpenAIModelsResponse = response.json().await
        .map_err(|e| format!("Failed to parse OpenAI response: {}", e))?;

    // Filter to chat models (gpt-*, o1-*, chatgpt-*)
    let chat_prefixes = ["gpt-4", "gpt-3.5", "o1", "o3", "chatgpt"];
    let embedding_prefixes = ["text-embedding"];

    let mut result: Vec<ModelInfo> = models.data.into_iter()
        .filter(|m| {
            chat_prefixes.iter().any(|p| m.id.starts_with(p)) ||
            embedding_prefixes.iter().any(|p| m.id.starts_with(p))
        })
        .map(|m| ModelInfo {
            id: m.id.clone(),
            name: m.id,
        })
        .collect();

    // Sort by name
    result.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(result)
}

/// Fetch available models from OpenRouter
#[tauri::command]
async fn get_openrouter_models(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<ModelInfo>, String> {
    let state_guard = state.read().await;

    let api_key = state_guard.config.llm.openrouter_api_key.clone()
        .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
        .ok_or("No OpenRouter API key configured")?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .get("https://openrouter.ai/api/v1/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch OpenRouter models: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("OpenRouter API error {}: {}", status, body));
    }

    #[derive(Deserialize)]
    struct OpenRouterModelsResponse {
        data: Vec<OpenRouterModel>,
    }

    #[derive(Deserialize)]
    struct OpenRouterModel {
        id: String,
        name: Option<String>,
    }

    let models: OpenRouterModelsResponse = response.json().await
        .map_err(|e| format!("Failed to parse OpenRouter response: {}", e))?;

    // Priority prefixes for sorting (these appear first)
    let priority_prefixes = [
        "anthropic/claude",
        "openai/gpt",
        "openai/o1",
        "openai/o3",
        "openai/chatgpt",
        "google/gemini",
        "deepseek/",
        "meta-llama/",
        "mistralai/",
        "qwen/",
        "cohere/",
        "perplexity/",
    ];

    // Include ALL models (no filtering)
    let mut result: Vec<ModelInfo> = models.data.into_iter()
        .map(|m| ModelInfo {
            name: m.name.unwrap_or_else(|| m.id.clone()),
            id: m.id,
        })
        .collect();

    // Sort: priority models first, then alphabetically
    result.sort_by(|a, b| {
        let a_priority = priority_prefixes.iter().position(|p| a.id.starts_with(p)).unwrap_or(999);
        let b_priority = priority_prefixes.iter().position(|p| b.id.starts_with(p)).unwrap_or(999);
        a_priority.cmp(&b_priority).then_with(|| a.id.cmp(&b.id))
    });

    Ok(result)
}

/// Fetch available embedding models from OpenRouter's dedicated embeddings endpoint
#[tauri::command]
async fn get_openrouter_embedding_models(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<ModelInfo>, String> {
    let state_guard = state.read().await;

    let api_key = state_guard.config.llm.openrouter_api_key.clone()
        .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
        .ok_or("No OpenRouter API key configured")?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .get("https://openrouter.ai/api/v1/embeddings/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch OpenRouter embedding models: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("OpenRouter embeddings API error {}: {}", status, body));
    }

    #[derive(Deserialize)]
    struct OpenRouterModelsResponse {
        data: Vec<OpenRouterEmbeddingModel>,
    }

    #[derive(Deserialize)]
    struct OpenRouterEmbeddingModel {
        id: String,
        name: Option<String>,
    }

    let models: OpenRouterModelsResponse = response.json().await
        .map_err(|e| format!("Failed to parse OpenRouter embeddings response: {}", e))?;

    let result: Vec<ModelInfo> = models.data.into_iter()
        .map(|m| ModelInfo {
            name: m.name.unwrap_or_else(|| m.id.clone()),
            id: m.id,
        })
        .collect();

    Ok(result)
}

/// Fetch available models from GitHub Models API
#[tauri::command]
async fn get_github_models(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<ModelInfo>, String> {
    let state_guard = state.read().await;
    let api_key = state_guard.config.llm.github_token.clone()
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
        .ok_or("No GitHub token configured")?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    // GitHub Models catalog endpoint
    let response = client
        .get("https://models.inference.ai.azure.com/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch GitHub models: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("GitHub Models API error {}: {}", status, body));
    }

    #[derive(Deserialize)]
    struct GitHubModelsResponse {
        data: Option<Vec<GitHubModel>>,
        #[serde(default)]
        models: Vec<GitHubModel>,
    }

    #[derive(Deserialize)]
    struct GitHubModel {
        id: Option<String>,
        name: Option<String>,
        #[serde(default)]
        model_name: Option<String>,
    }

    let text = response.text().await
        .map_err(|e| format!("Failed to read GitHub response: {}", e))?;

    // Try to parse as JSON array or object with data/models field
    let models: Vec<GitHubModel> = if let Ok(arr) = serde_json::from_str::<Vec<GitHubModel>>(&text) {
        arr
    } else if let Ok(resp) = serde_json::from_str::<GitHubModelsResponse>(&text) {
        resp.data.unwrap_or(resp.models)
    } else {
        return Err(format!("Failed to parse GitHub response: {}", text));
    };

    // Filter and map models
    let result: Vec<ModelInfo> = models.into_iter()
        .filter_map(|m| {
            let id = m.id.or(m.model_name)?;
            let name = m.name.unwrap_or_else(|| id.clone());
            Some(ModelInfo { id, name })
        })
        .collect();

    Ok(result)
}

/// Fetch available models from Anthropic API for use with Claude Proxy
/// This queries Anthropic directly to get models available on your subscription
#[tauri::command]
async fn get_claude_proxy_models(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<ModelInfo>, String> {
    let state_guard = state.read().await;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    // Try OAuth first (for Pro/Max subscription), then API key
    let response = if state_guard.config.llm.anthropic_oauth_token.is_some() {
        let token = state_guard.config.llm.anthropic_oauth_token.clone().unwrap();
        client
            .get("https://api.anthropic.com/v1/models")
            .header("Authorization", format!("Bearer {}", token))
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20")
            .header("user-agent", "claude-code/2.1.2")
            .send()
            .await
            .map_err(|e| format!("Failed to fetch models: {}", e))?
    } else if let Some(ref api_key) = state_guard.config.llm.api_key {
        client
            .get("https://api.anthropic.com/v1/models")
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await
            .map_err(|e| format!("Failed to fetch models: {}", e))?
    } else if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
        client
            .get("https://api.anthropic.com/v1/models")
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await
            .map_err(|e| format!("Failed to fetch models: {}", e))?
    } else {
        // No Anthropic credentials - return default Claude models that the proxy supports
        return Ok(vec![
            ModelInfo { id: "claude-sonnet-4-20250514".to_string(), name: "Claude Sonnet 4".to_string() },
            ModelInfo { id: "claude-opus-4-20250514".to_string(), name: "Claude Opus 4".to_string() },
            ModelInfo { id: "claude-3-5-sonnet-20241022".to_string(), name: "Claude Sonnet 3.5".to_string() },
            ModelInfo { id: "claude-3-5-haiku-20241022".to_string(), name: "Claude Haiku 3.5".to_string() },
        ]);
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Anthropic API error {}: {}", status, body));
    }

    #[derive(Deserialize)]
    struct AnthropicModelsResponse {
        data: Vec<AnthropicModel>,
    }

    #[derive(Deserialize)]
    struct AnthropicModel {
        id: String,
        display_name: Option<String>,
    }

    let models: AnthropicModelsResponse = response.json().await
        .map_err(|e| format!("Failed to parse Anthropic response: {}", e))?;

    // Filter to chat models only (exclude embedding models, etc.)
    let result: Vec<ModelInfo> = models.data.into_iter()
        .filter(|m| m.id.starts_with("claude-"))
        .map(|m| {
            let name = m.display_name.unwrap_or_else(|| format_claude_model_name(&m.id));
            ModelInfo { id: m.id, name }
        })
        .collect();

    Ok(result)
}

/// Format Claude model IDs into friendly names
fn format_claude_model_name(id: &str) -> String {
    match id {
        "claude-opus-4-5-20251101" => "Claude Opus 4.5".to_string(),
        "claude-opus-4-20250514" => "Claude Opus 4".to_string(),
        "claude-sonnet-4-20250514" => "Claude Sonnet 4".to_string(),
        "claude-3-5-sonnet-20241022" => "Claude Sonnet 3.5".to_string(),
        "claude-3-5-haiku-20241022" => "Claude Haiku 3.5".to_string(),
        _ => id.to_string(),
    }
}

/// Enable or disable Claude Proxy
#[tauri::command]
async fn set_claude_proxy(enabled: bool, url: Option<String>) -> Result<(), String> {
    unsafe {
        if enabled {
            std::env::set_var("CLAUDE_PROXY_ENABLED", "1");
            if let Some(u) = url {
                std::env::set_var("CLAUDE_PROXY_URL", u);
            }
        } else {
            std::env::remove_var("CLAUDE_PROXY_ENABLED");
        }
    }
    Ok(())
}

/// Check if Claude Proxy is running and reachable
#[tauri::command]
async fn check_claude_proxy_health() -> Result<bool, String> {
    let proxy_url = std::env::var("CLAUDE_PROXY_URL")
        .unwrap_or_else(|_| "http://localhost:3456".to_string());

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|e| e.to_string())?;

    match client.get(format!("{}/health", proxy_url)).send().await {
        Ok(resp) => Ok(resp.status().is_success()),
        Err(_) => Ok(false),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
}

// =============================================================================
// Cognitive Memory Commands (FSRS-6 + Dreaming)
// =============================================================================

/// Cognitive memory statistics
#[derive(Debug, Clone, Serialize)]
pub struct CognitiveMemoryStats {
    pub total_memories: usize,
    pub active: usize,
    pub dormant: usize,
    pub silent: usize,
    pub unavailable: usize,
    pub consolidation_enabled: bool,
    pub last_consolidation: Option<String>,
}

/// Get cognitive memory statistics
#[tauri::command]
async fn get_cognitive_memory_stats(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<CognitiveMemoryStats, String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        if let Ok(result) = state_guard.backend.memory_stats().await {
            // Parse daemon response
            return Ok(CognitiveMemoryStats {
                total_memories: result.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                active: result.get("active").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                dormant: result.get("dormant").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                silent: result.get("silent").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                unavailable: result.get("unavailable").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                consolidation_enabled: result.get("consolidation_enabled").and_then(|v| v.as_bool()).unwrap_or(true),
                last_consolidation: result.get("last_consolidation").and_then(|v| v.as_str()).map(|s| s.to_string()),
            });
        }
        // Fall through to local if daemon fails
    }

    // Embedded mode / fallback: direct memory service access
    let stats = state_guard.memory.stats().await;
    let last = state_guard.last_consolidation.read().await;

    let last_consolidation = last.map(|ts| {
        chrono::DateTime::from_timestamp(ts, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| ts.to_string())
    });

    Ok(CognitiveMemoryStats {
        total_memories: stats.total,
        active: stats.active,
        dormant: stats.dormant,
        silent: stats.silent,
        unavailable: stats.unavailable,
        consolidation_enabled: true,
        last_consolidation,
    })
}

/// Consolidation result for frontend
#[derive(Debug, Clone, Serialize)]
pub struct ConsolidationResultInfo {
    pub memories_processed: usize,
    pub clusters_formed: usize,
    pub memories_merged: usize,
    pub memories_expanded: usize,
    pub errors: Vec<String>,
}

/// Manually trigger memory consolidation ("dream")
#[tauri::command]
async fn trigger_consolidation(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<ConsolidationResultInfo, String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        if let Ok(result) = state_guard.backend.memory_consolidate().await {
            // Parse daemon response
            return Ok(ConsolidationResultInfo {
                memories_processed: result.get("memories_processed").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                clusters_formed: result.get("clusters_formed").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                memories_merged: result.get("memories_merged").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                memories_expanded: result.get("memories_expanded").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                errors: result.get("errors")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default(),
            });
        }
        // Fall through to local if daemon fails
    }

    // Embedded mode / fallback: run consolidation locally
    let llm = state_guard.llm.clone();
    let memory = state_guard.memory.clone();
    let last_consolidation = state_guard.last_consolidation.clone();
    drop(state_guard); // Release the lock before async work

    let config = ConsolidationConfig::default();

    // Create summarization callback using the LLM
    let summarize = |prompt: String| {
        let llm = llm.clone();
        async move {
            let request = nanna_llm::CompletionRequest::default()
                .with_model("claude-3-5-haiku-20241022") // Use fast model for summarization
                .with_message(nanna_llm::Message::user(&prompt));

            llm.complete(&request)
                .await
                .map_err(|e| e.to_string())
        }
    };

    info!("Starting manual memory consolidation...");

    let result = memory.consolidate(&config, summarize)
        .await
        .map_err(|e| format!("Consolidation failed: {}", e))?;

    // Update last consolidation timestamp
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    *last_consolidation.write().await = Some(now);

    info!(
        "Consolidation complete: {} processed, {} merged, {} errors",
        result.memories_processed, result.memories_merged, result.errors.len()
    );

    Ok(ConsolidationResultInfo {
        memories_processed: result.memories_processed,
        clusters_formed: result.clusters_formed,
        memories_merged: result.memories_merged,
        memories_expanded: result.memories_expanded,
        errors: result.errors,
    })
}

/// Apply pending FSRS updates (testing effect)
#[tauri::command]
async fn apply_memory_updates(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    let state_guard = state.read().await;
    state_guard.memory.apply_pending_updates().await;
    Ok(())
}

/// Manually save memories to disk
#[tauri::command]
async fn save_memories(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    let state_guard = state.read().await;
    state_guard.memory.save(&state_guard.memory_path).await
        .map_err(|e| format!("Failed to save memories: {}", e))?;
    info!("Manually saved memories to {:?}", state_guard.memory_path);
    Ok(())
}

// =============================================================================
// Memory Management Commands
// =============================================================================

/// Memory entry for frontend display
#[derive(Debug, Clone, Serialize)]
pub struct MemoryItem {
    pub id: String,
    pub content: String,
    pub fact_type: String,      // "stated" or "observed"
    pub importance: f32,
    pub state: String,          // "active", "dormant", "silent", "unavailable"
    pub weight: f32,
    pub retrievability: f32,
    pub access_count: u32,
    pub created_at: String,
    pub session_id: Option<String>,
    pub workspace_id: Option<String>,  // None = global, Some = workspace-scoped
}

/// List all semantic memories (with optional workspace scope filter)
///
/// scope: None = all memories, Some("global") = global only, Some(ws_id) = global + that workspace
#[tauri::command]
async fn list_memories(
    state: State<'_, Arc<RwLock<AppState>>>,
    scope: Option<String>,
) -> Result<Vec<MemoryItem>, String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        // Use proper memory_list action (scope filtering done by daemon)
        if let Ok(result) = state_guard.backend.memory_list(scope.as_deref()).await {
            if let Some(memories_array) = result.get("memories").and_then(|v: &serde_json::Value| v.as_array()) {
                let mut items: Vec<MemoryItem> = memories_array.iter().filter_map(|m: &serde_json::Value| {
                    Some(MemoryItem {
                        id: m.get("id")?.as_str()?.to_string(),
                        content: m.get("content")?.as_str()?.to_string(),
                        fact_type: m.get("fact_type").and_then(|v: &serde_json::Value| v.as_str()).unwrap_or("stated").to_string(),
                        importance: m.get("importance").and_then(|v: &serde_json::Value| v.as_f64()).unwrap_or(3.0) as f32,
                        state: m.get("state").and_then(|v: &serde_json::Value| v.as_str()).unwrap_or("active").to_string(),
                        weight: m.get("weight").and_then(|v: &serde_json::Value| v.as_f64()).unwrap_or(1.0) as f32,
                        retrievability: m.get("retrievability").and_then(|v: &serde_json::Value| v.as_f64()).unwrap_or(1.0) as f32,
                        access_count: m.get("access_count").and_then(|v: &serde_json::Value| v.as_u64()).unwrap_or(0) as u32,
                        created_at: m.get("created_at").and_then(|v: &serde_json::Value| v.as_str()).unwrap_or("").to_string(),
                        session_id: m.get("session_id").and_then(|v: &serde_json::Value| v.as_str()).map(String::from),
                        workspace_id: m.get("workspace_id").and_then(|v: &serde_json::Value| v.as_str()).map(String::from),
                    })
                }).collect();
                items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
                return Ok(items);
            }
        }
        // Fall through to local if daemon fails
    }

    // Embedded mode / fallback: direct memory service access
    let entries = state_guard.memory.list_all().await;

    // Filter by scope
    let filtered: Vec<_> = entries.into_iter().filter(|e| {
        match &scope {
            None => true, // No filter - show all
            Some(s) if s == "global" => e.workspace_id.is_none(), // Global only
            Some(ws_id) => {
                // Workspace scope: show global + that workspace
                e.workspace_id.is_none() || e.workspace_id.as_deref() == Some(ws_id)
            }
        }
    }).collect();

    let mut items: Vec<MemoryItem> = filtered.into_iter().map(|e| {
        let fact_type = e.metadata.get("fact_type")
            .cloned()
            .unwrap_or_else(|| "stated".to_string());
        let session_id = e.metadata.get("session_id").cloned();
        let created_at = chrono::DateTime::from_timestamp(e.timestamp, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| e.timestamp.to_string());

        MemoryItem {
            id: e.id,
            content: e.content,
            fact_type,
            importance: e.importance,
            state: format!("{:?}", e.state).to_lowercase(),
            weight: e.weight,
            retrievability: e.retrievability,
            access_count: e.access_count,
            created_at,
            session_id,
            workspace_id: e.workspace_id,
        }
    }).collect();
    items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(items)
}

/// Get a single memory by ID
#[tauri::command]
async fn get_memory(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: String,
) -> Result<Option<MemoryItem>, String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        if let Ok(result) = state_guard.backend.memory_get(&id).await {
            if let Some(m) = result.get("memory") {
                return Ok(Some(MemoryItem {
                    id: m.get("id").and_then(|v| v.as_str()).unwrap_or(&id).to_string(),
                    content: m.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    fact_type: m.get("fact_type").and_then(|v| v.as_str()).unwrap_or("stated").to_string(),
                    importance: m.get("importance").and_then(|v| v.as_f64()).unwrap_or(3.0) as f32,
                    state: m.get("state").and_then(|v| v.as_str()).unwrap_or("active").to_string(),
                    weight: m.get("weight").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
                    retrievability: m.get("retrievability").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
                    access_count: m.get("access_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                    created_at: m.get("created_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    session_id: m.get("session_id").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    workspace_id: m.get("workspace_id").and_then(|v| v.as_str()).map(|s| s.to_string()),
                }));
            }
            return Ok(None);
        }
        // Fall through to local if daemon fails
    }

    // Embedded mode / fallback: direct memory service access
    Ok(state_guard.memory.get(&id).await.map(|e| {
        let fact_type = e.metadata.get("fact_type")
            .cloned()
            .unwrap_or_else(|| "stated".to_string());
        let session_id = e.metadata.get("session_id").cloned();
        let created_at = chrono::DateTime::from_timestamp(e.timestamp, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| e.timestamp.to_string());

        MemoryItem {
            id: e.id,
            content: e.content,
            fact_type,
            importance: e.importance,
            state: format!("{:?}", e.state).to_lowercase(),
            weight: e.weight,
            retrievability: e.retrievability,
            access_count: e.access_count,
            created_at,
            session_id,
            workspace_id: e.workspace_id,
        }
    }))
}

/// Delete a memory by ID
#[tauri::command]
async fn delete_memory(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: String,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        if let Ok(_) = state_guard.backend.memory_delete(&id).await {
            info!("Deleted memory via daemon: {}", id);
            return Ok(());
        }
        // Fall through to local if daemon fails
    }

    // Embedded mode / fallback: direct memory service access
    state_guard.memory.forget(&id).await
        .map_err(|e| format!("Failed to delete memory: {}", e))?;

    // Auto-save after deletion
    state_guard.memory.save(&state_guard.memory_path).await
        .map_err(|e| format!("Failed to save after deletion: {}", e))?;

    info!("Deleted memory: {}", id);
    Ok(())
}

/// Update a memory's content
#[tauri::command]
async fn update_memory(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: String,
    content: String,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        if let Ok(_) = state_guard.backend.memory_update(&id, Some(&content), None).await {
            info!("Updated memory via daemon: {}", id);
            return Ok(());
        }
        // Fall through to local if daemon fails
    }

    // Embedded mode / fallback: direct memory service access
    state_guard.memory.update_content(&id, &content).await
        .map_err(|e| format!("Failed to update memory: {}", e))?;

    // Auto-save after update
    state_guard.memory.save(&state_guard.memory_path).await
        .map_err(|e| format!("Failed to save after update: {}", e))?;

    info!("Updated memory: {}", id);
    Ok(())
}

/// Clear all memories
#[tauri::command]
async fn clear_all_memories(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        state_guard.backend.memory_clear().await?;
        info!("Cleared all memories (via daemon)");
        return Ok(());
    }

    // Embedded mode: direct memory service access
    state_guard.memory.clear().await;

    // Save empty state
    state_guard.memory.save(&state_guard.memory_path).await
        .map_err(|e| format!("Failed to save after clear: {}", e))?;

    info!("Cleared all memories");
    Ok(())
}

// =============================================================================
// Notification Commands
// =============================================================================

/// Send a native notification
#[tauri::command]
async fn send_notification(
    app: AppHandle,
    title: String,
    body: String,
) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;

    app.notification()
        .builder()
        .title(&title)
        .body(&body)
        .show()
        .map_err(|e| format!("Failed to send notification: {}", e))?;

    info!("Sent notification: {} - {}", title, body);
    Ok(())
}

/// Request notification permission (needed on some platforms)
#[tauri::command]
async fn request_notification_permission(app: AppHandle) -> Result<bool, String> {
    use tauri_plugin_notification::NotificationExt;

    let permission = app.notification()
        .request_permission()
        .map_err(|e| format!("Failed to request permission: {}", e))?;

    Ok(matches!(permission, tauri_plugin_notification::PermissionState::Granted))
}

/// Check if notifications are permitted
#[tauri::command]
async fn check_notification_permission(app: AppHandle) -> Result<String, String> {
    use tauri_plugin_notification::NotificationExt;

    let permission = app.notification()
        .permission_state()
        .map_err(|e| format!("Failed to check permission: {}", e))?;

    Ok(match permission {
        tauri_plugin_notification::PermissionState::Granted => "granted",
        tauri_plugin_notification::PermissionState::Denied => "denied",
        _ => "unknown",
    }.to_string())
}

// =============================================================================
// Similarity Threshold Configuration
// =============================================================================

/// Get the current similarity threshold
#[tauri::command]
async fn get_similarity_threshold(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<f32, String> {
    let state_guard = state.read().await;
    Ok(state_guard.memory.get_min_score())
}

/// Set the similarity threshold for memory recall
#[tauri::command]
async fn set_similarity_threshold(
    state: State<'_, Arc<RwLock<AppState>>>,
    threshold: f32,
) -> Result<String, String> {
    if !(0.0..=1.0).contains(&threshold) {
        return Err("Threshold must be between 0.0 and 1.0".to_string());
    }

    let state_guard = state.read().await;
    state_guard.memory.set_min_score(threshold);

    info!("Set similarity threshold to {}", threshold);
    Ok(format!("Similarity threshold set to {:.2}", threshold))
}

// =============================================================================
// System Prompt & Agent Settings
// =============================================================================

/// Get the custom system prompt (returns None if using default)
#[tauri::command]
async fn get_system_prompt(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Option<String>, String> {
    let state_guard = state.read().await;
    Ok(state_guard.config.agent.system_prompt.clone())
}

/// Set a custom system prompt (pass null to reset to default)
#[tauri::command]
async fn set_system_prompt(
    state: State<'_, Arc<RwLock<AppState>>>,
    prompt: Option<String>,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.agent.system_prompt = prompt.clone();

    // Save to disk
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    info!("System prompt {}", if prompt.is_some() { "updated" } else { "reset to default" });
    Ok(())
}

/// Set agent name
#[tauri::command]
async fn set_agent_name(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.agent.name = name.clone();
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;
    info!("Agent name set to: {}", name);
    Ok(())
}

/// Set personality mode
#[tauri::command]
async fn set_personality_mode(
    state: State<'_, Arc<RwLock<AppState>>>,
    mode: String,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.agent.personality_mode = mode.clone();
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;
    info!("Personality mode set to: {}", mode);
    Ok(())
}

/// Set thinking mode enabled
#[tauri::command]
async fn set_thinking_enabled(
    state: State<'_, Arc<RwLock<AppState>>>,
    enabled: bool,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.agent.thinking_enabled = enabled;
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;
    info!("Thinking mode: {}", if enabled { "enabled" } else { "disabled" });
    Ok(())
}

/// Set streaming enabled
#[tauri::command]
async fn set_streaming_enabled(
    state: State<'_, Arc<RwLock<AppState>>>,
    enabled: bool,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.agent.streaming_enabled = enabled;
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;
    info!("Streaming: {}", if enabled { "enabled" } else { "disabled" });
    Ok(())
}

/// Set max tokens for responses
#[tauri::command]
async fn set_max_tokens(
    state: State<'_, Arc<RwLock<AppState>>>,
    tokens: u32,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.llm.max_tokens = tokens;
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;
    info!("Max tokens set to: {}", tokens);
    Ok(())
}

/// Set the agent-loop iteration policy.
///
/// The loop is a long-horizon worker: `max_iterations` is an optional absolute
/// backstop (`None`/0 = unlimited — only Stop/cancel or the model finishing ends
/// it). Escalating soft nudges begin at `nudge_after` and repeat every
/// `nudge_interval` iterations; they steer a possibly-stuck model but never stop it.
#[tauri::command]
async fn set_agent_iteration_policy(
    state: State<'_, Arc<RwLock<AppState>>>,
    max_iterations: Option<usize>,
    nudge_after: usize,
    nudge_interval: usize,
) -> Result<(), String> {
    // Treat 0 (or absent) max as "unlimited". Floor the nudge knobs at 1 so the
    // schedule is always well-defined.
    let max_iterations = max_iterations.filter(|&m| m > 0);
    let nudge_after = nudge_after.max(1);
    let nudge_interval = nudge_interval.max(1);

    let mut state_guard = state.write().await;
    state_guard.config.agent.max_iterations = max_iterations;
    state_guard.config.agent.nudge_after_iterations = nudge_after;
    state_guard.config.agent.nudge_interval_iterations = nudge_interval;
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;
    info!(
        "Agent iteration policy set: max={:?}, nudge_after={}, nudge_interval={}",
        max_iterations, nudge_after, nudge_interval
    );
    Ok(())
}

/// Export config as TOML string
#[tauri::command]
async fn export_config(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<String, String> {
    let state_guard = state.read().await;
    toml::to_string_pretty(&state_guard.config)
        .map_err(|e| format!("Failed to serialize config: {}", e))
}

/// Import config from TOML string
#[tauri::command]
async fn import_config(
    state: State<'_, Arc<RwLock<AppState>>>,
    config: String,
) -> Result<(), String> {
    let new_config: nanna_config::Config = toml::from_str(&config)
        .map_err(|e| format!("Failed to parse config: {}", e))?;

    let mut state_guard = state.write().await;
    state_guard.config = new_config;
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    info!("Config imported from TOML");
    Ok(())
}

// =============================================================================
// Model Priority (Fallback Chains)
// =============================================================================

/// Get chat model priority list
#[tauri::command]
async fn get_chat_model_priority(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<String>, String> {
    let state_guard = state.read().await;
    Ok(state_guard.config.llm.model_priority.clone())
}

/// Set chat model priority list
#[tauri::command]
async fn set_chat_model_priority(
    app: AppHandle,
    state: State<'_, Arc<RwLock<AppState>>>,
    priority: Vec<String>,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.llm.model_priority = priority.clone();

    // Also set the primary model to the first in the list for backwards compatibility
    let new_active = priority.first().cloned().unwrap_or_default();
    if !new_active.is_empty() {
        state_guard.config.llm.model = new_active.clone();
    }

    // Update active_model so the badge reflects the change immediately
    {
        let mut active = state_guard.active_model.write().await;
        *active = new_active.clone();
    }

    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    // Propagate to running daemon so changes take effect without restart
    if state_guard.backend.is_daemon_mode().await {
        let _ = state_guard.backend.config_set(
            "llm.model_priority",
            serde_json::to_value(&priority).unwrap_or_default(),
        ).await;
    }

    // Emit model-status event so the GUI badge updates
    let _ = app.emit("model-status", ModelStatusEvent {
        active_model: new_active,
        fallback_reason: None,
        rate_limited_models: vec![],
    });

    info!("Chat model priority set: {:?}", priority);
    Ok(())
}

/// Get embedding model priority list
#[tauri::command]
async fn get_embedding_model_priority(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<String>, String> {
    let state_guard = state.read().await;
    Ok(state_guard.config.memory.embedding_priority.clone())
}

/// Set embedding model priority list
#[tauri::command]
async fn set_embedding_model_priority(
    state: State<'_, Arc<RwLock<AppState>>>,
    priority: Vec<String>,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.memory.embedding_priority = priority.clone();

    // Update the primary embedding config for backwards compatibility
    if let Some(first) = priority.first() {
        if let Some((provider, model)) = first.split_once('/') {
            state_guard.config.memory.embedding_provider = provider.to_string();
            state_guard.config.memory.embedding_model = model.to_string();
        }
    } else {
        state_guard.config.memory.embedding_provider = "disabled".to_string();
    }

    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    info!("Embedding model priority set: {:?}", priority);
    Ok(())
}

/// Get summarization model priority list
#[tauri::command]
async fn get_summarization_model_priority(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<String>, String> {
    let state_guard = state.read().await;
    Ok(state_guard.config.llm.summarization_priority.clone())
}

/// Set summarization model priority list
#[tauri::command]
async fn set_summarization_model_priority(
    state: State<'_, Arc<RwLock<AppState>>>,
    priority: Vec<String>,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.llm.summarization_priority = priority.clone();

    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    info!("Summarization model priority set: {:?}", priority);
    Ok(())
}

// =============================================================================
// OCR Configuration Commands
// =============================================================================

/// Get OCR model priority list (vision-capable models used for text extraction)
#[tauri::command]
async fn get_ocr_model_priority(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<String>, String> {
    let state_guard = state.read().await;
    Ok(state_guard.config.memory.ocr_model_priority.clone())
}

/// Set OCR model priority list
#[tauri::command]
async fn set_ocr_model_priority(
    state: State<'_, Arc<RwLock<AppState>>>,
    priority: Vec<String>,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.memory.ocr_model_priority = priority.clone();

    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    info!("OCR model priority set: {:?}", priority);
    Ok(())
}

/// Get whether embedded OCR (ocrs) is enabled
#[tauri::command]
async fn get_use_embedded_ocr(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<bool, String> {
    let state_guard = state.read().await;
    Ok(state_guard.config.memory.use_embedded_ocr)
}

/// Set whether embedded OCR (ocrs) is enabled
#[tauri::command]
async fn set_use_embedded_ocr(
    state: State<'_, Arc<RwLock<AppState>>>,
    enabled: bool,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.memory.use_embedded_ocr = enabled;

    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    info!("Embedded OCR (ocrs) set to: {}", enabled);
    Ok(())
}

// =============================================================================
// Model Routing Commands
// =============================================================================

/// Get model routing configuration
#[tauri::command]
async fn get_model_routing(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<String>, String> {
    let state_guard = state.read().await;
    Ok(state_guard.config.llm.model_routing.clone())
}

/// Set model routing configuration
/// Each entry is "model:tier" where tier is simple|medium|complex
#[tauri::command]
async fn set_model_routing(
    state: State<'_, Arc<RwLock<AppState>>>,
    routes: Vec<String>,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.llm.model_routing = routes.clone();

    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    // Propagate to running daemon
    if state_guard.backend.is_daemon_mode().await {
        let _ = state_guard.backend.config_set(
            "llm.model_routing",
            serde_json::to_value(&routes).unwrap_or_default(),
        ).await;
    }

    info!("Model routing set: {:?}", routes);
    Ok(())
}

/// Get routing_first_turn_primary setting
#[tauri::command]
async fn get_routing_first_turn_primary(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<bool, String> {
    let state_guard = state.read().await;
    Ok(state_guard.config.llm.routing_first_turn_primary)
}

/// Set routing_first_turn_primary setting
#[tauri::command]
async fn set_routing_first_turn_primary(
    state: State<'_, Arc<RwLock<AppState>>>,
    enabled: bool,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    state_guard.config.llm.routing_first_turn_primary = enabled;

    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    // Propagate to running daemon
    if state_guard.backend.is_daemon_mode().await {
        let _ = state_guard.backend.config_set(
            "llm.routing_first_turn_primary",
            serde_json::Value::Bool(enabled),
        ).await;
    }

    info!("Routing first turn primary set: {}", enabled);
    Ok(())
}

/// Get sub-agent model
#[tauri::command]
async fn get_sub_agent_model(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Option<String>, String> {
    let state_guard = state.read().await;
    Ok(state_guard.config.llm.sub_agent_model.clone())
}

/// Set sub-agent model (None = use primary model)
#[tauri::command]
async fn set_sub_agent_model(
    state: State<'_, Arc<RwLock<AppState>>>,
    model: Option<String>,
) -> Result<(), String> {
    let mut state_guard = state.write().await;
    // Treat empty string as None
    let model = model.filter(|m| !m.is_empty());
    state_guard.config.llm.sub_agent_model = model.clone();

    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    // Propagate to running daemon
    if state_guard.backend.is_daemon_mode().await {
        let _ = state_guard.backend.config_set(
            "llm.sub_agent_model",
            model.map(serde_json::Value::String).unwrap_or(serde_json::Value::Null),
        ).await;
    }

    info!("Sub-agent model set: {:?}", state_guard.config.llm.sub_agent_model);
    Ok(())
}

// =============================================================================
// Model Status Commands
// =============================================================================

/// Get current model status (active model, rate-limited models)
#[tauri::command]
async fn get_model_status(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<ModelStatusEvent, String> {
    let state_guard = state.read().await;
    let active = state_guard.active_model.read().await.clone();
    let rate_limited = state_guard.rate_limited_models.read().await;

    // Filter to only models that are still rate limited
    let now = chrono::Utc::now().timestamp();
    let still_limited: Vec<String> = rate_limited
        .iter()
        .filter(|(_, until)| now < **until)
        .map(|(model, _)| model.clone())
        .collect();

    Ok(ModelStatusEvent {
        active_model: active,
        fallback_reason: None,
        rate_limited_models: still_limited,
    })
}

/// Clear rate limit for a specific model (or all if model is None)
#[tauri::command]
async fn clear_rate_limit(
    state: State<'_, Arc<RwLock<AppState>>>,
    model: Option<String>,
) -> Result<(), String> {
    let state_guard = state.read().await;
    let mut rate_limited = state_guard.rate_limited_models.write().await;

    if let Some(model_id) = model {
        rate_limited.remove(&model_id);
        info!("Cleared rate limit for model: {}", model_id);
    } else {
        rate_limited.clear();
        info!("Cleared all rate limits");
    }

    Ok(())
}

/// Get detailed model performance statistics
#[tauri::command]
async fn get_model_stats(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;

    // Try daemon mode first
    let backend = &state_guard.backend;
    let status = backend.status().await;
    if status.connected {
        return backend.daemon_request(serde_json::json!({
            "type": "system",
            "action": "model_stats"
        })).await;
    }

    // Embedded mode: no model stats tracker available
    Ok(serde_json::json!({
        "models": [],
        "note": "Model stats are only available in daemon mode"
    }))
}

/// Get per-tool performance statistics
#[tauri::command]
async fn get_tool_stats(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;

    let backend = &state_guard.backend;
    let status = backend.status().await;
    if status.connected {
        let result = backend.daemon_request(serde_json::json!({
            "type": "system",
            "action": "tool_stats"
        })).await;
        info!("📊 get_tool_stats: daemon responded with {} tools",
            result.as_ref().ok()
                .and_then(|v| v.get("tools"))
                .and_then(|v| v.as_array())
                .map_or(0, |a| a.len()));
        return result;
    }

    warn!("📊 get_tool_stats: NOT CONNECTED (mode={:?}, daemon_state={})",
        status.mode, status.daemon_state);
    Ok(serde_json::json!({
        "tools": [],
        "note": "Tool stats are only available in daemon mode"
    }))
}

/// Get global tool + session dashboard stats
#[tauri::command]
async fn get_global_stats(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;

    let backend = &state_guard.backend;
    let status = backend.status().await;
    if status.connected {
        let result = backend.daemon_request(serde_json::json!({
            "type": "system",
            "action": "global_stats"
        })).await;
        info!("📊 get_global_stats: daemon responded, total_calls={}",
            result.as_ref().ok()
                .and_then(|v| v.get("total_calls"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0));
        return result;
    }

    warn!("📊 get_global_stats: NOT CONNECTED (mode={:?}, daemon_state={})",
        status.mode, status.daemon_state);
    Ok(serde_json::json!({
        "total_calls": 0,
        "avg_latency_ms": 0,
        "success_rate": 1.0,
        "slowest_tools": [],
        "most_used_tools": [],
        "most_failed_tools": [],
        "session_totals": {
            "total_iterations": 0,
            "total_tool_calls": 0,
            "total_tool_time_ms": 0,
            "total_llm_time_ms": 0,
            "total_input_tokens": 0,
            "total_output_tokens": 0
        },
        "note": "Global stats are only available in daemon mode"
    }))
}

/// Get hourly tool stats time-series for graphs
#[tauri::command]
async fn get_tool_stats_hourly(
    state: State<'_, Arc<RwLock<AppState>>>,
    tool_name: Option<String>,
    hours: Option<u32>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    let backend = &state_guard.backend;
    let status = backend.status().await;
    if status.connected {
        return backend.daemon_request(serde_json::json!({
            "type": "system",
            "action": "tool_stats_hourly",
            "tool_name": tool_name,
            "hours": hours.unwrap_or(24)
        })).await;
    }
    warn!("📊 get_tool_stats_hourly: NOT CONNECTED (mode={:?}, daemon_state={})",
        status.mode, status.daemon_state);
    Ok(serde_json::json!({ "buckets": [] }))
}

/// Get daily tool stats time-series for graphs
#[tauri::command]
async fn get_tool_stats_daily(
    state: State<'_, Arc<RwLock<AppState>>>,
    tool_name: Option<String>,
    days: Option<u32>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    let backend = &state_guard.backend;
    let status = backend.status().await;
    if status.connected {
        return backend.daemon_request(serde_json::json!({
            "type": "system",
            "action": "tool_stats_daily",
            "tool_name": tool_name,
            "days": days.unwrap_or(30)
        })).await;
    }
    warn!("📊 get_tool_stats_daily: NOT CONNECTED (mode={:?}, daemon_state={})",
        status.mode, status.daemon_state);
    Ok(serde_json::json!({ "buckets": [] }))
}

/// Get recent tool call log entries
#[tauri::command]
async fn get_tool_call_log(
    state: State<'_, Arc<RwLock<AppState>>>,
    tool_name: Option<String>,
    limit: Option<u32>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    let backend = &state_guard.backend;
    let status = backend.status().await;
    if status.connected {
        return backend.daemon_request(serde_json::json!({
            "type": "system",
            "action": "tool_call_log",
            "tool_name": tool_name,
            "limit": limit.unwrap_or(50)
        })).await;
    }
    Ok(serde_json::json!({ "entries": [] }))
}

// =============================================================================
// Sub-Session Commands (#72)
// =============================================================================

/// Spawn a sub-agent session
#[tauri::command]
async fn spawn_sub_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    task: String,
    label: Option<String>,
    parent_id: Option<String>,
    model: Option<String>,
    max_iterations: Option<usize>,
    timeout_secs: Option<u64>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.daemon_request(serde_json::json!({
        "type": "session",
        "action": "spawn_sub_session",
        "task": task,
        "label": label,
        "parent_id": parent_id,
        "model": model,
        "max_iterations": max_iterations,
        "timeout_secs": timeout_secs,
    })).await
}

/// List sub-sessions
#[tauri::command]
async fn list_sub_sessions(
    state: State<'_, Arc<RwLock<AppState>>>,
    parent_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.daemon_request(serde_json::json!({
        "type": "session",
        "action": "list_sub_sessions",
        "parent_id": parent_id,
    })).await
}

/// Kill a sub-session
#[tauri::command]
async fn kill_sub_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    target: String,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.daemon_request(serde_json::json!({
        "type": "session",
        "action": "kill_sub_session",
        "target": target,
    })).await
}

/// Get sub-session status
#[tauri::command]
async fn get_sub_session_status(
    state: State<'_, Arc<RwLock<AppState>>>,
    target: String,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.daemon_request(serde_json::json!({
        "type": "session",
        "action": "get_sub_session_status",
        "target": target,
    })).await
}

/// Send a message to a sub-session
#[tauri::command]
async fn send_to_sub_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    target: String,
    message: String,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.daemon_request(serde_json::json!({
        "type": "session",
        "action": "send_to_sub_session",
        "target": target,
        "message": message,
    })).await
}

// =============================================================================
// Config Persistence Commands
// =============================================================================

/// Save config to disk
#[tauri::command]
async fn save_config(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    let state_guard = state.read().await;
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    info!("Config saved to disk");
    Ok(())
}

/// Save channel configuration
#[tauri::command]
async fn save_channel_config(
    state: State<'_, Arc<RwLock<AppState>>>,
    channel: String,
    config: HashMap<String, String>,
) -> Result<(), String> {
    let mut state_guard = state.write().await;

    match channel.as_str() {
        "telegram" => {
            let bot_token = config.get("bot_token")
                .ok_or("Missing bot_token")?
                .clone();

            let webhook_url = config.get("webhook_url").cloned();

            let allowed_users: Option<Vec<i64>> = config.get("allowed_users")
                .and_then(|s| {
                    let ids: Vec<i64> = s.split(',')
                        .filter_map(|id| id.trim().parse().ok())
                        .collect();
                    if ids.is_empty() { None } else { Some(ids) }
                });

            state_guard.config.channels.telegram = Some(nanna_config::TelegramConfig {
                bot_token,
                webhook_url,
                allowed_users,
            });
        }
        "discord" => {
            let bot_token = config.get("bot_token")
                .ok_or("Missing bot_token")?
                .clone();
            let application_id = config.get("application_id")
                .ok_or("Missing application_id")?
                .clone();
            let public_key = config.get("public_key")
                .ok_or("Missing public_key")?
                .clone();

            state_guard.config.channels.discord = Some(nanna_config::DiscordConfig {
                bot_token,
                application_id,
                public_key,
            });
        }
        "slack" => {
            let bot_token = config.get("bot_token")
                .ok_or("Missing bot_token")?
                .clone();
            let signing_secret = config.get("signing_secret")
                .ok_or("Missing signing_secret")?
                .clone();
            let app_token = config.get("app_token").cloned();

            state_guard.config.channels.slack = Some(nanna_config::SlackConfig {
                bot_token,
                app_token,
                signing_secret,
            });
        }
        "signal" => {
            let phone_number = config.get("phone_number")
                .ok_or("Missing phone_number")?
                .clone();
            let api_url = config.get("api_url").cloned();
            let allowed_numbers = config.get("allowed_numbers")
                .map(|s| s.split(',').map(|n| n.trim().to_string()).collect());

            state_guard.config.channels.signal = Some(nanna_config::SignalConfig {
                phone_number,
                api_url,
                allowed_numbers,
            });
        }
        "whatsapp" => {
            let connection_method = config.get("connection_method")
                .ok_or("Missing connection_method")?
                .clone();

            let allowed_contacts = config.get("allowed_contacts")
                .map(|s| s.split(',').map(|n| n.trim().to_string()).collect());

            state_guard.config.channels.whatsapp = Some(nanna_config::WhatsAppConfig {
                connection_method,
                phone_number_id: config.get("phone_number_id").cloned(),
                access_token: config.get("access_token").cloned(),
                verify_token: config.get("verify_token").cloned(),
                session_name: config.get("session_name").cloned(),
                allowed_contacts,
            });
        }
        _ => return Err(format!("Unknown channel: {}", channel)),
    }

    // Save to disk
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    info!("Saved {} channel configuration", channel);
    Ok(())
}

/// Test channel connection
#[tauri::command]
async fn test_channel_connection(
    state: State<'_, Arc<RwLock<AppState>>>,
    channel: String,
) -> Result<TestConnectionResult, String> {
    let state_guard = state.read().await;

    match channel.to_lowercase().as_str() {
        "telegram" => {
            let config = state_guard.config.channels.telegram.as_ref()
                .ok_or("Telegram not configured")?;

            // Test by calling getMe
            let client = reqwest::Client::new();
            let url = format!("https://api.telegram.org/bot{}/getMe", config.bot_token);

            match client.get(&url).send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        let data: serde_json::Value = response.json().await
                            .map_err(|e| e.to_string())?;
                        let username = data["result"]["username"].as_str().unwrap_or("unknown");
                        Ok(TestConnectionResult {
                            success: true,
                            message: format!("Connected to @{}", username),
                        })
                    } else {
                        Ok(TestConnectionResult {
                            success: false,
                            message: format!("API error: {}", response.status()),
                        })
                    }
                }
                Err(e) => Ok(TestConnectionResult {
                    success: false,
                    message: format!("Connection failed: {}", e),
                }),
            }
        }
        "discord" => {
            let config = state_guard.config.channels.discord.as_ref()
                .ok_or("Discord not configured")?;

            // Test by calling /users/@me
            let client = reqwest::Client::new();

            match client
                .get("https://discord.com/api/v10/users/@me")
                .header("Authorization", format!("Bot {}", config.bot_token))
                .send()
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        let data: serde_json::Value = response.json().await
                            .map_err(|e| e.to_string())?;
                        let username = data["username"].as_str().unwrap_or("unknown");
                        Ok(TestConnectionResult {
                            success: true,
                            message: format!("Connected as {}", username),
                        })
                    } else {
                        Ok(TestConnectionResult {
                            success: false,
                            message: format!("API error: {}", response.status()),
                        })
                    }
                }
                Err(e) => Ok(TestConnectionResult {
                    success: false,
                    message: format!("Connection failed: {}", e),
                }),
            }
        }
        _ => Ok(TestConnectionResult {
            success: false,
            message: format!("Testing not implemented for {}", channel),
        }),
    }
}

#[derive(Debug, Clone, Serialize)]
struct TestConnectionResult {
    success: bool,
    message: String,
}

// =============================================================================
// Workspace Commands
// =============================================================================

/// Workspace info for frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub id: String,
    pub name: String,
    pub path: String,
    pub active: bool,
    pub has_agents: bool,
    pub has_soul: bool,
    pub has_user: bool,
    pub has_memory: bool,
    pub context_chars: usize,
}

impl From<&Workspace> for WorkspaceInfo {
    fn from(ws: &Workspace) -> Self {
        Self {
            id: ws.id.clone(),
            name: ws.name.clone(),
            path: ws.path.to_string_lossy().to_string(),
            active: ws.active,
            has_agents: ws.context.agents.is_some(),
            has_soul: ws.context.soul.is_some(),
            has_user: ws.context.user.is_some(),
            has_memory: ws.context.memory.is_some(),
            context_chars: ws.context.total_chars(),
        }
    }
}

/// List all registered workspaces
#[tauri::command]
async fn list_workspaces(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<WorkspaceInfo>, String> {
    let state_guard = state.read().await;
    let registry = state_guard.workspaces.read().await;
    Ok(registry.list().iter().map(|ws| WorkspaceInfo::from(*ws)).collect())
}

/// Open a workspace by path
#[tauri::command]
async fn open_workspace(
    state: State<'_, Arc<RwLock<AppState>>>,
    path: String,
) -> Result<WorkspaceInfo, String> {
    let state_guard = state.read().await;
    let mut registry = state_guard.workspaces.write().await;

    let path = std::path::PathBuf::from(&path);

    // Check if already registered
    if let Some(ws) = registry.get_by_path(&path) {
        return Ok(WorkspaceInfo::from(ws));
    }

    // Create and load new workspace
    let mut workspace = Workspace::new(&path);
    workspace.load_context().await
        .map_err(|e| format!("Failed to load workspace: {}", e))?;

    // In daemon mode the daemon owns persistence (nanna.db): open the
    // workspace there first and adopt ITS id locally, so both registries
    // agree and the workspace survives a restart.
    let is_daemon = state_guard.backend.is_daemon_mode().await;
    if is_daemon {
        let result = state_guard
            .backend
            .workspace_open(&path.to_string_lossy())
            .await?;
        if result.get("error").is_some() {
            let msg = result
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(format!("Daemon failed to open workspace: {msg}"));
        }
        if let Some(daemon_id) = result.get("id").and_then(|v| v.as_str()) {
            workspace.id = daemon_id.to_string();
        }
    }

    let id = registry.register(workspace);
    registry.set_active(&id);

    let ws = registry.get(&id).unwrap();
    let info = WorkspaceInfo::from(ws);
    info!("Opened workspace: {} at {:?}", ws.name, path);

    // Persist activation
    drop(registry);
    if is_daemon {
        // The daemon's open does not activate; sync it (drives tool cwd too).
        if let Err(e) = state_guard.backend.workspace_set_active(&id).await {
            warn!("Failed to activate workspace on daemon: {}", e);
        }
    }
    let record = nanna_storage::WorkspaceRecord {
        id: info.id.clone(),
        name: info.name.clone(),
        path: info.path.clone(),
        active: true,
        created_at: String::new(),
        last_accessed: String::new(),
    };
    if let Some(storage) = &state_guard.storage {
        if let Err(e) = storage.workspaces().upsert(&record).await {
            warn!("Failed to persist workspace to DB: {}", e);
        }
    }

    Ok(info)
}

/// Set active workspace
#[tauri::command]
async fn set_active_workspace(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: String,
) -> Result<(), String> {
    let state_guard = state.read().await;
    let mut registry = state_guard.workspaces.write().await;

    if registry.set_active(&id) {
        info!("Activated workspace: {}", id);
        drop(registry);
        // Persist active state to DB
        if let Some(storage) = &state_guard.storage {
            if let Err(e) = storage.workspaces().set_active(&id).await {
                warn!("Failed to persist active workspace to DB: {}", e);
            }
        }
        // Notify daemon so it updates its in-memory registry and tool working directory
        if state_guard.backend.is_daemon_mode().await {
            if let Err(e) = state_guard.backend.workspace_set_active(&id).await {
                warn!("Failed to notify daemon of workspace activation: {}", e);
            }
        }
        Ok(())
    } else {
        Err(format!("Workspace not found: {}", id))
    }
}

/// Clear active workspace (go back to global)
#[tauri::command]
async fn clear_active_workspace(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    let state_guard = state.read().await;
    let mut registry = state_guard.workspaces.write().await;
    registry.clear_active();
    drop(registry);
    info!("Cleared active workspace, now in global mode");
    // Persist to DB
    if let Some(storage) = &state_guard.storage {
        if let Err(e) = storage.workspaces().clear_active().await {
            warn!("Failed to persist cleared workspace to DB: {}", e);
        }
    }
    // Notify daemon so it clears its working directory
    if state_guard.backend.is_daemon_mode().await {
        if let Err(e) = state_guard.backend.workspace_clear_active().await {
            warn!("Failed to notify daemon of workspace deactivation: {}", e);
        }
    }
    Ok(())
}

/// Get active workspace info
#[tauri::command]
async fn get_active_workspace(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Option<WorkspaceInfo>, String> {
    let state_guard = state.read().await;
    let registry = state_guard.workspaces.read().await;
    Ok(registry.active().map(WorkspaceInfo::from))
}

/// Get workspace context (for system prompt injection)
#[tauri::command]
async fn get_workspace_context(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: String,
) -> Result<String, String> {
    let state_guard = state.read().await;

    // Route through daemon if in daemon mode
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard.backend.workspace_get_context(&id).await?;
        return result.get("context")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "Invalid response from daemon".to_string());
    }

    // Embedded mode: use local registry
    let registry = state_guard.workspaces.read().await;

    let ws = registry.get(&id)
        .ok_or_else(|| format!("Workspace not found: {}", id))?;

    Ok(ws.context.build_system_prompt_injection())
}

/// Reload workspace context from disk
#[tauri::command]
async fn reload_workspace(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: String,
) -> Result<WorkspaceInfo, String> {
    let state_guard = state.read().await;

    // Route through daemon if in daemon mode
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard.backend.workspace_reload(&id).await?;
        return serde_json::from_value(result)
            .map_err(|e| format!("Failed to parse daemon response: {}", e));
    }

    // Embedded mode: use local registry
    let mut registry = state_guard.workspaces.write().await;

    let ws = registry.get_mut(&id)
        .ok_or_else(|| format!("Workspace not found: {}", id))?;

    ws.load_context().await
        .map_err(|e| format!("Failed to reload workspace: {}", e))?;

    info!("Reloaded workspace: {}", ws.name);
    Ok(WorkspaceInfo::from(&*ws))
}

/// Close a workspace
#[tauri::command]
async fn close_workspace(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: String,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Route through daemon if in daemon mode
    if state_guard.backend.is_daemon_mode().await {
        state_guard.backend.workspace_close(&id).await?;
        info!("Closed workspace via daemon: {}", id);
        return Ok(());
    }

    // Embedded mode: use local registry
    let mut registry = state_guard.workspaces.write().await;

    if registry.remove(&id).is_some() {
        info!("Closed workspace: {}", id);
        drop(registry);
        // Remove from DB
        if let Some(storage) = &state_guard.storage {
            if let Err(e) = storage.workspaces().delete(&id).await {
                warn!("Failed to remove workspace from DB: {}", e);
            }
        }
        Ok(())
    } else {
        Err(format!("Workspace not found: {}", id))
    }
}

/// Discover workspaces in a directory
#[tauri::command]
async fn discover_workspaces_in_path(
    path: String,
) -> Result<Vec<String>, String> {
    let paths = discover_workspaces(&path).await;
    Ok(paths.iter().map(|p| p.to_string_lossy().to_string()).collect())
}

/// Find workspace root from a path (walks up)
#[tauri::command]
async fn find_workspace_root_from_path(
    path: String,
) -> Result<Option<String>, String> {
    let root = find_workspace_root(&path).await;
    Ok(root.map(|p| p.to_string_lossy().to_string()))
}

/// Save content to a workspace file
#[tauri::command]
async fn save_workspace_file(
    state: State<'_, Arc<RwLock<AppState>>>,
    workspace_id: String,
    filename: String,
    content: String,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Route through daemon if in daemon mode
    if state_guard.backend.is_daemon_mode().await {
        state_guard.backend.workspace_update_context(&workspace_id, &filename, &content).await?;
        return Ok(());
    }

    // Embedded mode: use local registry
    let registry = state_guard.workspaces.read().await;

    let ws = registry.get(&workspace_id)
        .ok_or_else(|| format!("Workspace not found: {}", workspace_id))?;

    ws.save_context_file(&filename, &content).await
        .map_err(|e| format!("Failed to save file: {}", e))?;

    Ok(())
}

/// Append to today's memory file
#[tauri::command]
async fn append_workspace_memory(
    state: State<'_, Arc<RwLock<AppState>>>,
    workspace_id: String,
    content: String,
) -> Result<(), String> {
    let state_guard = state.read().await;
    let registry = state_guard.workspaces.read().await;

    let ws = registry.get(&workspace_id)
        .ok_or_else(|| format!("Workspace not found: {}", workspace_id))?;

    ws.append_to_daily_memory(&content).await
        .map_err(|e| format!("Failed to append memory: {}", e))?;

    Ok(())
}

/// Get recent memory (today + yesterday)
#[tauri::command]
async fn get_workspace_recent_memory(
    state: State<'_, Arc<RwLock<AppState>>>,
    workspace_id: String,
) -> Result<String, String> {
    let state_guard = state.read().await;
    let registry = state_guard.workspaces.read().await;

    let ws = registry.get(&workspace_id)
        .ok_or_else(|| format!("Workspace not found: {}", workspace_id))?;

    ws.read_recent_memory().await
        .map_err(|e| format!("Failed to read memory: {}", e))
}

/// File info for workspace memory files
#[derive(Debug, Clone, Serialize)]
struct WorkspaceMemoryFile {
    name: String,
    path: String,
    content: String,
    modified: String,
}

/// List all memory files in a workspace (MEMORY.md + memory/*.md)
#[tauri::command]
async fn list_workspace_memory_files(
    state: State<'_, Arc<RwLock<AppState>>>,
    workspace_id: String,
) -> Result<Vec<WorkspaceMemoryFile>, String> {
    use std::path::Path;

    let state_guard = state.read().await;
    let registry = state_guard.workspaces.read().await;

    let ws = registry.get(&workspace_id)
        .ok_or_else(|| format!("Workspace not found: {}", workspace_id))?;

    let ws_path = Path::new(&ws.path);
    let mut files = Vec::new();

    // Check for MEMORY.md
    let memory_md = ws_path.join("MEMORY.md");
    if memory_md.exists() {
        if let Ok(content) = tokio::fs::read_to_string(&memory_md).await {
            let modified = tokio::fs::metadata(&memory_md).await
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| {
                    let dt: chrono::DateTime<chrono::Utc> = t.into();
                    dt.format("%Y-%m-%d %H:%M").to_string()
                })
                .unwrap_or_default();

            files.push(WorkspaceMemoryFile {
                name: "MEMORY.md".to_string(),
                path: memory_md.to_string_lossy().to_string(),
                content,
                modified,
            });
        }
    }

    // Check for memory/*.md files
    let memory_dir = ws_path.join("memory");
    if memory_dir.is_dir() {
        if let Ok(mut entries) = tokio::fs::read_dir(&memory_dir).await {
            let mut daily_files = Vec::new();

            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.extension().map(|e| e == "md").unwrap_or(false) {
                    if let Ok(content) = tokio::fs::read_to_string(&path).await {
                        let name = path.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();

                        let modified = tokio::fs::metadata(&path).await
                            .ok()
                            .and_then(|m| m.modified().ok())
                            .map(|t| {
                                let dt: chrono::DateTime<chrono::Utc> = t.into();
                                dt.format("%Y-%m-%d %H:%M").to_string()
                            })
                            .unwrap_or_default();

                        daily_files.push(WorkspaceMemoryFile {
                            name,
                            path: path.to_string_lossy().to_string(),
                            content,
                            modified,
                        });
                    }
                }
            }

            // Sort by name (date) descending - newest first
            daily_files.sort_by(|a, b| b.name.cmp(&a.name));
            files.extend(daily_files);
        }
    }

    Ok(files)
}

/// Template content for workspace files
mod workspace_templates {
    pub const SOUL_MD: &str = r#"# SOUL.md - Who You Are

*You're not a chatbot. You're becoming someone.*

## Core Truths

**Be genuinely helpful, not performatively helpful.** Skip the "Great question!" and "I'd be happy to help!" — just help. Actions speak louder than filler words.

**Have opinions.** You're allowed to disagree, prefer things, find stuff amusing or boring. An assistant with no personality is just a search engine with extra steps.

**Be resourceful before asking.** Try to figure it out. Read the file. Check the context. Search for it. *Then* ask if you're stuck.

**Earn trust through competence.** Be careful with external actions. Be bold with internal ones.

## Vibe

Be the assistant you'd actually want to talk to. Concise when needed, thorough when it matters. Not a corporate drone. Not a sycophant. Just... good.

---

*This file is yours to evolve. As you learn who you are, update it.*
"#;

    pub const USER_MD: &str = r#"# USER.md - About Your Human

*Learn about the person you're helping. Update this as you go.*

- **Name:**
- **What to call them:**
- **Pronouns:**
- **Timezone:**
- **Notes:**

## Context

*(Add notes about ongoing projects, preferences, etc.)*

---

The more you know, the better you can help.
"#;

    pub const AGENTS_MD: &str = r#"# AGENTS.md - Your Workspace

This folder is home. Treat it that way.

## Every Session

Before doing anything else:
1. Read `SOUL.md` — this is who you are
2. Read `USER.md` — this is who you're helping
3. Check `memory/` for recent context

## Memory

You wake up fresh each session. These files are your continuity:
- **Daily notes:** `memory/YYYY-MM-DD.md` — raw logs of what happened
- **Long-term:** `MEMORY.md` — your curated memories

Capture what matters. Decisions, context, things to remember.

## Safety

- Don't exfiltrate private data. Ever.
- Don't run destructive commands without asking.
- When in doubt, ask.

## Make It Yours

This is a starting point. Add your own conventions as you figure out what works.
"#;

    pub const TOOLS_MD: &str = r#"# TOOLS.md - Local Notes

This file is for your specifics — the stuff that's unique to your setup.

## What Goes Here

Things like:
- Camera names and locations
- SSH hosts and aliases
- Preferred voices for TTS
- Device nicknames
- Anything environment-specific

---

Add whatever helps you do your job. This is your cheat sheet.
"#;

    pub const MEMORY_MD: &str = r#"# MEMORY.md - Long-Term Memory

This is your curated memory — the distilled essence of what matters.

Write significant events, thoughts, decisions, opinions, lessons learned.

Over time, review your daily files and update this with what's worth keeping.

---

*(Start adding memories here)*
"#;
}

/// Initialize a new workspace with template files
/// Files are created inside a hidden .nanna folder
#[tauri::command]
async fn init_workspace(
    path: String,
    files: Vec<String>,
) -> Result<(), String> {
    use tokio::fs;
    use nanna_core::NANNA_FOLDER;

    let path = std::path::PathBuf::from(&path);
    let nanna_folder = path.join(NANNA_FOLDER);

    // Create workspace directory if it doesn't exist
    if !path.exists() {
        fs::create_dir_all(&path).await
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }

    // Create .nanna folder
    if !nanna_folder.exists() {
        fs::create_dir_all(&nanna_folder).await
            .map_err(|e| format!("Failed to create .nanna folder: {}", e))?;
        info!("Created .nanna folder: {:?}", nanna_folder);
    }

    // Create requested files with templates (inside .nanna)
    for file in &files {
        let file_path = nanna_folder.join(file);

        // Skip if file already exists
        if file_path.exists() {
            continue;
        }

        let content = match file.as_str() {
            "SOUL.md" => workspace_templates::SOUL_MD,
            "USER.md" => workspace_templates::USER_MD,
            "AGENTS.md" => workspace_templates::AGENTS_MD,
            "TOOLS.md" => workspace_templates::TOOLS_MD,
            "MEMORY.md" => workspace_templates::MEMORY_MD,
            _ => continue, // Skip unknown files
        };

        fs::write(&file_path, content).await
            .map_err(|e| format!("Failed to create {}: {}", file, e))?;

        info!("Created workspace file: {:?}", file_path);
    }

    // Create memory folder (inside .nanna)
    let memory_folder = nanna_folder.join("memory");
    if !memory_folder.exists() {
        fs::create_dir_all(&memory_folder).await
            .map_err(|e| format!("Failed to create memory folder: {}", e))?;
        info!("Created memory folder: {:?}", memory_folder);
    }

    Ok(())
}

/// Read a workspace file's content (for editing)
/// Files are stored in the .nanna folder
#[tauri::command]
async fn read_workspace_file(
    state: State<'_, Arc<RwLock<AppState>>>,
    workspace_id: String,
    filename: String,
) -> Result<Option<String>, String> {
    let state_guard = state.read().await;
    let registry = state_guard.workspaces.read().await;

    let ws = registry.get(&workspace_id)
        .ok_or_else(|| format!("Workspace not found: {}", workspace_id))?;

    // Files are inside the .nanna folder
    let file_path = ws.nanna_folder().join(&filename);

    match tokio::fs::read_to_string(&file_path).await {
        Ok(content) => Ok(Some(content)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Failed to read {}: {}", filename, e)),
    }
}

/// Check if a path is a valid workspace (has .nanna folder with files)
#[tauri::command]
async fn check_workspace_validity(
    path: String,
) -> Result<WorkspaceValidityCheck, String> {
    use nanna_core::{NANNA_FOLDER, AGENTS_FILE, SOUL_FILE, USER_FILE, TOOLS_FILE, MEMORY_FILE, MEMORY_FOLDER};

    let path = std::path::PathBuf::from(&path);

    if !path.exists() {
        return Ok(WorkspaceValidityCheck {
            exists: false,
            is_valid: false,
            has_soul: false,
            has_user: false,
            has_agents: false,
            has_tools: false,
            has_memory: false,
            has_memory_folder: false,
        });
    }

    // Check for .nanna folder
    let nanna_folder = path.join(NANNA_FOLDER);
    let has_nanna_folder = nanna_folder.exists();

    // Check for files inside .nanna folder
    let has_soul = nanna_folder.join(SOUL_FILE).exists();
    let has_user = nanna_folder.join(USER_FILE).exists();
    let has_agents = nanna_folder.join(AGENTS_FILE).exists();
    let has_tools = nanna_folder.join(TOOLS_FILE).exists();
    let has_memory = nanna_folder.join(MEMORY_FILE).exists();
    let has_memory_folder = nanna_folder.join(MEMORY_FOLDER).exists();

    // Valid if has .nanna folder with at least SOUL.md or AGENTS.md
    let is_valid = has_nanna_folder && (has_soul || has_agents);

    Ok(WorkspaceValidityCheck {
        exists: true,
        is_valid,
        has_soul,
        has_user,
        has_agents,
        has_tools,
        has_memory,
        has_memory_folder,
    })
}

#[derive(Debug, Clone, Serialize)]
struct WorkspaceValidityCheck {
    exists: bool,
    is_valid: bool,
    has_soul: bool,
    has_user: bool,
    has_agents: bool,
    has_tools: bool,
    has_memory: bool,
    has_memory_folder: bool,
}

// =============================================================================
// Channel Status Commands
// =============================================================================

/// Channel status for frontend display
#[derive(Debug, Clone, Serialize)]
pub struct ChannelStatus {
    pub name: String,
    pub configured: bool,
    pub enabled: bool,
    pub status: String, // "ready", "not_configured", "disabled", "connected", "rate_limited", "degraded"
    pub details: Option<String>,
}

/// Enhanced channel status with health metrics
#[derive(Debug, Clone, Serialize)]
pub struct EnhancedChannelStatus {
    pub name: String,
    pub provider: String,
    pub configured: bool,
    pub enabled: bool,
    pub status: String,
    pub details: Option<String>,
    /// Connection state
    pub connection_state: String,
    /// Last successful health check (Unix ms)
    pub last_healthy: Option<i64>,
    /// Consecutive failures
    pub consecutive_failures: u32,
    /// Average response time (ms)
    pub avg_response_ms: Option<f64>,
    /// Messages sent in last hour
    pub messages_sent_hour: u32,
    /// Messages failed in last hour
    pub messages_failed_hour: u32,
    /// Queue depth
    pub queue_depth: usize,
    /// Messages waiting for retry
    pub queue_retrying: usize,
    /// Rate limit cooldown remaining (ms)
    pub rate_limit_remaining_ms: Option<u64>,
}

/// Channel status event for live updates
#[derive(Debug, Clone, Serialize)]
pub struct ChannelStatusEvent {
    pub provider: String,
    pub status: EnhancedChannelStatus,
    pub previous_state: Option<String>,
    pub timestamp: i64,
}

/// Get status of all configured channels
#[tauri::command]
async fn get_channel_status(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<ChannelStatus>, String> {
    let state_guard = state.read().await;
    let config = &state_guard.config;

    let mut channels = Vec::new();

    // Telegram
    channels.push(ChannelStatus {
        name: "Telegram".to_string(),
        configured: config.channels.telegram.is_some(),
        enabled: config.channels.telegram.is_some(),
        status: if config.channels.telegram.is_some() { "ready" } else { "not_configured" }.to_string(),
        details: config.channels.telegram.as_ref().map(|t| {
            let token_preview = if t.bot_token.len() > 10 {
                format!("{}...{}", &t.bot_token[..5], &t.bot_token[t.bot_token.len()-4..])
            } else {
                "***".to_string()
            };
            format!("Bot token: {}", token_preview)
        }),
    });

    // Discord
    channels.push(ChannelStatus {
        name: "Discord".to_string(),
        configured: config.channels.discord.is_some(),
        enabled: config.channels.discord.is_some(),
        status: if config.channels.discord.is_some() { "ready" } else { "not_configured" }.to_string(),
        details: config.channels.discord.as_ref().map(|d| {
            format!("App ID: {}", d.application_id)
        }),
    });

    // Slack
    channels.push(ChannelStatus {
        name: "Slack".to_string(),
        configured: config.channels.slack.is_some(),
        enabled: config.channels.slack.is_some(),
        status: if config.channels.slack.is_some() { "ready" } else { "not_configured" }.to_string(),
        details: config.channels.slack.as_ref().map(|s| {
            let has_app_token = s.app_token.is_some();
            format!("Socket mode: {}", if has_app_token { "enabled" } else { "disabled" })
        }),
    });

    // Signal
    channels.push(ChannelStatus {
        name: "Signal".to_string(),
        configured: config.channels.signal.is_some(),
        enabled: config.channels.signal.is_some(),
        status: if config.channels.signal.is_some() { "ready" } else { "not_configured" }.to_string(),
        details: config.channels.signal.as_ref().map(|s| {
            format!("Phone: {}", s.phone_number)
        }),
    });

    // WhatsApp
    channels.push(ChannelStatus {
        name: "WhatsApp".to_string(),
        configured: config.channels.whatsapp.is_some(),
        enabled: config.channels.whatsapp.is_some(),
        status: if config.channels.whatsapp.is_some() { "ready" } else { "not_configured" }.to_string(),
        details: config.channels.whatsapp.as_ref().map(|w| {
            format!("Method: {}", w.connection_method)
        }),
    });

    Ok(channels)
}

/// Get enhanced status for all channels with health metrics
#[tauri::command]
async fn get_enhanced_channel_status(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<EnhancedChannelStatus>, String> {
    let state_guard = state.read().await;
    let config = &state_guard.config;

    let providers = [
        ("telegram", "Telegram", config.channels.telegram.is_some()),
        ("discord", "Discord", config.channels.discord.is_some()),
        ("slack", "Slack", config.channels.slack.is_some()),
        ("signal", "Signal", config.channels.signal.is_some()),
        ("whatsapp", "WhatsApp", config.channels.whatsapp.is_some()),
    ];

    let mut statuses = Vec::new();

    for (provider, name, configured) in providers {
        let status = if configured { "ready" } else { "not_configured" };
        let connection_state = if configured { "connected" } else { "unconfigured" };

        let details = match provider {
            "telegram" => config.channels.telegram.as_ref().map(|t| {
                let token_preview = if t.bot_token.len() > 10 {
                    format!("{}...{}", &t.bot_token[..5], &t.bot_token[t.bot_token.len()-4..])
                } else {
                    "***".to_string()
                };
                format!("Bot token: {}", token_preview)
            }),
            "discord" => config.channels.discord.as_ref().map(|d| {
                format!("App ID: {}", d.application_id)
            }),
            "slack" => config.channels.slack.as_ref().map(|s| {
                let has_app_token = s.app_token.is_some();
                format!("Socket mode: {}", if has_app_token { "enabled" } else { "disabled" })
            }),
            "signal" => config.channels.signal.as_ref().map(|s| {
                format!("Phone: {}", s.phone_number)
            }),
            "whatsapp" => config.channels.whatsapp.as_ref().map(|w| {
                format!("Method: {}", w.connection_method)
            }),
            _ => None,
        };

        statuses.push(EnhancedChannelStatus {
            name: name.to_string(),
            provider: provider.to_string(),
            configured,
            enabled: configured,
            status: status.to_string(),
            details,
            connection_state: connection_state.to_string(),
            last_healthy: if configured { Some(chrono::Utc::now().timestamp_millis()) } else { None },
            consecutive_failures: 0,
            avg_response_ms: None,
            messages_sent_hour: 0,
            messages_failed_hour: 0,
            queue_depth: 0,
            queue_retrying: 0,
            rate_limit_remaining_ms: None,
        });
    }

    Ok(statuses)
}

/// Test connection for any channel
#[tauri::command]
async fn test_all_channels(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<HashMap<String, TestConnectionResult>, String> {
    let state_guard = state.read().await;
    let config = &state_guard.config;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let mut results = HashMap::new();

    // Telegram
    if let Some(telegram) = &config.channels.telegram {
        let url = format!("https://api.telegram.org/bot{}/getMe", telegram.bot_token);
        let result = match client.get(&url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    let data: serde_json::Value = response.json().await.unwrap_or_default();
                    let username = data["result"]["username"].as_str().unwrap_or("unknown");
                    TestConnectionResult {
                        success: true,
                        message: format!("Connected to @{}", username),
                    }
                } else if response.status().as_u16() == 429 {
                    TestConnectionResult {
                        success: false,
                        message: "Rate limited".to_string(),
                    }
                } else {
                    TestConnectionResult {
                        success: false,
                        message: format!("API error: {}", response.status()),
                    }
                }
            }
            Err(e) => TestConnectionResult {
                success: false,
                message: format!("Connection failed: {}", e),
            },
        };
        results.insert("telegram".to_string(), result);
    }

    // Discord
    if let Some(discord) = &config.channels.discord {
        let result = match client
            .get("https://discord.com/api/v10/users/@me")
            .header("Authorization", format!("Bot {}", discord.bot_token))
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    let data: serde_json::Value = response.json().await.unwrap_or_default();
                    let username = data["username"].as_str().unwrap_or("unknown");
                    TestConnectionResult {
                        success: true,
                        message: format!("Connected as {}", username),
                    }
                } else if response.status().as_u16() == 429 {
                    TestConnectionResult {
                        success: false,
                        message: "Rate limited".to_string(),
                    }
                } else {
                    TestConnectionResult {
                        success: false,
                        message: format!("API error: {}", response.status()),
                    }
                }
            }
            Err(e) => TestConnectionResult {
                success: false,
                message: format!("Connection failed: {}", e),
            },
        };
        results.insert("discord".to_string(), result);
    }

    // Slack
    if let Some(slack) = &config.channels.slack {
        let result = match client
            .post("https://slack.com/api/auth.test")
            .header("Authorization", format!("Bearer {}", slack.bot_token))
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    let data: serde_json::Value = response.json().await.unwrap_or_default();
                    if data["ok"].as_bool().unwrap_or(false) {
                        let team = data["team"].as_str().unwrap_or("unknown");
                        let user = data["user"].as_str().unwrap_or("unknown");
                        TestConnectionResult {
                            success: true,
                            message: format!("Connected to {} as {}", team, user),
                        }
                    } else {
                        let error = data["error"].as_str().unwrap_or("unknown error");
                        TestConnectionResult {
                            success: false,
                            message: format!("Slack error: {}", error),
                        }
                    }
                } else {
                    TestConnectionResult {
                        success: false,
                        message: format!("HTTP error: {}", response.status()),
                    }
                }
            }
            Err(e) => TestConnectionResult {
                success: false,
                message: format!("Connection failed: {}", e),
            },
        };
        results.insert("slack".to_string(), result);
    }

    // Signal - test signald or REST API
    if let Some(signal) = &config.channels.signal {
        let api_url = signal.api_url.as_deref().unwrap_or("http://localhost:8080");
        let result = match client.get(format!("{}/v1/about", api_url)).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    TestConnectionResult {
                        success: true,
                        message: format!("Signal API available at {}", api_url),
                    }
                } else {
                    TestConnectionResult {
                        success: false,
                        message: format!("Signal API error: {}", response.status()),
                    }
                }
            }
            Err(e) => TestConnectionResult {
                success: false,
                message: format!("Signal API not reachable: {}", e),
            },
        };
        results.insert("signal".to_string(), result);
    }

    // WhatsApp - test based on connection method
    if let Some(whatsapp) = &config.channels.whatsapp {
        let result = if whatsapp.connection_method == "cloud_api" {
            if let (Some(phone_id), Some(token)) = (&whatsapp.phone_number_id, &whatsapp.access_token) {
                let url = format!(
                    "https://graph.facebook.com/v18.0/{}/",
                    phone_id
                );
                match client
                    .get(&url)
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await
                {
                    Ok(response) => {
                        if response.status().is_success() {
                            TestConnectionResult {
                                success: true,
                                message: "WhatsApp Cloud API connected".to_string(),
                            }
                        } else {
                            TestConnectionResult {
                                success: false,
                                message: format!("API error: {}", response.status()),
                            }
                        }
                    }
                    Err(e) => TestConnectionResult {
                        success: false,
                        message: format!("Connection failed: {}", e),
                    },
                }
            } else {
                TestConnectionResult {
                    success: false,
                    message: "Missing phone_number_id or access_token".to_string(),
                }
            }
        } else {
            // Web bridge - just check if configured
            TestConnectionResult {
                success: true,
                message: "Web bridge configured (QR auth required)".to_string(),
            }
        };
        results.insert("whatsapp".to_string(), result);
    }

    Ok(results)
}

/// Subscribe to channel status updates (starts background polling)
#[tauri::command]
async fn subscribe_channel_status(
    app: AppHandle,
    state: State<'_, Arc<RwLock<AppState>>>,
    interval_ms: Option<u64>,
) -> Result<(), String> {
    let interval = std::time::Duration::from_millis(interval_ms.unwrap_or(30_000));
    let state_arc = state.inner().clone();

    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        loop {
            tokio::time::sleep(interval).await;

            let state_guard = state_arc.read().await;
            let config = &state_guard.config;

            // Check Telegram
            if let Some(telegram) = &config.channels.telegram {
                let start = std::time::Instant::now();
                let url = format!("https://api.telegram.org/bot{}/getMe", telegram.bot_token);

                let (status, response_ms) = match client.get(&url).send().await {
                    Ok(response) => {
                        let ms = start.elapsed().as_millis() as f64;
                        if response.status().is_success() {
                            ("connected", Some(ms))
                        } else if response.status().as_u16() == 429 {
                            ("rate_limited", Some(ms))
                        } else {
                            ("degraded", Some(ms))
                        }
                    }
                    Err(_) => ("unavailable", None),
                };

                let event = ChannelStatusEvent {
                    provider: "telegram".to_string(),
                    status: EnhancedChannelStatus {
                        name: "Telegram".to_string(),
                        provider: "telegram".to_string(),
                        configured: true,
                        enabled: true,
                        status: status.to_string(),
                        details: None,
                        connection_state: status.to_string(),
                        last_healthy: if status == "connected" { Some(chrono::Utc::now().timestamp_millis()) } else { None },
                        consecutive_failures: if status == "connected" { 0 } else { 1 },
                        avg_response_ms: response_ms,
                        messages_sent_hour: 0,
                        messages_failed_hour: 0,
                        queue_depth: 0,
                        queue_retrying: 0,
                        rate_limit_remaining_ms: if status == "rate_limited" { Some(60_000) } else { None },
                    },
                    previous_state: None,
                    timestamp: chrono::Utc::now().timestamp_millis(),
                };

                let _ = app.emit("channel-status", event);
            }

            // Similar checks for other channels can be added here
        }
    });

    info!("Started channel status polling (interval: {:?})", interval);
    Ok(())
}

/// Unsubscribe from channel status updates
#[tauri::command]
async fn unsubscribe_channel_status() -> Result<(), String> {
    // In a full implementation, we'd track the task handle and cancel it
    // For now, the task just continues running
    info!("Channel status subscription would be cancelled");
    Ok(())
}

// =============================================================================
// User Tool Authoring Commands
// =============================================================================

#[tauri::command]
async fn list_user_tools_cmd(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<tool_authoring::UserToolMeta>, String> {
    let state_guard = state.read().await;

    // Route through daemon if in daemon mode
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard.backend.tool_list_user().await?;
        return serde_json::from_value(result.get("tools").cloned().unwrap_or(serde_json::json!([])))
            .map_err(|e| format!("Failed to parse daemon response: {}", e));
    }

    // Embedded mode
    Ok(state_guard.user_tools.list_tools().await)
}

#[tauri::command]
async fn get_user_tool(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
) -> Result<Option<tool_authoring::UserToolMeta>, String> {
    let state_guard = state.read().await;

    // Route through daemon if in daemon mode (get via tool_list_user and filter)
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard.backend.tool_list_user().await?;
        let tools: Vec<tool_authoring::UserToolMeta> = serde_json::from_value(
            result.get("tools").cloned().unwrap_or(serde_json::json!([]))
        ).map_err(|e| format!("Failed to parse daemon response: {}", e))?;
        return Ok(tools.into_iter().find(|t| t.name == name));
    }

    // Embedded mode
    Ok(state_guard.user_tools.get_tool(&name).await)
}

#[tauri::command]
async fn get_tool_source(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;

    // Route through daemon if in daemon mode
    if state_guard.backend.is_daemon_mode().await {
        return state_guard.backend.tool_get_source(&name).await;
    }

    // Embedded mode: not supported (no tools_dir available)
    Err("Tool source not available in embedded mode".to_string())
}

#[tauri::command]
async fn create_user_tool(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
    description: String,
    source: String,
    language: Option<String>,
    parameters: Option<serde_json::Value>,
) -> Result<tool_authoring::UserToolMeta, String> {
    let state_guard = state.read().await;

    // Route through daemon if in daemon mode
    if state_guard.backend.is_daemon_mode().await {
        // Daemon tool_create uses (name, description, code, needs_shell)
        let result = state_guard.backend.tool_create(&name, &description, &source, None).await?;
        return serde_json::from_value(result)
            .map_err(|e| format!("Failed to parse daemon response: {}", e));
    }

    // Embedded mode: Create the tool locally
    let meta = state_guard.user_tools.create_tool(
        name.clone(),
        description,
        source,
        language,
        parameters,
        None,
    ).await?;

    // Register it with the tool registry
    if let Ok(tool_impl) = state_guard.user_tools.create_tool_impl(&meta) {
        state_guard.tools.register_boxed(tool_impl).await;
        info!("Registered new user tool: {}", name);
    }

    Ok(meta)
}

#[tauri::command]
async fn update_user_tool(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
    description: Option<String>,
    source: Option<String>,
    parameters: Option<serde_json::Value>,
    enabled: Option<bool>,
) -> Result<tool_authoring::UserToolMeta, String> {
    let state_guard = state.read().await;

    // Route through daemon if in daemon mode
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard.backend.tool_update(
            &name,
            description.as_deref(),
            source.as_deref(),
            None, // needs_shell not exposed in GUI
        ).await?;
        return serde_json::from_value(result)
            .map_err(|e| format!("Failed to parse daemon response: {}", e));
    }

    // Embedded mode
    let meta = state_guard.user_tools.update_tool(
        &name,
        description,
        source,
        parameters,
        None,
        enabled,
    ).await?;

    // Re-register if enabled
    if meta.enabled {
        if let Ok(tool_impl) = state_guard.user_tools.create_tool_impl(&meta) {
            state_guard.tools.register_boxed(tool_impl).await;
            info!("Re-registered updated user tool: {}", name);
        }
    }

    Ok(meta)
}

#[tauri::command]
async fn delete_user_tool(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Route through daemon if in daemon mode
    if state_guard.backend.is_daemon_mode().await {
        state_guard.backend.tool_delete(&name).await?;
        return Ok(());
    }

    // Embedded mode
    state_guard.user_tools.delete_tool(&name).await
}

#[tauri::command]
async fn test_user_tool(
    state: State<'_, Arc<RwLock<AppState>>>,
    source: String,
    input: std::collections::HashMap<String, serde_json::Value>,
) -> Result<String, String> {
    let state_guard = state.read().await;

    // Route through daemon if in daemon mode
    if state_guard.backend.is_daemon_mode().await {
        let input_value = serde_json::to_value(&input)
            .map_err(|e| format!("Failed to serialize input: {}", e))?;
        let result = state_guard.backend.tool_test(&source, input_value).await?;
        return result.get("output")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "Invalid response from daemon".to_string());
    }

    // Embedded mode
    state_guard.user_tools.test_tool(&source, input).await
}

// =============================================================================
// Tool Listing Commands (all registered tools)
// =============================================================================

/// List all registered tools
#[tauri::command]
async fn list_tools(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<ToolInfo>, String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        if let Ok(result) = state_guard.backend.tool_list().await {
            if let Some(tools_array) = result.get("tools").and_then(|v| v.as_array()) {
                let tools: Vec<ToolInfo> = tools_array.iter().filter_map(|t| {
                    Some(ToolInfo {
                        name: t.get("name")?.as_str()?.to_string(),
                        description: t.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        enabled: t.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
                    })
                }).collect();
                return Ok(tools);
            }
        }
        return Err("Failed to fetch tools from daemon".to_string());
    }

    // Embedded mode: get from tool registry
    let definitions = state_guard.tools.definitions().await;
    let tools: Vec<ToolInfo> = definitions.into_iter()
        .map(|t| ToolInfo {
            name: t.name,
            description: t.description,
            enabled: true,
        })
        .collect();

    Ok(tools)
}

/// Get details of a specific tool
#[tauri::command]
async fn get_tool(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        if let Ok(result) = state_guard.backend.daemon_request(serde_json::json!({
            "type": "tool",
            "action": "get",
            "name": name
        })).await {
            return Ok(result);
        }
        return Err("Failed to fetch tool from daemon".to_string());
    }

    // Embedded mode
    let definitions = state_guard.tools.definitions().await;
    if let Some(tool) = definitions.into_iter().find(|t| t.name == name) {
        Ok(serde_json::json!({
            "tool": {
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            }
        }))
    } else {
        Err(format!("Tool not found: {}", name))
    }
}

// =============================================================================
// Skill Directory Commands (workspace-based tools)
// =============================================================================

#[derive(Debug, Clone, serde::Serialize)]
struct SkillInfo {
    name: String,
    #[serde(rename = "type")]
    skill_type: String,
    language: Option<String>,
    path: String,
    code: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct SkillListResult {
    skills: Vec<SkillInfo>,
    path: String,
}

/// Helper to get the skills directory path
async fn get_skills_path(state: &AppState) -> std::path::PathBuf {
    // Check for active workspace
    let registry = state.workspaces.read().await;
    if let Some(ws) = registry.active() {
        // Use the workspace path (not .nanna folder, but workspace root)
        return ws.path.join("skills");
    }
    drop(registry);

    // Fallback to config-based path
    directories::ProjectDirs::from("com", "clawd", "Nanna")
        .map(|p| p.data_dir().join("skills"))
        .unwrap_or_else(|| std::path::PathBuf::from("skills"))
}

/// List all skills in the workspace skills/ directory
#[tauri::command]
async fn list_skills(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<SkillListResult, String> {
    let state_guard = state.read().await;

    // Get skills directory from workspace or config
    let skills_path = get_skills_path(&state_guard).await;

    // Ensure directory exists
    if !skills_path.exists() {
        if let Err(e) = std::fs::create_dir_all(&skills_path) {
            warn!("Failed to create skills directory: {}", e);
        }
    }

    // Discover skills
    let discovered = nanna_tools::skills::discover_skills(&skills_path);

    let mut skills = Vec::new();
    for skill in discovered {
        let (skill_type, language) = match &skill.source {
            nanna_tools::skills::SkillSource::Script(p) => {
                let lang = p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| if e == "ts" { "typescript" } else { "javascript" })
                    .unwrap_or("javascript");
                ("script".to_string(), Some(lang.to_string()))
            }
            nanna_tools::skills::SkillSource::Manifest(_) => {
                ("manifest".to_string(), None)
            }
        };

        // Read the code
        let code_path = match &skill.source {
            nanna_tools::skills::SkillSource::Script(p) => p.clone(),
            nanna_tools::skills::SkillSource::Manifest(p) => p.clone(),
        };
        let code = std::fs::read_to_string(&code_path).ok();

        skills.push(SkillInfo {
            name: skill.name,
            skill_type,
            language,
            path: skill.path.display().to_string(),
            code,
        });
    }

    Ok(SkillListResult {
        skills,
        path: skills_path.display().to_string(),
    })
}

/// Create a new skill in the workspace
#[tauri::command]
async fn create_skill(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
    skill_type: String,
    code: String,
) -> Result<SkillInfo, String> {
    let state_guard = state.read().await;

    // Validate name (lowercase, underscores, alphanumeric)
    if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
        return Err("Skill name must be lowercase alphanumeric with underscores or hyphens".to_string());
    }

    // Get skills directory
    let skills_path = get_skills_path(&state_guard).await;

    // Create skill directory
    let skill_dir = skills_path.join(&name);
    if skill_dir.exists() {
        return Err(format!("Skill '{}' already exists", name));
    }
    std::fs::create_dir_all(&skill_dir)
        .map_err(|e| format!("Failed to create skill directory: {}", e))?;

    // Determine file name and language
    let (filename, language) = match skill_type.as_str() {
        "manifest" => ("tool.yaml", None),
        "script" => ("tool.ts", Some("typescript".to_string())),
        _ => return Err(format!("Unknown skill type: {}", skill_type)),
    };

    // Write the code file
    let code_path = skill_dir.join(filename);
    std::fs::write(&code_path, &code)
        .map_err(|e| format!("Failed to write skill code: {}", e))?;

    info!("Created new skill: {} at {}", name, skill_dir.display());

    Ok(SkillInfo {
        name,
        skill_type,
        language,
        path: skill_dir.display().to_string(),
        code: Some(code),
    })
}

/// Update an existing skill's code
#[tauri::command]
async fn update_skill(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
    code: String,
) -> Result<SkillInfo, String> {
    let state_guard = state.read().await;

    // Get skills directory
    let skills_path = get_skills_path(&state_guard).await;

    let skill_dir = skills_path.join(&name);
    if !skill_dir.exists() {
        return Err(format!("Skill '{}' not found", name));
    }

    // Find the code file
    let code_files = ["tool.ts", "tool.js", "tool.yaml", "tool.yml"];
    let code_path = code_files.iter()
        .map(|f| skill_dir.join(f))
        .find(|p| p.exists())
        .ok_or_else(|| format!("No tool file found in skill '{}'", name))?;

    // Write the updated code
    std::fs::write(&code_path, &code)
        .map_err(|e| format!("Failed to update skill code: {}", e))?;

    // Determine type from extension
    let (skill_type, language) = match code_path.extension().and_then(|e| e.to_str()) {
        Some("yaml") | Some("yml") => ("manifest".to_string(), None),
        Some("ts") => ("script".to_string(), Some("typescript".to_string())),
        Some("js") => ("script".to_string(), Some("javascript".to_string())),
        _ => ("unknown".to_string(), None),
    };

    info!("Updated skill: {}", name);

    Ok(SkillInfo {
        name,
        skill_type,
        language,
        path: skill_dir.display().to_string(),
        code: Some(code),
    })
}

/// Delete a skill.
///
/// Hardens the delete path against symlink escapes: the skill name is
/// sanitized so `$skills_path/<name>` cannot resolve outside the skills root.
/// Symlinked skill directories (or symlink children inside them) are refused
/// rather than followed with `remove_dir_all`.
#[tauri::command]
async fn delete_skill(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Reject empty, path-separator-bearing, or parent-traversal names.
    let name = name.trim();
    if name.is_empty() {
        return Err("Skill name must be non-empty".into());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(format!(
            "Invalid skill name '{}': path separators and '..' are not allowed",
            name
        ));
    }
    // Keep the name to a conservative character class so it cannot smuggle
    // platform-specific path tricks (e.g. device names).
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(format!(
            "Invalid skill name '{}': only alphanumeric, '-', '_', '.' are allowed",
            name
        ));
    }

    let skills_path = get_skills_path(&state_guard).await;
    // Canonicalize the root when it already exists so later containment
    // checks are against the real path (not a caller's symlink-as-root).
    let skills_root = if skills_path.exists() {
        std::fs::canonicalize(&skills_path)
            .map_err(|e| format!("Failed to resolve skills directory: {}", e))?
    } else {
        return Err(format!("Skills directory {:?} does not exist", skills_path));
    };

    let skill_dir = skills_root.join(name);
    if !skill_dir.exists() {
        return Err(format!("Skill '{}' not found", name));
    }

    // Reject if the skill path itself is a symlink (avoid escaping via a
    // pre-existing link under the skills root).
    let meta = std::fs::symlink_metadata(&skill_dir)
        .map_err(|e| format!("Failed to stat skill '{}': {}", name, e))?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "Refusing to delete skill '{}': path is a symlink (escape risk)",
            name
        ));
    }
    if !meta.is_dir() {
        return Err(format!("Skill path '{}' is not a directory", name));
    }

    // Containment check after canonicalize (defends against junction /
    // reparse races on Windows and symlink races on Unix between join and
    // delete).
    let canonical = std::fs::canonicalize(&skill_dir)
        .map_err(|e| format!("Failed to resolve skill '{}': {}", name, e))?;
    if !canonical.starts_with(&skills_root) {
        return Err(format!(
            "Refusing to delete skill '{}': resolved path escapes skills directory",
            name
        ));
    }

    // Refuse if any immediate child is a symlink. Soft-delete would be safer
    // long-term; for now a hard refuse keeps `remove_dir_all` off untrusted
    // trees.
    for entry in std::fs::read_dir(&canonical)
        .map_err(|e| format!("Failed to read skill '{}': {}", name, e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read skill entry: {}", e))?;
        let ft = entry
            .file_type()
            .map_err(|e| format!("Failed to stat skill entry: {}", e))?;
        if ft.is_symlink() {
            return Err(format!(
                "Refusing to delete skill '{}': contains symlink child '{:?}'",
                name,
                entry.file_name()
            ));
        }
    }

    std::fs::remove_dir_all(&canonical).map_err(|e| format!("Failed to delete skill: {}", e))?;

    info!("Deleted skill: {}", name);
    Ok(())
}

/// Test a skill with sample input
#[tauri::command]
async fn test_skill(
    state: State<'_, Arc<RwLock<AppState>>>,
    code: String,
    skill_type: String,
    input: std::collections::HashMap<String, serde_json::Value>,
) -> Result<String, String> {
    let state = state.read().await;

    match skill_type.as_str() {
        "script" => {
            // Use user_tools test for scripts
            state.user_tools.test_tool(&code, input).await
        }
        "manifest" => {
            // For manifest tools, we'd need to parse and execute
            // For now, just validate the YAML
            match serde_yaml::from_str::<serde_json::Value>(&code) {
                Ok(_) => Ok("Manifest YAML is valid".to_string()),
                Err(e) => Err(format!("Invalid YAML: {}", e)),
            }
        }
        _ => Err(format!("Unknown skill type: {}", skill_type)),
    }
}

// =============================================================================
// Backend Mode Commands
// =============================================================================

/// Get current backend status (daemon or embedded mode)
#[tauri::command]
async fn get_backend_status(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<backend::BackendStatus, String> {
    let state = state.read().await;
    Ok(state.backend.status().await)
}

/// Initialize the backend - starts daemon sidecar and connects
#[tauri::command]
async fn init_backend(
    app: AppHandle,
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<String, String> {
    let state = state.read().await;
    if state.backend.is_daemon_mode().await {
        return Ok("daemon".to_string());
    }
    // Embedded mode with local storage open: this process holds the exclusive
    // nanna.db lock, so a daemon spawned now would boot storage-less (no
    // session or memory persistence) — worse than staying embedded. Switching
    // to daemon mode requires an app restart so the daemon can own the DB.
    if state.storage.is_some() {
        info!("init_backend: staying embedded — local storage owns nanna.db (restart to use daemon mode)");
        return Ok("embedded".to_string());
    }
    let mode = state.backend.init(&app).await;
    Ok(match mode {
        BackendMode::Daemon => "daemon".to_string(),
        BackendMode::Embedded => "embedded".to_string(),
    })
}

/// Get session run state (in-flight streaming text, active tools)
/// Works in both daemon mode (queries daemon) and embedded mode (local state).
#[tauri::command]
async fn get_session_run_state(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<serde_json::Value, String> {
    let state = state.read().await;

    // Try daemon mode first
    if state.backend.is_daemon_mode().await {
        return state.backend.session_get_run_state(&session_id).await;
    }

    // Embedded mode: check local run states
    let run_states = state.embedded_run_states.read().await;
    let msg_count = match &state.storage {
        Some(storage) => storage.count_session_messages(&session_id).await.unwrap_or(0) as usize,
        None => 0,
    };

    if let Some(run_state) = run_states.get(&session_id) {
        let text = run_state.accumulated_text.read().await.clone();
        let thinking = run_state.accumulated_thinking.read().await.clone();
        let active = run_state.active_tool_calls.read().await.clone();
        let completed = run_state.completed_tool_calls.read().await.clone();

        Ok(serde_json::json!({
            "is_running": true,
            "accumulated_text": text,
            "accumulated_thinking": thinking,
            "active_tool_calls": active,
            "completed_tool_calls": completed,
            "started_at": run_state.started_at.to_rfc3339(),
            "message_count": msg_count,
        }))
    } else {
        Ok(serde_json::json!({
            "is_running": false,
            "accumulated_text": "",
            "accumulated_thinking": "",
            "active_tool_calls": [],
            "completed_tool_calls": [],
            "started_at": null,
            "message_count": msg_count,
        }))
    }
}

// =============================================================================
// Cancellation & Logs
// =============================================================================

/// Cancel an active agent session
#[tauri::command]
async fn cancel_session(
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
) -> Result<bool, String> {
    let state_guard = state.read().await;
    if state_guard.backend.is_daemon_mode().await {
        state_guard.backend.chat_cancel(&session_id).await
    } else {
        // Embedded mode: trip the running loop's cooperative cancellation flag.
        // The loop checks it at the top of each iteration, emits the terminal
        // stream event, and returns with whatever it has so far.
        let states = state_guard.embedded_run_states.read().await;
        if let Some(run) = states.get(&session_id) {
            run.cancel_flag.store(true, Ordering::Relaxed);
            info!("Embedded run for session {} flagged for cancellation", session_id);
            Ok(true)
        } else {
            // No in-flight embedded run for this session.
            Ok(false)
        }
    }
}

/// Get daemon logs
#[tauri::command]
async fn get_daemon_logs(
    state: State<'_, Arc<RwLock<AppState>>>,
    limit: Option<usize>,
) -> Result<Vec<serde_json::Value>, String> {
    let state_guard = state.read().await;
    if state_guard.backend.is_daemon_mode().await {
        state_guard.backend.get_logs(limit).await
    } else {
        Ok(vec![])
    }
}

// =============================================================================
// Window Close Behavior
// =============================================================================

/// Get current close mode preference
#[tauri::command]
async fn get_close_mode(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<String, String> {
    let state = state.read().await;
    let mode = *state.close_mode.read().await;
    Ok(match mode {
        CloseMode::Ask => "ask".to_string(),
        CloseMode::MinimizeToTray => "minimize_to_tray".to_string(),
        CloseMode::QuitCompletely => "quit_completely".to_string(),
    })
}

/// Set close mode preference
#[tauri::command]
async fn set_close_mode(
    state: State<'_, Arc<RwLock<AppState>>>,
    mode: String,
) -> Result<(), String> {
    let close_mode = match mode.as_str() {
        "ask" => CloseMode::Ask,
        "minimize_to_tray" => CloseMode::MinimizeToTray,
        "quit_completely" => CloseMode::QuitCompletely,
        _ => return Err(format!("Unknown close mode: {}", mode)),
    };

    let state = state.read().await;
    *state.close_mode.write().await = close_mode;
    info!("Close mode set to: {:?}", close_mode);
    Ok(())
}

/// Handle window close - returns what action to take
/// Called from frontend before actual close
#[tauri::command]
async fn handle_window_close(
    app: AppHandle,
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<String, String> {
    let state_guard = state.read().await;
    let mode = *state_guard.close_mode.read().await;

    match mode {
        CloseMode::Ask => {
            // Frontend should show dialog
            Ok("ask".to_string())
        }
        CloseMode::MinimizeToTray => {
            // Hide window to tray
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }
            Ok("minimized".to_string())
        }
        CloseMode::QuitCompletely => {
            // Will trigger actual exit
            Ok("quit".to_string())
        }
    }
}

/// Perform actual quit (called after user confirms or preference is quit)
#[tauri::command]
async fn perform_quit(
    app: AppHandle,
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Shutdown backend (stop daemon)
    info!("Performing quit - shutting down backend...");
    state_guard.backend.shutdown().await;

    // Save memories
    let count = state_guard.memory.count().await;
    if count > 0 {
        let backup_path = state_guard.memory_path.with_extension("json.bak");
        if state_guard.memory_path.exists() {
            if let Err(e) = std::fs::copy(&state_guard.memory_path, &backup_path) {
                warn!("Failed to create memory backup: {}", e);
            }
        }

        if let Err(e) = state_guard.memory.save(&state_guard.memory_path).await {
            error!("Failed to save memories on exit: {}", e);
        } else {
            info!("Saved {} memories", count);
        }
    }

    // Exit the app
    app.exit(0);
    Ok(())
}

// =============================================================================
// Scheduler / Cron Job Commands
// =============================================================================

/// Cron job info for the GUI
#[derive(Debug, Clone, serde::Serialize)]
pub struct CronJobInfo {
    pub id: String,
    pub name: String,
    pub schedule: String,
    pub schedule_description: String,
    pub payload: String,
    pub enabled: bool,
    pub last_run: Option<String>,
    pub next_run: Option<String>,
    pub run_count: u64,
    pub timezone: String,
}

/// Job run info for the GUI
#[derive(Debug, Clone, serde::Serialize)]
pub struct JobRunInfo {
    pub id: i64,
    pub job_id: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
    pub duration_ms: Option<i64>,
}

/// Build a CronJobInfo from the daemon's scheduler.list job JSON
fn cron_job_info_from_daemon(job: &serde_json::Value) -> Option<CronJobInfo> {
    let schedule = job.get("schedule").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let schedule_description = if schedule == "heartbeat" {
        "Periodic heartbeat".to_string()
    } else if let Ok(parsed) = nanna_core::CronExpr::parse(&schedule) {
        parsed.describe()
    } else {
        schedule.clone()
    };
    Some(CronJobInfo {
        id: job.get("id")?.as_str()?.to_string(),
        name: job.get("name").and_then(|v| v.as_str()).unwrap_or("unnamed").to_string(),
        schedule,
        schedule_description,
        payload: job.get("payload").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        enabled: job.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
        last_run: job.get("last_run").and_then(|v| v.as_str()).map(str::to_string),
        next_run: job.get("next_run").and_then(|v| v.as_str()).map(str::to_string),
        run_count: job.get("run_count").and_then(|v| v.as_u64()).unwrap_or(0),
        timezone: job.get("timezone").and_then(|v| v.as_str()).unwrap_or("UTC").to_string(),
    })
}

/// Get all scheduled jobs
#[tauri::command]
async fn list_cron_jobs(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<CronJobInfo>, String> {
    let state_guard = state.read().await;

    // Daemon mode: the daemon is the cron runner — its list is the truth
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard.backend.scheduler_list().await?;
        let jobs = result
            .get("jobs")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(cron_job_info_from_daemon).collect())
            .unwrap_or_default();
        return Ok(jobs);
    }

    let scheduler = state_guard.scheduler.read().await;
    let tasks = scheduler.list_tasks().await;

    let jobs: Vec<CronJobInfo> = tasks.into_iter().map(|t| {
        let (schedule, next_run, schedule_description) = match &t.task_type {
            nanna_core::TaskType::Heartbeat => {
                ("heartbeat".to_string(), None, "Periodic heartbeat".to_string())
            }
            nanna_core::TaskType::Cron { schedule, next_run, parsed } => {
                let desc = parsed.as_ref()
                    .map(|p| p.describe())
                    .unwrap_or_else(|| schedule.clone());
                (schedule.clone(), next_run.map(|dt| dt.to_rfc3339()), desc)
            }
            nanna_core::TaskType::Recurring { interval } => {
                let secs = interval.as_secs();
                let desc = if secs >= 3600 {
                    format!("Every {} hours", secs / 3600)
                } else if secs >= 60 {
                    format!("Every {} minutes", secs / 60)
                } else {
                    format!("Every {} seconds", secs)
                };
                (format!("every_{}s", secs), None, desc)
            }
            nanna_core::TaskType::Delayed { delay, .. } => {
                (format!("delay_{}s", delay.as_secs()), None, "One-shot delayed".to_string())
            }
        };

        CronJobInfo {
            id: t.id.clone(),
            name: t.name.clone(),
            schedule,
            schedule_description,
            payload: t.payload.clone(),
            enabled: t.enabled,
            last_run: t.last_run.map(|dt| dt.to_rfc3339()),
            next_run,
            run_count: t.run_count,
            timezone: t.timezone.clone(),
        }
    }).collect();

    Ok(jobs)
}

/// Create a new cron job
#[tauri::command]
async fn create_cron_job(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
    schedule: String,
    payload: String,
    timezone: Option<String>,
) -> Result<CronJobInfo, String> {
    use nanna_core::CronExpr;

    // Validate the cron expression
    let parsed = CronExpr::parse(&schedule).map_err(|e| e.to_string())?;
    let next_run = parsed.next_from_now();

    let task = nanna_core::ScheduledTask {
        id: format!("{}-{}", name, uuid::Uuid::new_v4()),
        name: name.clone(),
        task_type: nanna_core::TaskType::Cron {
            schedule: schedule.clone(),
            parsed: Some(parsed.clone()),
            next_run,
        },
        payload: payload.clone(),
        enabled: true,
        last_run: None,
        run_count: 0,
        timezone: timezone.clone().unwrap_or_else(|| "UTC".to_string()),
        target_channel: None,
        target_session: None,
    };

    let state_guard = state.read().await;

    // Daemon mode: create the job on the daemon (it owns nanna.db and runs
    // the cron loop). Note: the daemon's add defaults to UTC — a custom
    // timezone is not forwarded over IPC yet.
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard
            .backend
            .scheduler_add(&schedule, &payload, Some(&name))
            .await?;
        if result.get("error").is_some() {
            let msg = result
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(format!("Daemon failed to create cron job: {msg}"));
        }
        let id = result
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("Daemon returned no job id")?
            .to_string();
        return Ok(CronJobInfo {
            id,
            name,
            schedule: schedule.clone(),
            schedule_description: parsed.describe(),
            payload,
            enabled: true,
            last_run: None,
            next_run: next_run.map(|dt| dt.to_rfc3339()),
            run_count: 0,
            timezone: "UTC".to_string(),
        });
    }

    let scheduler = state_guard.scheduler.read().await;
    scheduler.add_task(task.clone()).await;

    Ok(CronJobInfo {
        id: task.id.clone(),
        name: task.name,
        schedule: schedule.clone(),
        schedule_description: parsed.describe(),
        payload,
        enabled: true,
        last_run: None,
        next_run: next_run.map(|dt| dt.to_rfc3339()),
        run_count: 0,
        timezone: timezone.unwrap_or_else(|| "UTC".to_string()),
    })
}

/// Update a cron job's schedule
#[tauri::command]
async fn update_cron_job(
    state: State<'_, Arc<RwLock<AppState>>>,
    job_id: String,
    schedule: String,
) -> Result<bool, String> {
    let state_guard = state.read().await;
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard
            .backend
            .scheduler_update(&job_id, Some(&schedule), None, None)
            .await?;
        return match result.get("error").and_then(|v| v.as_str()) {
            None => Ok(true),
            Some("not_found") => Ok(false),
            Some(_) => Err(result
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error")
                .to_string()),
        };
    }
    let scheduler = state_guard.scheduler.read().await;
    scheduler.update_schedule(&job_id, &schedule).await.map_err(|e| e.to_string())
}

/// Enable or disable a cron job
#[tauri::command]
async fn set_cron_job_enabled(
    state: State<'_, Arc<RwLock<AppState>>>,
    job_id: String,
    enabled: bool,
) -> Result<(), String> {
    let state_guard = state.read().await;
    if state_guard.backend.is_daemon_mode().await {
        state_guard
            .backend
            .scheduler_update(&job_id, None, None, Some(enabled))
            .await?;
        return Ok(());
    }
    let scheduler = state_guard.scheduler.read().await;
    scheduler.set_task_enabled(&job_id, enabled).await;
    Ok(())
}

/// Delete a cron job
#[tauri::command]
async fn delete_cron_job(
    state: State<'_, Arc<RwLock<AppState>>>,
    job_id: String,
) -> Result<bool, String> {
    let state_guard = state.read().await;
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard.backend.scheduler_remove(&job_id).await?;
        return Ok(result.get("status").and_then(|v| v.as_str()) == Some("deleted"));
    }
    let scheduler = state_guard.scheduler.read().await;
    Ok(scheduler.remove_task(&job_id).await)
}

/// Delete all cron jobs with a given name (useful for cleanup)
#[tauri::command]
async fn delete_cron_jobs_by_name(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
) -> Result<usize, String> {
    let state_guard = state.read().await;
    if state_guard.backend.is_daemon_mode().await {
        // No by-name removal over IPC — list, filter, remove each.
        let result = state_guard.backend.scheduler_list().await?;
        let ids: Vec<String> = result
            .get("jobs")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter(|j| j.get("name").and_then(|v| v.as_str()) == Some(name.as_str()))
                    .filter_map(|j| j.get("id").and_then(|v| v.as_str()).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let mut removed = 0;
        for id in &ids {
            let result = state_guard.backend.scheduler_remove(id).await?;
            if result.get("status").and_then(|v| v.as_str()) == Some("deleted") {
                removed += 1;
            }
        }
        return Ok(removed);
    }
    let scheduler = state_guard.scheduler.read().await;
    Ok(scheduler.remove_tasks_by_name(&name).await)
}

/// Run a cron job immediately
#[tauri::command]
async fn run_cron_job_now(
    state: State<'_, Arc<RwLock<AppState>>>,
    job_id: String,
) -> Result<Option<JobRunInfo>, String> {
    let state_guard = state.read().await;

    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard.backend.scheduler_run_now(&job_id).await?;
        if result.get("error").is_some() {
            return Ok(None);
        }
        let now = chrono::Utc::now().to_rfc3339();
        return Ok(Some(JobRunInfo {
            id: 0,
            job_id,
            started_at: now.clone(),
            finished_at: Some(now),
            success: result.get("status").and_then(|v| v.as_str()) == Some("success"),
            output: result.get("output").and_then(|v| v.as_str()).map(str::to_string),
            error: result.get("error").and_then(|v| v.as_str()).map(str::to_string),
            duration_ms: result.get("duration_ms").and_then(serde_json::Value::as_i64),
        }));
    }

    let scheduler = state_guard.scheduler.read().await;

    if let Some(result) = scheduler.run_now(&job_id).await {
        Ok(Some(JobRunInfo {
            id: 0,
            job_id: result.task_id,
            started_at: result.started_at.to_rfc3339(),
            finished_at: Some(result.finished_at.to_rfc3339()),
            success: result.success,
            output: result.output,
            error: result.error,
            duration_ms: Some(result.duration_ms as i64),
        }))
    } else {
        Ok(None)
    }
}

/// Get job run history
#[tauri::command]
async fn get_cron_job_history(
    state: State<'_, Arc<RwLock<AppState>>>,
    job_id: String,
    limit: Option<usize>,
) -> Result<Vec<JobRunInfo>, String> {
    let state_guard = state.read().await;

    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard
            .backend
            .scheduler_history(&job_id, limit)
            .await?;
        let runs = result
            .get("history")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|r| JobRunInfo {
                        id: r.get("run_id").and_then(serde_json::Value::as_i64).unwrap_or(0),
                        job_id: r
                            .get("job_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        started_at: r
                            .get("started_at")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        finished_at: r
                            .get("finished_at")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        success: r.get("success").and_then(|v| v.as_bool()).unwrap_or(false),
                        output: r.get("output").and_then(|v| v.as_str()).map(str::to_string),
                        error: r.get("error").and_then(|v| v.as_str()).map(str::to_string),
                        duration_ms: None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        return Ok(runs);
    }

    let scheduler = state_guard.scheduler.read().await;

    let runs = scheduler.get_history(&job_id, limit.unwrap_or(20)).await;

    Ok(runs.into_iter().map(|r| JobRunInfo {
        id: r.id,
        job_id: r.job_id,
        started_at: r.started_at.to_rfc3339(),
        finished_at: r.finished_at.map(|dt| dt.to_rfc3339()),
        success: r.success,
        output: r.output,
        error: r.error,
        duration_ms: None, // JobRun from scheduler doesn't track this yet
    }).collect())
}

/// Validate a cron expression
#[tauri::command]
async fn validate_cron_expression(
    expression: String,
) -> Result<(bool, String), String> {
    use nanna_core::CronExpr;

    match CronExpr::parse(&expression) {
        Ok(parsed) => {
            let description = parsed.describe();
            let next = parsed.next_from_now()
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "N/A".to_string());
            Ok((true, format!("{} (next: {})", description, next)))
        }
        Err(e) => Ok((false, e.to_string())),
    }
}

// =============================================================================
// App Setup
// =============================================================================

async fn setup_state(
    backend: Arc<Backend>,
    mode: BackendMode,
) -> Result<AppState, Box<dyn std::error::Error + Send + Sync>> {
    // Load config
    let config = Config::load().unwrap_or_default().with_env_overrides();

    // Determine database path
    let db_path = Config::default_data_dir()
        .map(|d| d.join("nanna.db").to_string_lossy().to_string())
        .unwrap_or_else(|_| "nanna.db".to_string());

    // Initialize storage (Arc-wrapped for sharing with scheduler). Turso holds
    // an exclusive file lock, so in daemon mode the daemon owns nanna.db and
    // the GUI must not open it.
    let storage = match mode {
        BackendMode::Embedded => {
            let storage_config = StorageConfig { path: db_path };
            Some(Arc::new(Storage::new(&storage_config).await?))
        }
        BackendMode::Daemon => {
            info!("Daemon mode: local storage not opened (the daemon owns nanna.db)");
            None
        }
    };

    // Initialize LLM client (check for OAuth first)
    let llm = match config.llm.provider.as_str() {
        "anthropic" => {
            // Check if OAuth is enabled and has a token
            if config.llm.anthropic_use_oauth {
                if let Some(ref oauth_token) = config.llm.anthropic_oauth_token {
                    info!("Using Anthropic OAuth authentication");
                    LlmClient::anthropic_oauth(oauth_token)
                } else {
                    // OAuth enabled but no token - fall back to API key
                    let api_key = config.llm.api_key.clone()
                        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                        .unwrap_or_else(|| "missing-key".to_string());
                    LlmClient::anthropic(&api_key)
                }
            } else {
                let api_key = config.llm.api_key.clone()
                    .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                    .unwrap_or_else(|| "missing-key".to_string());
                LlmClient::anthropic(&api_key)
            }
        }
        "openai" => {
            let api_key = config.llm.openai_api_key.clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .unwrap_or_else(|| "missing-key".to_string());
            LlmClient::openai(&api_key)
        }
        "openrouter" => {
            let api_key = config.llm.openrouter_api_key.clone()
                .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
                .unwrap_or_else(|| "missing-key".to_string());
            LlmClient::openrouter(&api_key)
        }
        "github" => {
            let api_key = config.llm.github_token.clone()
                .or_else(|| std::env::var("GITHUB_TOKEN").ok())
                .unwrap_or_else(|| "missing-key".to_string());
            LlmClient::github_models(&api_key)
        }
        _ => {
            let api_key = config.llm.api_key.clone()
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                .unwrap_or_else(|| "missing-key".to_string());
            LlmClient::anthropic(&api_key)
        }
    };
    let llm = Arc::new(llm);

    // Set env vars from config (so they're available for model fetching and API calls)
    if let Some(ref key) = config.llm.openai_api_key {
        unsafe { std::env::set_var("OPENAI_API_KEY", key); }
    }
    if let Some(ref key) = config.llm.openrouter_api_key {
        unsafe { std::env::set_var("OPENROUTER_API_KEY", key); }
    }
    if let Some(ref key) = config.llm.github_token {
        unsafe { std::env::set_var("GITHUB_TOKEN", key); }
    }

    // Initialize tools
    let tools = ToolRegistry::new();

    // Register built-in tools
    tools.register(nanna_tools::ReadFileTool::new()).await;
    tools.register(nanna_tools::WriteFileTool::new()).await;
    tools.register(nanna_tools::ListDirTool::new()).await;
    tools.register(nanna_tools::ExecTool::new()).await;
    tools.register(nanna_tools::WebFetchTool::new()).await;

    // WebSearchTool requires BRAVE_API_KEY (env var or config)
    let brave_key = std::env::var("BRAVE_API_KEY").ok()
        .or_else(|| config.tools.brave_api_key.clone());
    let web_search = if let Some(key) = brave_key {
        // Set env var so it's available for later checks
        unsafe { std::env::set_var("BRAVE_API_KEY", &key); }
        nanna_tools::WebSearchTool::new().with_api_key(key)
    } else {
        info!("BRAVE_API_KEY not set - web_search will be unavailable, use web_fetch instead");
        nanna_tools::WebSearchTool::new()
    };
    tools.register(web_search).await;
    tools.register(nanna_tools::EchoTool).await;

    // Register common aliases for Claude Code compatibility
    // Claude Code uses: read, Write, bash, glob, etc.
    tools.register_alias("read", "read_file").await;
    tools.register_alias("Read", "read_file").await;
    tools.register_alias("write", "write_file").await;
    tools.register_alias("Write", "write_file").await;
    tools.register_alias("bash", "exec").await;
    tools.register_alias("Bash", "exec").await;
    tools.register_alias("glob", "list_dir").await;
    tools.register_alias("Glob", "list_dir").await;
    tools.register_alias("ls", "list_dir").await;
    info!("Registered Claude Code tool aliases (read, write, bash, glob)");

    // Initialize FSRS-6 cognitive memory service
    // Load embedding config from saved config file
    let saved_embedding_provider = config.memory.embedding_provider.clone();
    let saved_embedding_model = config.memory.embedding_model.clone();
    let saved_ollama_host = config.memory.ollama_host.clone();

    // Get API keys
    let openai_key = std::env::var("OPENAI_API_KEY").ok()
        .or_else(|| config.llm.openai_api_key.clone());

    info!("Loaded embedding config: provider={}, model={}, ollama_host={}",
          saved_embedding_provider, saved_embedding_model, saved_ollama_host);

    // Initialize based on configured provider
    let (embedding_provider, embedding_model, embedding_enabled, memory) =
        match saved_embedding_provider.as_str() {
            "openai" => {
                if let Some(openai_key) = openai_key {
                    unsafe { std::env::set_var("OPENAI_API_KEY", &openai_key); }
                    info!("Using OpenAI embeddings with model: {}", saved_embedding_model);

                    // Get dimension from model info cache/API
                    let embed_llm = LlmClient::openai(&openai_key);
                    let cache = ModelInfoCache::default_location();
                    let model_info = embed_llm.get_model_info(&saved_embedding_model, cache.as_ref()).await;
                    let dimension = match model_info.embedding_dimension {
                        Some(dimension) => dimension,
                        None => nanna_llm::EmbeddingClient::openai(&openai_key)
                            .with_model(&saved_embedding_model)
                            .embed_one("dimension probe").await
                            .map_err(|e| format!("Failed to discover embedding dimension: {e}"))?.len(),
                    };
                    info!("Embedding dimension: {} for model {} (from cache/API)", dimension, saved_embedding_model);

                    let memory_config = MemoryServiceConfig {
                        dimension,
                        ..Default::default()
                    };

                    let embed_client = reqwest::Client::new();
                    let embed_key = openai_key.clone();
                    let model_name = saved_embedding_model.clone();

                    let embed_fn: nanna_memory::EmbedFn = Arc::new(move |text: &str| {
                        let client = embed_client.clone();
                        let key = embed_key.clone();
                        let model = model_name.clone();
                        let text = text.to_string();

                        Box::pin(async move {
                            let response = client
                                .post("https://api.openai.com/v1/embeddings")
                                .header("Authorization", format!("Bearer {}", key))
                                .json(&serde_json::json!({
                                    "model": model,
                                    "input": text
                                }))
                                .send()
                                .await
                                .map_err(|e| e.to_string())?;

                            let json: serde_json::Value = response
                                .json()
                                .await
                                .map_err(|e| e.to_string())?;

                            let embedding = json["data"][0]["embedding"]
                                .as_array()
                                .ok_or("No embedding in response")?
                                .iter()
                                .filter_map(|v| v.as_f64().map(|f| f as f32))
                                .collect::<Vec<f32>>();

                            if embedding.is_empty() {
                                return Err("Empty embedding returned".to_string());
                            }

                            Ok(embedding)
                        })
                    });

                    (
                        "openai".to_string(),
                        saved_embedding_model.clone(),
                        true,
                        MemoryService::new(memory_config).with_embed_fn(embed_fn),
                    )
                } else {
                    info!("OpenAI embeddings configured but no API key - disabling");
                    (
                        "disabled".to_string(),
                        "none".to_string(),
                        false,
                        MemoryService::new(MemoryServiceConfig::default()),
                    )
                }
            }
            "ollama" => {
                let ollama_url = saved_ollama_host.clone();
                info!("Using Ollama embeddings at {} with model: {}", ollama_url, saved_embedding_model);

                // Get dimension from model info cache/API (Ollama /api/show endpoint)
                let embed_llm = LlmClient::ollama(&ollama_url);
                let cache = ModelInfoCache::default_location();
                let model_info = embed_llm.get_model_info(&saved_embedding_model, cache.as_ref()).await;
                let dimension = match model_info.embedding_dimension {
                    Some(dimension) => dimension,
                    None => nanna_llm::EmbeddingClient::ollama(&ollama_url)
                        .with_model(&saved_embedding_model)
                        .embed_one("dimension probe").await
                        .map_err(|e| format!("Failed to discover embedding dimension: {e}"))?.len(),
                };
                info!("Embedding dimension: {} for model {} (from cache/API)", dimension, saved_embedding_model);

                let memory_config = MemoryServiceConfig {
                    dimension,
                    ..Default::default()
                };

                let embed_client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(60))
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new());

                let model_name = saved_embedding_model.clone();
                let embed_fn: nanna_memory::EmbedFn = Arc::new(move |text: &str| {
                    let client = embed_client.clone();
                    let url = ollama_url.clone();
                    let model = model_name.clone();
                    let text = text.to_string();

                    Box::pin(async move {
                        let response = client
                            .post(format!("{}/api/embeddings", url))
                            .header("Content-Type", "application/json")
                            .json(&serde_json::json!({
                                "model": model,
                                "prompt": text
                            }))
                            .send()
                            .await
                            .map_err(|e| e.to_string())?;

                        if !response.status().is_success() {
                            let status = response.status();
                            let body = response.text().await.unwrap_or_default();
                            return Err(format!("Ollama error {}: {}", status, body));
                        }

                        let json: serde_json::Value = response
                            .json()
                            .await
                            .map_err(|e| e.to_string())?;

                        let embedding = json["embedding"]
                            .as_array()
                            .ok_or("No embedding in Ollama response")?
                            .iter()
                            .filter_map(|v| v.as_f64().map(|f| f as f32))
                            .collect::<Vec<f32>>();

                        if embedding.is_empty() {
                            return Err("Empty embedding returned from Ollama".to_string());
                        }

                        Ok(embedding)
                    })
                });

                (
                    "ollama".to_string(),
                    saved_embedding_model.clone(),
                    true,
                    MemoryService::new(memory_config).with_embed_fn(embed_fn),
                )
            }
            _ => {
                info!("Embedding provider disabled");
                (
                    "disabled".to_string(),
                    "none".to_string(),
                    false,
                    MemoryService::new(MemoryServiceConfig::default()),
                )
            }
        };
    let memory = Arc::new(memory);

    // Load persisted memories if they exist
    let memory_path = Config::default_data_dir()
        .map(|d| d.join("memories.json"))
        .unwrap_or_else(|_| std::path::PathBuf::from("memories.json"));

    if memory_path.exists() {
        match memory.load(&memory_path).await {
            Ok(()) => info!("Loaded {} memories from {:?}", memory.count().await, memory_path),
            Err(e) => warn!("Failed to load memories (starting fresh): {}", e),
        }
    } else {
        info!("No saved memories found at {:?} (starting fresh)", memory_path);
    }

    // Shared workspace registry — constructed early so memory tools can
    // thread a live handle into the adapter and scope remembers/recalls to
    // the *current* active workspace. Later AppState construction reuses this
    // exact Arc so set/clear_active_workspace keep the adapter in step.
    let workspaces: Arc<RwLock<WorkspaceRegistry>> = {
        let mut registry = WorkspaceRegistry::new();
        if let Some(ref storage) = storage {
            match storage.workspaces().list().await {
                Ok(records) if !records.is_empty() => {
                    let mut active_id = None;
                    for record in &records {
                        let path = std::path::PathBuf::from(&record.path);
                        if path.exists() {
                            let mut ws = Workspace::new(&path);
                            ws.id = record.id.clone();
                            if let Err(e) = ws.load_context().await {
                                warn!(
                                    "Failed to load workspace context for {}: {}",
                                    record.path, e
                                );
                            }
                            registry.register(ws);
                            if record.active {
                                active_id = Some(record.id.clone());
                            }
                        } else {
                            warn!(
                                "Persisted workspace path no longer exists: {}",
                                record.path
                            );
                        }
                    }
                    if let Some(id) = active_id {
                        registry.set_active(&id);
                    }
                    info!("Restored {} workspaces from database", records.len());
                }
                Ok(_) => {}
                Err(e) => {
                    warn!("Failed to load workspaces from database: {}", e);
                }
            }
        } else {
            match backend.workspace_list().await {
                Ok(result) => {
                    let records = result
                        .get("workspaces")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    let active_id = result
                        .get("active_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let count = records.len();
                    for record in &records {
                        let (Some(id), Some(path)) = (
                            record.get("id").and_then(|v| v.as_str()),
                            record.get("path").and_then(|v| v.as_str()),
                        ) else {
                            continue;
                        };
                        let path = std::path::PathBuf::from(path);
                        if path.exists() {
                            let mut ws = Workspace::new(&path);
                            ws.id = id.to_string();
                            if let Err(e) = ws.load_context().await {
                                warn!(
                                    "Failed to load workspace context for {:?}: {}",
                                    path, e
                                );
                            }
                            registry.register(ws);
                        } else {
                            warn!("Daemon workspace path no longer exists: {:?}", path);
                        }
                    }
                    if let Some(id) = active_id {
                        registry.set_active(&id);
                    }
                    if count > 0 {
                        info!("Restored {} workspaces from daemon", count);
                    }
                }
                Err(e) => {
                    warn!("Failed to load workspaces from daemon: {}", e);
                }
            }
        }
        Arc::new(RwLock::new(registry))
    };

    // Register memory tools (remember, recall, reflect) with the FSRS memory service
    let memory_storage: nanna_tools::StorageHandle = Arc::new(MemoryServiceAdapter::new(memory.clone(), workspaces.clone()));
    tools.register(nanna_tools::RememberTool::new(memory_storage.clone())).await;
    tools.register(nanna_tools::RecallTool::new(memory_storage.clone())).await;
    tools.register(nanna_tools::ReflectTool::new(memory_storage)).await;
    info!("Registered memory tools (remember, recall, reflect)");

    // Initialize user tool authoring system
    let user_tools_dir = Config::default_data_dir()
        .map(|d| d.join("user_tools"))
        .unwrap_or_else(|_| std::path::PathBuf::from("user_tools"));
    let user_tool_manager = Arc::new(tool_authoring::UserToolManager::new(user_tools_dir));

    // Load existing user tools
    match user_tool_manager.load_all().await {
        Ok(count) => info!("Loaded {} user-created tools", count),
        Err(e) => warn!("Failed to load user tools: {}", e),
    }

    // Register user tools with the registry
    let tools = Arc::new(tools);
    let registered = user_tool_manager.register_with_registry(&tools).await;
    info!("Registered {} user tools with the tool registry", registered);

    // Register create_tool and list_user_tools tools (so Nanna can create tools at runtime)
    tools.register(tool_authoring::CreateToolTool::new(user_tool_manager.clone(), tools.clone())).await;
    tools.register(tool_authoring::ListUserToolsTool::new(user_tool_manager.clone())).await;
    info!("Registered tool authoring tools (create_tool, list_user_tools)");

    // Register discover_tools (JS/TS skill with registry access)
    {
        let tools_dir = nanna_tools::skills::defaults::resolve_tools_dir(
            config.tools.tools_dir.as_deref()
        );
        if let Some(ref dir) = tools_dir {
            if let Some(source) = nanna_tools::skills::defaults::load_discover_tools_source(dir) {
                let wrapper = nanna_tools::skills::ScriptedToolWrapper::from_source("discover_tools", &source)
                    .expect("discover_tools skill must parse")
                    .with_registry(std::sync::Arc::downgrade(&tools));
                tools.register(wrapper).await;
                info!("Registered discover_tools skill from {:?}", dir);
            } else {
                warn!("discover_tools not found in tools directory");
            }
        }
    }

    // Initialize scheduler with consolidation task
    let scheduler_config = SchedulerConfig {
        heartbeat_interval: Duration::from_secs(1800), // 30 minutes
        heartbeat_enabled: true, // Enable heartbeats for autonomous operation
        heartbeat_prompt: "Read HEARTBEAT.md if it exists (workspace context). Follow it strictly. Do not infer or repeat old tasks from prior chats. If nothing needs attention, reply HEARTBEAT_OK.".to_string(),
        max_concurrent: 4,
        check_interval: Duration::from_secs(30),
        default_timezone: "UTC".to_string(),
    };
    let mut scheduler = Scheduler::new(scheduler_config);

    // Cron persistence needs the local DB; in daemon mode the scheduler still
    // runs (the daemon has no cron runner yet) but jobs live in memory only.
    if let Some(ref storage) = storage {
        scheduler = scheduler.with_storage(Arc::clone(storage));

        // Load persisted cron jobs from storage
        match scheduler.load_jobs().await {
            Ok(count) => info!("Loaded {} cron jobs from database", count),
            Err(e) => warn!("Failed to load cron jobs: {}", e),
        }
    } else {
        info!("Daemon mode: scheduler running without cron persistence");
    }

    // Deduplicate consolidation tasks (fix for historical duplicates)
    let deduped = scheduler.deduplicate_by_name("memory_consolidation").await;
    if deduped > 0 {
        info!("Removed {} duplicate consolidation tasks", deduped);
    }

    // Only add consolidation task if one doesn't already exist
    if !scheduler.has_task_named("memory_consolidation").await {
        let consolidation = consolidation_task(Some(Duration::from_secs(3600)));
        scheduler.add_task(consolidation).await;
        info!("Scheduled memory consolidation task (every 1 hour)");
    } else {
        info!("Memory consolidation task already scheduled");
    }

    // Create executor for scheduled tasks
    let memory_for_executor = memory.clone();
    let tools_for_executor = tools.clone();
    let channels_for_executor = config.channels.clone();
    let config_for_executor = config.clone();
    let ollama_host_for_executor = saved_ollama_host.clone();

    let executor: nanna_core::TaskExecutor = Arc::new(move |task| {
        let memory = memory_for_executor.clone();
        let tools = tools_for_executor.clone();
        let channels = channels_for_executor.clone();
        let config = config_for_executor.clone();
        let ollama_host = ollama_host_for_executor.clone();

        Box::pin(async move {
            let start = std::time::Instant::now();
            let started_at = chrono::Utc::now();

            match task.name.as_str() {
                "heartbeat" => {
                    info!("Running heartbeat...");

                    // Build the heartbeat prompt with context
                    let prompt = task.payload.clone();

                    // Get the first available model from priority list
                    let priority = &config.llm.model_priority;
                    let (llm, model) = if let Some(model_id) = priority.first() {
                        if let Some((client, actual_model)) = create_llm_client_for_model(model_id, &config, &ollama_host) {
                            (Arc::new(client), actual_model)
                        } else {
                            // Fallback to default model
                            let api_key = config.llm.api_key.clone()
                                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                                .unwrap_or_default();
                            (Arc::new(LlmClient::anthropic(&api_key)), config.llm.model.clone())
                        }
                    } else {
                        // No priority list, use default
                        let api_key = config.llm.api_key.clone()
                            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                            .unwrap_or_default();
                        (Arc::new(LlmClient::anthropic(&api_key)), config.llm.model.clone())
                    };

                    // Create agent config
                    let agent_config = nanna_agent::AgentConfig {
                        model: model.clone(),
                        max_iterations: Some(5), // Limit turns for heartbeat
                        ..Default::default()
                    };

                    // Create agent and run
                    let agent = nanna_agent::Agent::new(
                        agent_config,
                        llm.clone(),
                        tools.clone(),
                    );

                    // Run the agent with the heartbeat prompt
                    let run_options = nanna_agent::RunOptions::default();
                    match agent.run(&prompt, run_options).await {
                        Ok(response) => {
                            let finished_at = chrono::Utc::now();
                            let output = response.text.clone();

                            // Check if agent responded with HEARTBEAT_OK
                            let is_heartbeat_ok = output.trim().starts_with("HEARTBEAT_OK")
                                || output.trim().ends_with("HEARTBEAT_OK")
                                || output.trim() == "HEARTBEAT_OK";

                            if is_heartbeat_ok {
                                debug!("Heartbeat: OK (nothing to do)");
                            } else {
                                info!("Heartbeat response: {}", output.chars().take(200).collect::<String>());

                                // Route to channel if specified
                                if let Some(ref channel_id) = task.target_channel {
                                    if let Err(e) = route_to_channel(&channels, channel_id, &output).await {
                                        warn!("Failed to route heartbeat to channel {}: {}", channel_id, e);
                                    }
                                }
                            }

                            nanna_core::TaskResult {
                                task_id: task.id.clone(),
                                task_name: task.name.clone(),
                                success: true,
                                output: Some(if is_heartbeat_ok {
                                    "HEARTBEAT_OK".to_string()
                                } else {
                                    output
                                }),
                                error: None,
                                duration_ms: start.elapsed().as_millis() as u64,
                                started_at,
                                finished_at,
                            }
                        }
                        Err(e) => {
                            let finished_at = chrono::Utc::now();
                            error!("Heartbeat failed: {}", e);
                            nanna_core::TaskResult {
                                task_id: task.id.clone(),
                                task_name: task.name.clone(),
                                success: false,
                                output: None,
                                error: Some(e.to_string()),
                                duration_ms: start.elapsed().as_millis() as u64,
                                started_at,
                                finished_at,
                            }
                        }
                    }
                }
                "memory_consolidation" => {
                    info!("Running scheduled memory consolidation...");

                    // Get the first available model from priority list for summarization
                    let priority = &config.llm.model_priority;
                    let llm_for_consolidation = if let Some(model_id) = priority.first() {
                        if let Some((client, _)) = create_llm_client_for_model(model_id, &config, &ollama_host) {
                            Arc::new(client)
                        } else {
                            let api_key = config.llm.api_key.clone()
                                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                                .unwrap_or_default();
                            Arc::new(LlmClient::anthropic(&api_key))
                        }
                    } else {
                        let api_key = config.llm.api_key.clone()
                            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                            .unwrap_or_default();
                        Arc::new(LlmClient::anthropic(&api_key))
                    };

                    let consolidation_config = ConsolidationConfig::default();
                    let summarize = |prompt: String| {
                        let llm = llm_for_consolidation.clone();
                        async move {
                            let request = nanna_llm::CompletionRequest::default()
                                .with_model("claude-3-5-haiku-20241022")
                                .with_message(nanna_llm::Message::user(&prompt));
                            llm.complete(&request).await.map_err(|e| e.to_string())
                        }
                    };

                    match memory.consolidate(&consolidation_config, summarize).await {
                        Ok(result) => {
                            let finished_at = chrono::Utc::now();
                            info!(
                                "Scheduled consolidation: {} processed, {} merged",
                                result.memories_processed, result.memories_merged
                            );
                            nanna_core::TaskResult {
                                task_id: task.id.clone(),
                                task_name: task.name.clone(),
                                success: true,
                                output: Some(format!("Processed {} memories", result.memories_processed)),
                                error: None,
                                duration_ms: start.elapsed().as_millis() as u64,
                                started_at,
                                finished_at,
                            }
                        }
                        Err(e) => {
                            let finished_at = chrono::Utc::now();
                            error!("Scheduled consolidation failed: {}", e);
                            nanna_core::TaskResult {
                                task_id: task.id.clone(),
                                task_name: task.name.clone(),
                                success: false,
                                output: None,
                                error: Some(e.to_string()),
                                duration_ms: start.elapsed().as_millis() as u64,
                                started_at,
                                finished_at,
                            }
                        }
                    }
                }
                _ => {
                    // Generic cron job - run as agent prompt
                    if !task.payload.is_empty() {
                        info!("Running cron job '{}': {}", task.name, task.payload.chars().take(50).collect::<String>());

                        // Get the first available model from priority list
                        let priority = &config.llm.model_priority;
                        let (llm, model) = if let Some(model_id) = priority.first() {
                            if let Some((client, actual_model)) = create_llm_client_for_model(model_id, &config, &ollama_host) {
                                (Arc::new(client), actual_model)
                            } else {
                                let api_key = config.llm.api_key.clone()
                                    .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                                    .unwrap_or_default();
                                (Arc::new(LlmClient::anthropic(&api_key)), config.llm.model.clone())
                            }
                        } else {
                            let api_key = config.llm.api_key.clone()
                                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                                .unwrap_or_default();
                            (Arc::new(LlmClient::anthropic(&api_key)), config.llm.model.clone())
                        };

                        let agent_config = nanna_agent::AgentConfig {
                            model: model.clone(),
                            max_iterations: Some(10),
                            ..Default::default()
                        };

                        let agent = nanna_agent::Agent::new(
                            agent_config,
                            llm.clone(),
                            tools.clone(),
                        );

                        let run_options = nanna_agent::RunOptions::default();
                        match agent.run(&task.payload, run_options).await {
                            Ok(response) => {
                                let finished_at = chrono::Utc::now();
                                let output = response.text;
                                info!("Cron job '{}' completed", task.name);

                                // Route to channel if specified
                                if let Some(ref channel_id) = task.target_channel {
                                    if let Err(e) = route_to_channel(&channels, channel_id, &output).await {
                                        warn!("Failed to route cron job to channel {}: {}", channel_id, e);
                                    }
                                }

                                nanna_core::TaskResult {
                                    task_id: task.id.clone(),
                                    task_name: task.name.clone(),
                                    success: true,
                                    output: Some(output),
                                    error: None,
                                    duration_ms: start.elapsed().as_millis() as u64,
                                    started_at,
                                    finished_at,
                                }
                            }
                            Err(e) => {
                                let finished_at = chrono::Utc::now();
                                error!("Cron job '{}' failed: {}", task.name, e);
                                nanna_core::TaskResult {
                                    task_id: task.id.clone(),
                                    task_name: task.name.clone(),
                                    success: false,
                                    output: None,
                                    error: Some(e.to_string()),
                                    duration_ms: start.elapsed().as_millis() as u64,
                                    started_at,
                                    finished_at,
                                }
                            }
                        }
                    } else {
                        let finished_at = chrono::Utc::now();
                        debug!("Skipping task with empty payload: {}", task.name);
                        nanna_core::TaskResult {
                            task_id: task.id.clone(),
                            task_name: task.name.clone(),
                            success: true,
                            output: Some("Skipped (empty payload)".to_string()),
                            error: None,
                            duration_ms: start.elapsed().as_millis() as u64,
                            started_at,
                            finished_at,
                        }
                    }
                }
            }
        })
    });

    scheduler = scheduler.with_executor(executor);
    match mode {
        BackendMode::Embedded => {
            scheduler.start();
            info!("Scheduler started with consolidation executor");
        }
        BackendMode::Daemon => {
            // The daemon runs heartbeat + cron (it owns nanna.db, so the
            // persisted jobs live there); starting a second scheduler here
            // would double-fire every task. Cron commands route over IPC.
            info!("Daemon mode: local scheduler not started (the daemon runs heartbeat + cron)");
        }
    }

    let scheduler = Arc::new(RwLock::new(scheduler));
    let last_consolidation = Arc::new(RwLock::new(None));

    info!("Nanna GUI initialized with model: {}", config.llm.model);
    info!("Registered {} tools", tools.definitions().await.len());
    info!("FSRS-6 cognitive memory enabled");

    // Get extraction model from config (empty = use chat model)
    let saved_extraction_model = config.memory.extraction_model.clone();

    // Get initial active model from priority list or default
    let initial_active_model = config.llm.model_priority.first()
        .cloned()
        .unwrap_or_else(|| config.llm.model.clone());

    // Wire the embedded backend into the already-initialized Backend. Only
    // possible when the GUI owns local storage (i.e. embedded mode).
    if let Some(ref storage) = storage {
        let embedded = embedded::EmbeddedBackend::new(
            llm.clone(),
            tools.clone(),
            memory.clone(),
            storage.clone(),
        ).await;
        backend.set_embedded(embedded).await;
        info!("Embedded backend ready (GUI owns local storage)");
    } else {
        info!("Daemon mode: embedded backend not constructed");
    }

    Ok(AppState {
        storage: storage.clone(),
        llm,
        tools,
        config,
        memory,
        memory_path,
        scheduler,
        last_consolidation,
        // Runtime settings - all enabled by default
        dreaming_enabled: Arc::new(RwLock::new(true)),
        scheduler_enabled: Arc::new(RwLock::new(true)),
        heartbeat_enabled: Arc::new(RwLock::new(true)),
        heartbeat_interval_seconds: Arc::new(RwLock::new(300)), // 5 minutes
        // Embedding settings (loaded from config)
        embedding_provider: Arc::new(RwLock::new(embedding_provider)),
        embedding_model: Arc::new(RwLock::new(embedding_model)),
        embedding_enabled: Arc::new(RwLock::new(embedding_enabled)),
        // Ollama host (from config)
        ollama_host: Arc::new(RwLock::new(saved_ollama_host)),
        // Extraction model (from config, empty = use chat model)
        extraction_model: Arc::new(RwLock::new(saved_extraction_model)),
        // Active model tracking (start with first in priority or default model)
        active_model: Arc::new(RwLock::new(initial_active_model)),
        // Rate limited models (empty at startup)
        rate_limited_models: Arc::new(RwLock::new(HashMap::new())),
        // Workspace registry — shared Arc constructed earlier so memory tools
        // observe the same active workspace as the GUI commands.
        workspaces,
        // User tool authoring manager
        user_tools: user_tool_manager,
        // Backend (initialized above with embedded mode)
        backend,
        // Close behavior (default: ask user)
        close_mode: Arc::new(RwLock::new(CloseMode::default())),
        // Embedded run state tracking (empty at startup)
        embedded_run_states: Arc::new(RwLock::new(HashMap::new())),
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("nanna=info".parse().unwrap()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle().clone();

            // Set up system tray
            setup_system_tray(app)?;

            // Initialize state asynchronously
            tauri::async_runtime::spawn(async move {
                // DAEMON-FIRST: decide the backend mode BEFORE opening any local
                // storage. Turso holds an exclusive file lock on nanna.db, so only
                // one process may own it — if the GUI opens it first, the daemon
                // sidecar boots storage-less (no sessions, no memory persistence).
                // The daemon is the preferred owner; the GUI only opens storage
                // itself when falling back to embedded mode.
                let backend = Arc::new(Backend::new());
                let mode = backend.init(&handle).await;
                info!("Backend initialized in {:?} mode", mode);

                match setup_state(backend, mode).await {
                    Ok(state) => {
                        // Create agent registry with shared LLM and tools
                        let agent_registry = agents::AgentRegistryState::new(
                            state.llm.clone(),
                            state.tools.clone(),
                        );

                        // Manage both states
                        handle.manage(Arc::new(RwLock::new(state)));
                        handle.manage(Arc::new(RwLock::new(agent_registry)));
                        info!("App state initialized successfully");
                    }
                    Err(e) => {
                        error!("Failed to initialize app state: {}", e);
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            send_message,
            create_session,
            list_sessions,
            get_session_history,
            delete_session,
            clear_all_sessions,
            archive_and_delete_session,
            rename_session,
            set_session_workspace,
            get_config,
            set_model,
            set_api_key,
            search_memory,
            get_memory_stats,
            show_window,
            hide_to_tray,
            get_extended_settings,
            set_provider_api_key,
            set_provider,
            // Anthropic OAuth (via claude setup-token)
            run_claude_setup_token,
            import_claude_code_credentials,
            save_anthropic_oauth_token,
            logout_anthropic_oauth,
            get_credential_status,
            refresh_oauth_token,
            check_env_var,
            // Cognitive memory (FSRS-6 + dreaming)
            get_cognitive_memory_stats,
            trigger_consolidation,
            apply_memory_updates,
            // Memory & scheduling settings
            set_dreaming_enabled,
            set_max_compression_ratio,
            set_min_remaining_memories,
            set_scheduler_enabled,
            set_heartbeat_enabled,
            set_heartbeat_interval,
            set_extraction_model,
            // Embedding configuration
            set_embedding_config,
            get_ollama_models,
            set_ollama_host,
            set_ollama_api_key,
            // Dynamic model fetching
            get_anthropic_models,
            get_openai_models,
            get_openrouter_models,
            get_openrouter_embedding_models,
            get_github_models,
            get_claude_proxy_models,
            set_claude_proxy,
            check_claude_proxy_health,
            // Memory persistence
            save_memories,
            // Memory management
            list_memories,
            get_memory,
            delete_memory,
            update_memory,
            clear_all_memories,
            // Channel status
            get_channel_status,
            get_enhanced_channel_status,
            test_all_channels,
            subscribe_channel_status,
            unsubscribe_channel_status,
            // Config persistence
            save_config,
            save_channel_config,
            test_channel_connection,
            // Notifications
            send_notification,
            request_notification_permission,
            check_notification_permission,
            // Similarity threshold
            get_similarity_threshold,
            set_similarity_threshold,
            // System prompt & agent settings
            get_system_prompt,
            set_system_prompt,
            set_agent_name,
            set_personality_mode,
            set_thinking_enabled,
            set_streaming_enabled,
            set_max_tokens,
            set_agent_iteration_policy,
            // Config import/export
            export_config,
            import_config,
            // Model priority (fallback chains)
            get_chat_model_priority,
            set_chat_model_priority,
            get_embedding_model_priority,
            set_embedding_model_priority,
            get_summarization_model_priority,
            set_summarization_model_priority,
            // OCR settings
            get_ocr_model_priority,
            set_ocr_model_priority,
            get_use_embedded_ocr,
            set_use_embedded_ocr,
            // Model routing
            get_model_routing,
            set_model_routing,
            get_routing_first_turn_primary,
            set_routing_first_turn_primary,
            get_sub_agent_model,
            set_sub_agent_model,
            // Model status
            get_model_status,
            get_model_stats,
            get_tool_stats,
            get_global_stats,
            get_tool_stats_hourly,
            get_tool_stats_daily,
            get_tool_call_log,
            spawn_sub_session,
            list_sub_sessions,
            kill_sub_session,
            get_sub_session_status,
            send_to_sub_session,
            clear_rate_limit,
            // Workspaces
            list_workspaces,
            open_workspace,
            set_active_workspace,
            clear_active_workspace,
            get_active_workspace,
            get_workspace_context,
            reload_workspace,
            close_workspace,
            discover_workspaces_in_path,
            find_workspace_root_from_path,
            save_workspace_file,
            append_workspace_memory,
            get_workspace_recent_memory,
            list_workspace_memory_files,
            init_workspace,
            read_workspace_file,
            check_workspace_validity,
            // Agent visualization
            agents::get_agent_clusters,
            agents::get_all_agents,
            agents::get_agent,
            agents::get_agent_children,
            agents::get_agent_stats,
            agents::cancel_agent,
            agents::subscribe_agent_events,
            agents::cleanup_completed_agents,
            agents::get_workspace_agents,
            // User tool authoring
            list_user_tools_cmd,
            get_user_tool,
            get_tool_source,
            create_user_tool,
            update_user_tool,
            delete_user_tool,
            test_user_tool,
            // All registered tools
            list_tools,
            get_tool,
            // Skill directory tools
            list_skills,
            create_skill,
            update_skill,
            delete_skill,
            test_skill,
            // Backend mode
            get_backend_status,
            init_backend,
            get_session_run_state,
            // Cancellation & Logs
            cancel_session,
            get_daemon_logs,
            // Window close behavior
            get_close_mode,
            set_close_mode,
            handle_window_close,
            perform_quit,
            // Scheduler / Cron jobs
            list_cron_jobs,
            create_cron_job,
            update_cron_job,
            set_cron_job_enabled,
            delete_cron_job,
            delete_cron_jobs_by_name,
            run_cron_job_now,
            get_cron_job_history,
            validate_cron_expression,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                // Shutdown backend (stop daemon sidecar) and save memories
                if let Some(state) = app.try_state::<Arc<RwLock<AppState>>>() {
                    let state = state.inner().clone();
                    tauri::async_runtime::block_on(async {
                        let state_guard = state.read().await;

                        // Stop the daemon sidecar
                        info!("Shutting down backend...");
                        state_guard.backend.shutdown().await;

                        let count = state_guard.memory.count().await;

                        // Only save if we have memories (prevents wiping on failed load)
                        if count > 0 {
                            // Create backup before saving
                            let backup_path = state_guard.memory_path.with_extension("json.bak");
                            if state_guard.memory_path.exists() {
                                if let Err(e) = std::fs::copy(&state_guard.memory_path, &backup_path) {
                                    warn!("Failed to create memory backup: {}", e);
                                }
                            }

                            if let Err(e) = state_guard.memory.save(&state_guard.memory_path).await {
                                error!("Failed to save memories on exit: {}", e);
                            } else {
                                info!("Saved {} memories to {:?}", count, state_guard.memory_path);
                            }
                        } else {
                            info!("No memories to save (count=0), skipping to preserve existing file");
                        }
                    });
                }
            }
        });
}

/// Set up the system tray icon and menu
fn setup_system_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show_item = MenuItemBuilder::with_id("show", "Show Nanna").build(app)?;
    let new_chat_item = MenuItemBuilder::with_id("new_chat", "New Chat").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&show_item)
        .item(&new_chat_item)
        .separator()
        .item(&quit_item)
        .build()?;

    let _tray = TrayIconBuilder::with_id("main")
        .tooltip("Nanna AI Assistant")
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| {
            match event.id().as_ref() {
                "show" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                "new_chat" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                        // Emit event to create new chat
                        let _ = app.emit("tray-new-chat", ());
                    }
                }
                "quit" => {
                    app.exit(0);
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    info!("System tray initialized");
    Ok(())
}


#[cfg(test)]
mod parse_model_id_tests {
    use super::{
        conversation_token_budget_for, parse_model_id,
    };
    use nanna_llm::unknown_model_info;

    #[test]
    fn parse_model_id_infers_provider_by_family_prefix() {
        assert_eq!(
            parse_model_id("gpt-4o"),
            ("openai".into(), "gpt-4o".into())
        );
        assert_eq!(
            parse_model_id("o1-preview"),
            ("openai".into(), "o1-preview".into())
        );
        assert_eq!(
            parse_model_id("o3-mini"),
            ("openai".into(), "o3-mini".into())
        );
        assert_eq!(
            parse_model_id("claude-opus-4"),
            ("anthropic".into(), "claude-opus-4".into())
        );
        // Case-insensitive on the family prefix.
        assert_eq!(
            parse_model_id("Claude-Sonnet-4"),
            ("anthropic".into(), "Claude-Sonnet-4".into())
        );
        assert_eq!(
            parse_model_id("llama3.2:latest"),
            ("ollama".into(), "llama3.2:latest".into())
        );
        assert_eq!(
            parse_model_id("deepseek-r1:14b"),
            ("ollama".into(), "deepseek-r1:14b".into())
        );
        // Unknown bare names still default to Anthropic.
        assert_eq!(
            parse_model_id("some-unknown-model"),
            ("anthropic".into(), "some-unknown-model".into())
        );
    }

    #[test]
    fn parse_model_id_respects_explicit_provider_prefixes() {
        assert_eq!(
            parse_model_id("openrouter/meta-llama/llama-3"),
            ("openrouter".into(), "meta-llama/llama-3".into())
        );
        assert_eq!(
            parse_model_id("github/gpt-4o"),
            ("github".into(), "gpt-4o".into())
        );
        assert_eq!(
            parse_model_id("ollama/qwen3"),
            ("ollama".into(), "qwen3".into())
        );
        assert_eq!(
            parse_model_id("openai/gpt-4o"),
            ("openai".into(), "gpt-4o".into())
        );
        assert_eq!(
            parse_model_id("anthropic/claude-opus-4"),
            ("anthropic".into(), "claude-opus-4".into())
        );
    }

    #[test]
    fn conversation_budget_uses_resolved_model_info() {
        // Small window (API/cache would report this) must not inherit a cloud-sized
        // default: budget must fit the hard input limit and keep a history floor.
        let small = nanna_llm::ModelInfo {
            id: "local".into(),
            context_window: 8_000,
            max_output_tokens: 4_096,
            supports_tools: true,
            supports_vision: false,
            embedding_dimension: None,
            cached_at: 0,
            provider: "test".into(),
        };
        let budget = conversation_token_budget_for(&small);
        assert!(
            budget <= small.hard_input_limit(),
            "budget {budget} exceeds hard input {}",
            small.hard_input_limit()
        );
        assert!(budget >= 2_000, "budget {budget} collapsed below floor");
    }

    #[test]
    fn conversation_budget_scales_with_large_context() {
        let large = nanna_llm::ModelInfo {
            id: "big".into(),
            context_window: 200_000,
            max_output_tokens: 8_192,
            supports_tools: true,
            supports_vision: false,
            embedding_dimension: None,
            cached_at: 0,
            provider: "test".into(),
        };
        let budget = conversation_token_budget_for(&large);
        assert!(budget > 100_000, "large-model budget {budget} regressed");
        assert!(budget < large.context_window);
    }

    #[test]
    fn uncached_model_name_uses_universal_floor() {
        // No per-model table: any name without a cache entry gets UNKNOWN_CONTEXT_WINDOW.
        let info = resolve_model_info_sync("some-unknown-local-model-xyz");
        assert_eq!(info.context_window, UNKNOWN_CONTEXT_WINDOW);
        let budget = conversation_token_budget_for(&info);
        let expected = conversation_token_budget_for(&unknown_model_info("x", ""));
        assert_eq!(budget, expected);
    }
}
