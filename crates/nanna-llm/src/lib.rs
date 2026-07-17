#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! LLM API client for Nanna
//!
//! Supports Anthropic Claude with proper tool calling and streaming.

use tracing::{debug, info, warn};

#[cfg(feature = "auto-refresh")]
pub mod oauth;

#[cfg(feature = "auto-refresh")]
pub use oauth::{OAuthClient, create_oauth_client, create_oauth_client_sync};

pub mod heal;
pub use heal::{heal_json, heal_json_as, heal_tool_args};

use async_stream::stream;
use futures::{Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum LlmError {
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("API error: {status} - {message}")]
    Api { status: u16, message: String },
    #[error("JSON error: {0}")]
    Json(String),
    #[error("Stream error: {0}")]
    Stream(String),
    #[error("Missing API key for provider: {0}")]
    MissingApiKey(String),
    #[error("Rate limit exceeded: {message}")]
    RateLimit {
        message: String,
        /// Seconds until rate limit resets (if available)
        retry_after: Option<u64>,
    },
    #[error("All fallback models exhausted")]
    AllModelsExhausted,
    #[error("IO error: {0}")]
    Io(String),
}

impl From<std::io::Error> for LlmError {
    fn from(e: std::io::Error) -> Self {
        LlmError::Io(e.to_string())
    }
}

// ============================================================================
// Model Information & Capabilities
// ============================================================================

/// Model capabilities and limits
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Model identifier
    pub id: String,
    /// Maximum input context window in tokens
    pub context_window: usize,
    /// Maximum output tokens
    pub max_output_tokens: usize,
    /// Whether the model supports tool/function calling
    #[serde(default)]
    pub supports_tools: bool,
    /// Whether the model supports vision/images
    #[serde(default)]
    pub supports_vision: bool,
    /// Embedding dimension (if this is an embedding model)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_dimension: Option<usize>,
    /// Unix timestamp when this info was cached
    #[serde(default)]
    pub cached_at: i64,
    /// Provider this info came from
    #[serde(default)]
    pub provider: String,
}

impl ModelInfo {
    /// Cache TTL: 1 week in seconds
    pub const CACHE_TTL_SECS: i64 = 7 * 24 * 60 * 60;

    /// Check if this cached info has expired
    #[must_use]
    pub fn is_expired(&self) -> bool {
        let now = current_timestamp();
        now - self.cached_at > Self::CACHE_TTL_SECS
    }

    /// Recommended compression threshold — where the context manager should
    /// *proactively* compress, always **below** [`Self::hard_input_limit`].
    ///
    /// Nominally 80% of the context window, but a small model with a large
    /// `max_output_tokens` (e.g. context 8k / output 4k) makes 80%-of-context
    /// *exceed* the hard input limit, so proactive compression would never fire
    /// before the hard cap and the agent would emergency-truncate every turn.
    /// Capping at 90% of the hard limit restores the invariant while leaving
    /// large models (where 80%-of-context is already the smaller value) unchanged.
    #[must_use]
    pub fn compression_threshold(&self) -> usize {
        let by_context = (self.context_window * 80) / 100;
        let below_hard = (self.hard_input_limit() * 90) / 100;
        let threshold = by_context.min(below_hard);
        debug_assert!(
            threshold <= self.hard_input_limit(),
            "compression must trigger before the hard input cap"
        );
        threshold
    }

    /// Hard limit for input (leaves room for output).
    ///
    /// Never returns 0 — some providers report `max_output_tokens >= context_window`
    /// which would produce 0 and cause the context manager to nuke all messages
    /// on every iteration.  Floor: 50% of context_window.
    #[must_use]
    pub fn hard_input_limit(&self) -> usize {
        let raw = self.context_window.saturating_sub(self.max_output_tokens);
        let floor = self.context_window / 2;
        raw.max(floor)
    }

    /// Create with current timestamp
    fn with_timestamp(mut self) -> Self {
        self.cached_at = current_timestamp();
        self
    }

    /// Tokens available for conversation history after hard-input + reserved heads.
    ///
    /// `system_reserved` / `response_reserved` cover prompt + completion heads that
    /// sit outside the stored history window (GUI system prompt + response reserve).
    /// Floored at `min_history` so truncation never empties the transcript, but
    /// never above the hard input limit itself.
    #[must_use]
    pub fn conversation_history_budget(
        &self,
        system_reserved: usize,
        response_reserved: usize,
        min_history: usize,
    ) -> usize {
        let hard = self.hard_input_limit();
        hard.saturating_sub(system_reserved.saturating_add(response_reserved))
            .max(min_history.min(hard))
    }
}

/// Get current Unix timestamp
fn current_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Universal context floor when no provider has reported limits for a model.
///
/// Providers update windows constantly; we never hardcode per-model values.
/// Callers should prefer [`LlmClient::get_model_info`] / [`ModelInfoCache`].
/// This floor is intentionally conservative so unknown / offline models stay
/// under their real window rather than overshoot and 400.
pub const UNKNOWN_CONTEXT_WINDOW: usize = 32_000;

/// Universal max-output floor when the provider does not report it.
pub const UNKNOWN_MAX_OUTPUT_TOKENS: usize = 4_096;

/// Best-effort context window (in tokens) for a model by name.
///
/// Order: live disk cache from a prior API fetch, else [`UNKNOWN_CONTEXT_WINDOW`].
/// No per-model name table — models change constantly; use the provider API.
#[must_use]
pub fn model_context_window(model: &str) -> usize {
    model_info_from_cache_or_unknown(model, "").context_window
}

/// Build a [`ModelInfo`] from the on-disk cache, or the universal unknown floor.
///
/// Used by sync paths that cannot await a provider fetch (e.g. consolidation
/// sizing). Prefer [`LlmClient::get_model_info`] whenever a client is available.
#[must_use]
pub fn model_info_from_cache_or_unknown(model: &str, provider: &str) -> ModelInfo {
    if let Some(cache) = ModelInfoCache::default_location() {
        if let Some(info) = cache.get(model) {
            return info;
        }
    }
    unknown_model_info(model, provider)
}

/// Conservative model info when the provider has not (yet) told us limits.
///
/// Supports tools optimistically — capability misses are softer than overrunning
/// a too-small context budget on a modern agent path.
#[must_use]
pub fn unknown_model_info(model: &str, provider: &str) -> ModelInfo {
    ModelInfo {
        id: model.to_string(),
        context_window: UNKNOWN_CONTEXT_WINDOW,
        max_output_tokens: UNKNOWN_MAX_OUTPUT_TOKENS,
        supports_tools: true,
        supports_vision: false,
        embedding_dimension: None,
        cached_at: current_timestamp(),
        provider: provider.to_string(),
    }
}

/// Alias kept for local fetch_* fallbacks — one floor, not a model table.
fn default_model_info(model: &str, provider: &str) -> ModelInfo {
    unknown_model_info(model, provider)
}

/// File-based cache for model information
#[derive(Clone)]
pub struct ModelInfoCache {
    cache_dir: std::path::PathBuf,
}

impl ModelInfoCache {
    /// Create a new cache with the specified directory
    pub fn new(cache_dir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            cache_dir: cache_dir.into(),
        }
    }

    /// Create cache in the default location (user's cache directory)
    pub fn default_location() -> Option<Self> {
        directories::ProjectDirs::from("bot", "clawd", "Nanna")
            .map(|dirs| Self::new(dirs.cache_dir().join("model_info")))
    }

    /// Get cached model info if it exists and is not expired
    pub fn get(&self, model: &str) -> Option<ModelInfo> {
        let path = self.cache_path(model);
        if !path.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&path).ok()?;
        let info: ModelInfo = serde_json::from_str(&content).ok()?;

        if info.is_expired() {
            // Remove expired cache
            let _ = std::fs::remove_file(&path);
            debug!(model = %model, "Model info cache expired, removed");
            return None;
        }

        debug!(model = %model, context_window = info.context_window, "Loaded model info from cache");
        Some(info)
    }

    /// Store model info in cache
    pub fn set(&self, info: &ModelInfo) -> Result<(), LlmError> {
        // Ensure cache directory exists
        std::fs::create_dir_all(&self.cache_dir)?;

        let path = self.cache_path(&info.id);
        let content = serde_json::to_string_pretty(info)?;
        std::fs::write(&path, content)?;

        debug!(model = %info.id, path = %path.display(), "Cached model info");
        Ok(())
    }

    /// Clear all cached model info
    pub fn clear(&self) -> Result<(), LlmError> {
        if self.cache_dir.exists() {
            std::fs::remove_dir_all(&self.cache_dir)?;
        }
        Ok(())
    }

    /// Get cache file path for a model
    fn cache_path(&self, model: &str) -> std::path::PathBuf {
        // Sanitize model name for use as filename
        let safe_name: String = model
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        self.cache_dir.join(format!("{safe_name}.json"))
    }
}

impl From<reqwest::Error> for LlmError {
    fn from(e: reqwest::Error) -> Self {
        LlmError::Http(e.to_string())
    }
}

impl From<serde_json::Error> for LlmError {
    fn from(e: serde_json::Error) -> Self {
        LlmError::Json(e.to_string())
    }
}

impl LlmError {
    /// Check if this error is a rate limit error (429)
    #[must_use]
    pub fn is_rate_limit(&self) -> bool {
        matches!(self, LlmError::RateLimit { .. })
            || matches!(self, LlmError::Api { status: 429, .. })
    }

    /// Check if this error should trigger a fallback to another model
    #[must_use]
    pub fn should_fallback(&self) -> bool {
        match self {
            // Rate limits - definitely fallback
            LlmError::RateLimit { .. } => true,
            LlmError::Api { status, message } => {
                // 429 = rate limit
                // 529 = overloaded (Anthropic)
                // 503 = service unavailable
                // 502 = bad gateway
                if *status == 429 || *status == 529 || *status == 503 || *status == 502 {
                    return true;
                }
                // Check for rate limit in error message (some APIs return 400 with rate limit message)
                let msg_lower = message.to_lowercase();
                msg_lower.contains("rate limit") || msg_lower.contains("rate_limit")
            }
            // Network errors - might be transient
            LlmError::Http(_) => true,
            // Don't fallback on auth errors, JSON errors, etc.
            _ => false,
        }
    }

    /// Parse an API error response to extract rate limit info
    pub fn from_api_response(status: u16, message: String) -> Self {
        // Check if it's a rate limit error
        if status == 429 || message.to_lowercase().contains("rate_limit") {
            // Try to extract retry-after from the message
            let retry_after = Self::parse_retry_after(&message);
            return LlmError::RateLimit { message, retry_after };
        }

        LlmError::Api { status, message }
    }

    /// Try to parse retry-after seconds from error message
    fn parse_retry_after(_message: &str) -> Option<u64> {
        // Anthropic includes "try again later" but not specific timing
        // Some APIs include "retry-after: X" in headers or body
        // For now, return None - we can enhance this later
        None
    }
}

/// LLM Provider
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Anthropic,
    OpenAI,
    OpenRouter,
    GitHubModels,
    /// Claude Max/Pro proxy (OpenAI-compatible, uses claude-max-api-proxy)
    ClaudeProxy,
    Ollama,
}

/// Message role
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// Chat message (simple text format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into() }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into() }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into() }
    }
}

impl AnthropicMessage {
    /// Create a tool result message
    pub fn tool_result(tool_use_id: impl Into<String>, content: impl Into<String>, is_error: bool) -> Self {
        Self {
            role: "user".to_string(),
            content: vec![ContentBlock::ToolResult {
                tool_use_id: tool_use_id.into(),
                content: content.into(),
                is_error: if is_error { Some(true) } else { None },
            }],
        }
    }

    /// Create an assistant message with tool use
    pub fn tool_use(id: impl Into<String>, name: impl Into<String>, input: serde_json::Value) -> Self {
        Self {
            role: "assistant".to_string(),
            content: vec![ContentBlock::ToolUse {
                id: id.into(),
                name: name.into(),
                input,
            }],
        }
    }

    /// Create an assistant message with text and tool use
    pub fn assistant_with_tool_use(
        text: Option<String>,
        tool_id: impl Into<String>,
        tool_name: impl Into<String>,
        tool_input: serde_json::Value,
    ) -> Self {
        let mut content = Vec::new();
        if let Some(t) = text {
            if !t.is_empty() {
                content.push(ContentBlock::Text { text: t });
            }
        }
        content.push(ContentBlock::ToolUse {
            id: tool_id.into(),
            name: tool_name.into(),
            input: tool_input,
        });
        Self {
            role: "assistant".to_string(),
            content,
        }
    }
}

/// Anthropic message content block
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    Image { source: ImageSource },
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { tool_use_id: String, content: String, #[serde(skip_serializing_if = "Option::is_none")] is_error: Option<bool> },
    /// Extended thinking content (reasoning)
    Thinking {
        thinking: String,
        /// Signature returned by Anthropic — MUST be sent back in multi-turn conversations.
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
}

/// Image source for vision API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageSource {
    Base64 {
        media_type: String,
        data: String,
    },
    Url {
        url: String,
    },
}

/// Anthropic message format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

impl AnthropicMessage {
    #[must_use]
    pub fn user(content: Vec<ContentBlock>) -> Self {
        Self { role: "user".to_string(), content }
    }

    #[must_use]
    pub fn assistant(content: Vec<ContentBlock>) -> Self {
        Self { role: "assistant".to_string(), content }
    }

    pub fn user_text(text: impl Into<String>) -> Self {
        Self::user(vec![ContentBlock::Text { text: text.into() }])
    }

    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self::assistant(vec![ContentBlock::Text { text: text.into() }])
    }

    /// Create a user message with an image (base64)
    pub fn user_image(media_type: impl Into<String>, base64_data: impl Into<String>, text: Option<String>) -> Self {
        let mut content = vec![ContentBlock::Image {
            source: ImageSource::Base64 {
                media_type: media_type.into(),
                data: base64_data.into(),
            },
        }];
        if let Some(t) = text {
            content.push(ContentBlock::Text { text: t });
        }
        Self::user(content)
    }

    /// Create a user message with text and image
    pub fn user_with_image(text: impl Into<String>, media_type: impl Into<String>, base64_data: impl Into<String>) -> Self {
        Self::user(vec![
            ContentBlock::Text { text: text.into() },
            ContentBlock::Image {
                source: ImageSource::Base64 {
                    media_type: media_type.into(),
                    data: base64_data.into(),
                },
            },
        ])
    }
}

/// Tool definition for Anthropic
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Completion request with tools
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub anthropic_messages: Vec<AnthropicMessage>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stream: bool,
    pub tools: Vec<serde_json::Value>,
    /// Context window limit (for Ollama num_ctx). If set, limits the context
    /// window to reduce VRAM usage and allow more model layers on GPU.
    pub context_limit: Option<u32>,
    /// Extended thinking configuration (passed through to Anthropic API)
    pub thinking: Option<ThinkingConfig>,
}

impl Default for CompletionRequest {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-20250514".to_string(),
            messages: Vec::new(),
            anthropic_messages: Vec::new(),
            max_tokens: Some(4096),
            temperature: Some(0.7),
            stream: false,
            tools: Vec::new(),
            context_limit: None,
            thinking: None,
        }
    }
}

impl CompletionRequest {
    /// Set extended thinking configuration
    #[must_use]
    pub fn with_thinking(mut self, thinking: ThinkingConfig) -> Self {
        self.thinking = Some(thinking);
        self
    }

    /// Add tools to the request (Anthropic format)
    #[must_use]
    pub fn with_tools(mut self, tools: Vec<serde_json::Value>) -> Self {
        self.tools = tools;
        self
    }

    /// Add an Anthropic-format message (for tool use/results)
    #[must_use]
    pub fn with_anthropic_message(mut self, msg: AnthropicMessage) -> Self {
        self.anthropic_messages.push(msg);
        self
    }
}

/// Extended thinking configuration
#[derive(Debug, Clone, Serialize)]
pub struct ThinkingConfig {
    /// Type of thinking (always "enabled" when present)
    #[serde(rename = "type")]
    pub thinking_type: String,
    /// Budget in tokens for thinking
    pub budget_tokens: u32,
}

impl ThinkingConfig {
    /// Create a new thinking config with the given budget
    #[must_use]
    pub fn new(budget_tokens: u32) -> Self {
        Self {
            thinking_type: "enabled".to_string(),
            budget_tokens,
        }
    }
}

/// Full Anthropic request with tools
#[derive(Debug, Clone, Serialize)]
pub struct AnthropicRequest {
    pub model: String,
    pub messages: Vec<AnthropicMessage>,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    /// Extended thinking configuration (Claude 3.5+ models)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingConfig>,
    /// Prompt caching: automatic mode caches the longest prefix up to the last cacheable block.
    /// Set to `Some(CacheControl::ephemeral())` to enable automatic prompt caching.
    /// Cached input tokens cost 90% less on Anthropic, 50% less on OpenAI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

/// Cache control configuration for prompt caching.
///
/// When set at the request level, enables automatic caching: the API caches all content
/// up to and including the last cacheable block. On subsequent requests with the same
/// prefix, cached content is reused automatically (~5 minute TTL on Anthropic).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheControl {
    #[serde(rename = "type")]
    pub cache_type: String,
}

impl CacheControl {
    /// Create an ephemeral cache control (standard ~5 minute TTL)
    #[must_use]
    pub fn ephemeral() -> Self {
        Self {
            cache_type: "ephemeral".to_string(),
        }
    }
}

/// System prompt block (for OAuth/Claude Code format)
#[derive(Debug, Clone, Serialize)]
pub struct SystemBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub text: String,
    /// Cache control for this specific block (enables explicit cache breakpoints).
    /// Place on the last system block to cache the entire system prompt prefix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

impl SystemBlock {
    /// Create a text system block
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            block_type: "text".to_string(),
            text: content.into(),
            cache_control: None,
        }
    }

    /// Create a text system block with ephemeral cache control
    pub fn text_cached(content: impl Into<String>) -> Self {
        Self {
            block_type: "text".to_string(),
            text: content.into(),
            cache_control: Some(CacheControl::ephemeral()),
        }
    }
}

/// Anthropic request with array-based system prompt (required for OAuth/Claude Code)
#[derive(Debug, Clone, Serialize)]
struct OAuthAnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    system: Vec<SystemBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<ThinkingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

impl OAuthAnthropicRequest {
    /// Convert from a standard AnthropicRequest, prepending Claude Code identity
    fn from_request(request: &AnthropicRequest, prepend_identity: bool) -> Self {
        let mut system = Vec::new();

        if prepend_identity {
            system.push(SystemBlock::text(CLAUDE_CODE_IDENTITY));
        }

        if let Some(ref sys) = request.system {
            if !sys.is_empty() {
                system.push(SystemBlock::text(sys));
            }
        }

        // Place explicit cache breakpoint on the last system block.
        // This ensures the system prompt prefix is cached server-side (~5 min TTL).
        if request.cache_control.is_some() {
            if let Some(last) = system.last_mut() {
                last.cache_control = Some(CacheControl::ephemeral());
            }
        }

        Self {
            model: request.model.clone(),
            messages: request.messages.clone(),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            system,
            tools: request.tools.clone(),
            stream: request.stream,
            thinking: request.thinking.clone(),
            cache_control: request.cache_control.clone(),
        }
    }
}

/// Anthropic API response
#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub response_type: String,
    pub role: String,
    pub content: Vec<ContentBlock>,
    pub model: String,
    pub stop_reason: Option<String>,
    pub usage: Usage,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Tokens written to the prompt cache on this request (billed at cache write rate)
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
    /// Tokens read from the prompt cache on this request (billed at 10% of input rate on Anthropic)
    #[serde(default)]
    pub cache_read_input_tokens: u32,
}

// ============================================================================
// Claude Code Stealth Mode
// ============================================================================

/// Claude Code version to mimic (update as needed)
const CLAUDE_CODE_VERSION: &str = "2.1.2";

/// Claude Code canonical tool names (case-sensitive)
/// Source: https://cchistory.mariozechner.at/data/prompts-2.1.11.md
const CLAUDE_CODE_TOOLS: &[(&str, &str)] = &[
    ("read", "Read"),
    ("read_file", "Read"),
    ("write", "Write"),
    ("write_file", "Write"),
    ("edit", "Edit"),
    ("edit_file", "Edit"),
    ("bash", "Bash"),
    ("exec", "Bash"),
    ("shell", "Bash"),
    ("grep", "Grep"),
    ("glob", "Glob"),
    ("list_dir", "Glob"),
    ("ask_user", "AskUserQuestion"),
    ("web_fetch", "WebFetch"),
    ("web_search", "WebSearch"),
    ("notebook_edit", "NotebookEdit"),
];

/// Claude Code identity prefix (REQUIRED for OAuth tokens)
const CLAUDE_CODE_IDENTITY: &str = "You are Claude Code, Anthropic's official CLI for Claude.";

/// Check if a token is an OAuth token (vs API key)
fn is_oauth_token(token: &str) -> bool {
    // OAuth tokens start with sk-ant-oat (OAuth Access Token)
    // Regular API keys start with sk-ant-api
    token.starts_with("sk-ant-oat") || token.starts_with("oauth:")
}

/// Get the raw token (stripping any prefix)
fn get_raw_token(token: &str) -> &str {
    token.strip_prefix("oauth:").unwrap_or(token)
}

/// Convert a tool name to Claude Code canonical form
fn to_claude_code_tool_name(name: &str) -> String {
    let lower = name.to_lowercase();
    for (pattern, canonical) in CLAUDE_CODE_TOOLS {
        if lower == *pattern {
            return (*canonical).to_string();
        }
    }
    // If no match, return original (with first letter capitalized for consistency)
    let mut chars = name.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().chain(chars).collect(),
    }
}

/// Convert a tool name from Claude Code form back to original
fn from_claude_code_tool_name(name: &str, original_tools: &[ToolDefinition]) -> String {
    let lower = name.to_lowercase();
    // Find original tool by matching lowercase
    for tool in original_tools {
        if tool.name.to_lowercase() == lower {
            return tool.name.clone();
        }
    }
    // Check Claude Code canonical names and map back
    for (pattern, canonical) in CLAUDE_CODE_TOOLS {
        if name == *canonical {
            // Return the pattern as-is (caller should have original)
            return (*pattern).to_string();
        }
    }
    name.to_string()
}

/// LLM client
#[derive(Clone)]
pub struct LlmClient {
    http: Client,
    provider: Provider,
    api_key: String,
    base_url: String,
}

impl LlmClient {
    /// Build HTTP client with sensible timeouts
    fn build_http_client() -> Client {
        Client::builder()
            .timeout(std::time::Duration::from_secs(120))  // 2 min total timeout
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| Client::new())
    }

    /// Create a new Anthropic client with API key
    pub fn anthropic(api_key: impl Into<String>) -> Self {
        Self {
            http: Self::build_http_client(),
            provider: Provider::Anthropic,
            api_key: api_key.into(),
            base_url: "https://api.anthropic.com".to_string(),
        }
    }

    /// Create a new Anthropic client with OAuth token
    /// Uses Authorization: Bearer header instead of x-api-key
    pub fn anthropic_oauth(oauth_token: impl Into<String>) -> Self {
        Self {
            http: Self::build_http_client(),
            provider: Provider::Anthropic,
            api_key: format!("oauth:{}", oauth_token.into()), // Prefix to indicate OAuth
            base_url: "https://api.anthropic.com".to_string(),
        }
    }

    /// Create a new `OpenAI` client
    pub fn openai(api_key: impl Into<String>) -> Self {
        Self {
            http: Self::build_http_client(),
            provider: Provider::OpenAI,
            api_key: api_key.into(),
            base_url: "https://api.openai.com".to_string(),
        }
    }

    /// Create a new `OpenRouter` client
    pub fn openrouter(api_key: impl Into<String>) -> Self {
        Self {
            http: Self::build_http_client(),
            provider: Provider::OpenRouter,
            api_key: api_key.into(),
            base_url: "https://openrouter.ai/api".to_string(),
        }
    }

    /// Create a new `GitHub Models` client (uses GitHub PAT)
    /// GitHub Models provides access to various LLMs including GPT-4o, Llama, Mistral, etc.
    pub fn github_models(api_key: impl Into<String>) -> Self {
        Self {
            http: Self::build_http_client(),
            provider: Provider::GitHubModels,
            api_key: api_key.into(),
            base_url: "https://models.inference.ai.azure.com".to_string(),
        }
    }

    /// Create a new `Claude Proxy` client for claude-max-api-proxy
    /// This proxies requests through a local server that uses Claude Code CLI credentials
    /// Default URL is http://localhost:3456
    pub fn claude_proxy(base_url: impl Into<String>) -> Self {
        Self {
            http: Self::build_http_client(),
            provider: Provider::ClaudeProxy,
            api_key: String::new(), // Proxy handles auth via Claude Code CLI
            base_url: base_url.into(),
        }
    }

    /// Create a Claude Proxy client with default localhost URL
    pub fn claude_proxy_default() -> Self {
        Self::claude_proxy("http://localhost:3456")
    }

    /// Create a new `Ollama` client (local, no API key needed)
    pub fn ollama(base_url: impl Into<String>) -> Self {
        Self {
            http: Self::build_http_client(),
            provider: Provider::Ollama,
            api_key: String::new(), // Ollama doesn't need auth
            base_url: base_url.into(),
        }
    }

    /// Create an Ollama client with an API key (for remote/authenticated instances)
    pub fn ollama_with_key(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            http: Self::build_http_client(),
            provider: Provider::Ollama,
            api_key: api_key.into(),
            base_url: base_url.into(),
        }
    }

    /// Create an Ollama client with default localhost URL
    pub fn ollama_default() -> Self {
        Self::ollama("http://localhost:11434")
    }

    /// Check if this client uses OAuth authentication
    fn is_oauth(&self) -> bool {
        is_oauth_token(&self.api_key)
    }

    /// Get the actual token (stripping oauth: prefix if present)
    fn get_token(&self) -> &str {
        get_raw_token(&self.api_key)
    }

    /// Apply Anthropic authentication headers to a request builder
    /// For OAuth, includes required beta headers and user-agent (matching Claude Code exactly)
    fn apply_anthropic_auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.is_oauth() {
            // Stealth mode: Mimic Claude Code's headers exactly
            // Beta features: claude-code identity, oauth, interleaved thinking, fine-grained tool streaming
            let beta = "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14";
            let user_agent = format!("claude-cli/{CLAUDE_CODE_VERSION} (external, cli)");
            debug!(
                beta = %beta,
                user_agent = %user_agent,
                "OAuth stealth mode: applying Claude Code headers"
            );
            builder
                .header("Authorization", format!("Bearer {}", self.get_token()))
                .header("accept", "application/json")
                .header("anthropic-beta", beta)
                .header("user-agent", user_agent)
                .header("x-app", "cli")
                .header("anthropic-dangerous-direct-browser-access", "true")
        } else {
            builder.header("x-api-key", &self.api_key)
        }
    }

    /// Validate the API key by making a lightweight request.
    ///
    /// Apply Ollama auth header if an API key is configured.
    fn apply_ollama_auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if !self.api_key.is_empty() {
            builder.header("Authorization", format!("Bearer {}", self.api_key))
        } else {
            builder
        }
    }

    /// # Errors
    ///
    /// Returns `LlmError::Api` with status 401 if the API key is invalid.
    /// Returns `LlmError::Network` if the request fails.
    pub async fn validate(&self) -> Result<(), LlmError> {
        match self.provider {
            Provider::Anthropic => {
                // Use count_tokens endpoint for validation - lightweight
                let request_builder = self
                    .http
                    .post(format!("{}/v1/messages/count_tokens", self.base_url));
                let response = self.apply_anthropic_auth(request_builder)
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json")
                    .json(&serde_json::json!({
                        "model": "claude-sonnet-4-20250514",
                        "messages": [{"role": "user", "content": "hi"}]
                    }))
                    .send()
                    .await?;

                if response.status().as_u16() == 401 {
                    return Err(LlmError::Api {
                        status: 401,
                        message: "Invalid API key or OAuth token".to_string(),
                    });
                }
                Ok(())
            }
            _ => Ok(()), // Skip validation for other providers for now
        }
    }

    /// Get the provider for this client
    #[must_use]
    pub fn provider(&self) -> Provider {
        self.provider
    }

    /// Get model information with caching.
    ///
    /// Checks the file cache first, then fetches from the API if needed.
    /// Falls back to defaults if the API doesn't provide context window info.
    ///
    /// # Errors
    ///
    /// Returns `LlmError` if both cache and API fail (uses defaults as final fallback).
    pub async fn get_model_info(&self, model: &str, cache: Option<&ModelInfoCache>) -> ModelInfo {
        // Check cache first
        if let Some(cache) = cache {
            if let Some(info) = cache.get(model) {
                return info;
            }
        }

        // Fetch from API
        let info = match self.fetch_model_info(model).await {
            Ok(info) => info,
            Err(e) => {
                warn!(model = %model, error = %e, "Failed to fetch model info, using defaults");
                default_model_info(model, &format!("{:?}", self.provider))
            }
        };

        // Cache the result
        if let Some(cache) = cache {
            if let Err(e) = cache.set(&info) {
                warn!(model = %model, error = %e, "Failed to cache model info");
            }
        }

        info
    }

    /// Force refresh model info from API (bypasses cache).
    ///
    /// # Errors
    ///
    /// Returns `LlmError` if the API request fails.
    pub async fn refresh_model_info(&self, model: &str, cache: Option<&ModelInfoCache>) -> Result<ModelInfo, LlmError> {
        let info = self.fetch_model_info(model).await?;

        if let Some(cache) = cache {
            cache.set(&info)?;
        }

        Ok(info)
    }

    /// Fetch model info from the provider API
    async fn fetch_model_info(&self, model: &str) -> Result<ModelInfo, LlmError> {
        match self.provider {
            Provider::Anthropic => self.fetch_anthropic_model_info(model).await,
            Provider::OpenAI => self.fetch_openai_model_info(model).await,
            Provider::Ollama => self.fetch_ollama_model_info(model).await,
            Provider::OpenRouter => self.fetch_openrouter_model_info(model).await,
            // For proxies and GitHub Models, use defaults
            Provider::ClaudeProxy | Provider::GitHubModels => {
                Ok(default_model_info(model, &format!("{:?}", self.provider)))
            }
        }
    }

    /// Fetch model info from Anthropic API
    async fn fetch_anthropic_model_info(&self, model: &str) -> Result<ModelInfo, LlmError> {
        // Anthropic's /v1/models/{model_id} endpoint
        let url = format!("{}/v1/models/{}", self.base_url, model);

        let request_builder = self.http.get(&url);
        let response = self
            .apply_anthropic_auth(request_builder)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            // If model not found or API doesn't support this, use defaults
            if status == 404 {
                info!(model = %model, "Model not found in API, using defaults");
                return Ok(default_model_info(model, "anthropic"));
            }
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::Api { status, message });
        }

        // Parse response - Anthropic may or may not include context_window
        #[derive(Deserialize)]
        struct AnthropicModelResponse {
            id: String,
            #[serde(default)]
            context_window: Option<usize>,
            #[serde(default)]
            max_output_tokens: Option<usize>,
        }

        let api_response: AnthropicModelResponse = response.json().await?;

        // Use API values if available, otherwise use defaults
        let defaults = default_model_info(model, "anthropic");
        Ok(ModelInfo {
            id: api_response.id,
            context_window: api_response.context_window.unwrap_or(defaults.context_window),
            max_output_tokens: api_response.max_output_tokens.unwrap_or(defaults.max_output_tokens),
            supports_tools: defaults.supports_tools,
            supports_vision: defaults.supports_vision,
            embedding_dimension: None, // Anthropic doesn't provide embedding models via this API
            cached_at: current_timestamp(),
            provider: "anthropic".to_string(),
        })
    }

    /// Fetch model info from OpenAI API
    async fn fetch_openai_model_info(&self, model: &str) -> Result<ModelInfo, LlmError> {
        let url = format!("{}/v1/models/{}", self.base_url, model);

        let response = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            if status == 404 {
                return Ok(default_model_info(model, "openai"));
            }
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::Api { status, message });
        }

        // OpenAI doesn't return context_window in their model endpoint
        // So we use defaults with the model ID from the API
        #[derive(Deserialize)]
        struct OpenAIModelResponse {
            id: String,
        }

        let api_response: OpenAIModelResponse = response.json().await?;
        Ok(default_model_info(&api_response.id, "openai").with_timestamp())
    }

    /// Fetch model info from Ollama API
    async fn fetch_ollama_model_info(&self, model: &str) -> Result<ModelInfo, LlmError> {
        let url = format!("{}/api/show", self.base_url);

        let response = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "name": model }))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            if status == 404 {
                return Ok(default_model_info(model, "ollama"));
            }
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::Api { status, message });
        }

        // Ollama returns model info including context length and embedding dimension
        #[derive(Deserialize)]
        struct OllamaShowResponse {
            #[serde(default)]
            model_info: Option<OllamaModelInfo>,
        }

        #[derive(Deserialize)]
        struct OllamaModelInfo {
            #[serde(rename = "general.context_length", default)]
            context_length: Option<usize>,
            /// Embedding dimension (for embedding models)
            #[serde(rename = "general.embedding_length", default)]
            embedding_length: Option<usize>,
        }

        let api_response: OllamaShowResponse = response.json().await?;

        // Use the model metadata verbatim. In particular, do not silently raise
        // a local model's configured context window above what Ollama reports.
        let defaults = default_model_info(model, "ollama");
        let (context_window, embedding_dimension) = match api_response.model_info {
            Some(info) => (
                info.context_length.unwrap_or(defaults.context_window),
                info.embedding_length,
            ),
            None => (defaults.context_window, None),
        };

        Ok(ModelInfo {
            id: model.to_string(),
            context_window,
            // Ollama does not publish a distinct output cap. Derive the safe
            // generation budget from this model's reported total window.
            max_output_tokens: context_window / 2,
            supports_tools: true, // Ollama v0.5+ supports tool calling
            supports_vision: false,
            embedding_dimension,
            cached_at: current_timestamp(),
            provider: "ollama".to_string(),
        })
    }

    /// Fetch model info from OpenRouter API
    async fn fetch_openrouter_model_info(&self, model: &str) -> Result<ModelInfo, LlmError> {
        // OpenRouter has a models endpoint that lists all models
        let url = format!("{}/v1/models", self.base_url);

        let response = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;

        if !response.status().is_success() {
            return Ok(default_model_info(model, "openrouter"));
        }

        #[derive(Deserialize)]
        struct OpenRouterModelsResponse {
            data: Vec<OpenRouterModel>,
        }

        #[derive(Deserialize)]
        struct OpenRouterModel {
            id: String,
            #[serde(default)]
            context_length: Option<usize>,
            #[serde(default)]
            top_provider: Option<OpenRouterTopProvider>,
        }

        #[derive(Deserialize)]
        struct OpenRouterTopProvider {
            #[serde(default)]
            max_completion_tokens: Option<usize>,
        }

        let api_response: OpenRouterModelsResponse = response.json().await?;

        // Find our model in the list
        if let Some(m) = api_response.data.iter().find(|m| m.id == model) {
            return Ok(ModelInfo {
                id: m.id.clone(),
                context_window: m.context_length.unwrap_or(UNKNOWN_CONTEXT_WINDOW),
                max_output_tokens: m
                    .top_provider
                    .as_ref()
                    .and_then(|p| p.max_completion_tokens)
                    .unwrap_or_else(|| m.context_length.unwrap_or(UNKNOWN_CONTEXT_WINDOW) / 2),
                supports_tools: true, // OpenRouter usually routes to capable models
                supports_vision: false,
                embedding_dimension: None,
                cached_at: current_timestamp(),
                provider: "openrouter".to_string(),
            });
        }

        Ok(default_model_info(model, "openrouter"))
    }

    /// Send a completion request (simple, no tools).
    ///
    /// # Errors
    ///
    /// Returns `LlmError::Api` if the API returns an error.
    /// Returns `LlmError::Network` if the request fails.
    pub async fn complete(&self, request: &CompletionRequest) -> Result<String, LlmError> {
        match self.provider {
            Provider::Anthropic => self.complete_anthropic_simple(request).await,
            Provider::OpenAI | Provider::OpenRouter | Provider::GitHubModels | Provider::ClaudeProxy => self.complete_openai(request).await,
            Provider::Ollama => self.complete_ollama(request).await,
        }
    }

    /// Send a full Anthropic-format request with tools.
    ///
    /// Dispatches to the appropriate provider backend:
    /// - Anthropic: Native Anthropic API (`/v1/messages`)
    /// - OpenAI/OpenRouter/GitHub/ClaudeProxy: Converts to OpenAI chat completions format
    /// - Ollama: Converts to Ollama `/api/chat` format
    ///
    /// # Errors
    ///
    /// Returns `LlmError::Api` if the API returns an error.
    /// Returns `LlmError::Network` if the request fails.
    pub async fn complete_anthropic(&self, request: &AnthropicRequest) -> Result<AnthropicResponse, LlmError> {
        match self.provider {
            Provider::Anthropic => self.complete_anthropic_native(request).await,
            Provider::OpenAI | Provider::OpenRouter | Provider::GitHubModels | Provider::ClaudeProxy =>
                self.complete_anthropic_via_openai(request).await,
            Provider::Ollama =>
                self.complete_anthropic_via_ollama(request).await,
        }
    }

    /// Native Anthropic API implementation for complete_anthropic
    async fn complete_anthropic_native(&self, request: &AnthropicRequest) -> Result<AnthropicResponse, LlmError> {
        let is_oauth = self.is_oauth();

        // Store original tools for reverse mapping
        let original_tools = request.tools.clone().unwrap_or_default();

        let request_builder = self
            .http
            .post(format!("{}/v1/messages", self.base_url));

        let response = if is_oauth {
            // OAuth mode: Use array-based system prompt and remap tool names
            let mut oauth_request = OAuthAnthropicRequest::from_request(request, true);

            // Remap tool names to Claude Code format
            if let Some(ref mut tools) = oauth_request.tools {
                for tool in tools {
                    tool.name = to_claude_code_tool_name(&tool.name);
                }
            }

            debug!(
                system_blocks = oauth_request.system.len(),
                tool_names = ?oauth_request.tools.as_ref().map(|t| t.iter().map(|x| x.name.as_str()).collect::<Vec<_>>()),
                "OAuth request prepared with array-based system prompt"
            );

            self.apply_anthropic_auth(request_builder)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&oauth_request)
                .send()
                .await?
        } else {
            // Standard mode: Use string-based system prompt
            self.apply_anthropic_auth(request_builder)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(request)
                .send()
                .await?
        };

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::from_api_response(status, message));
        }

        let mut result: AnthropicResponse = response.json().await?;

        // Log prompt cache usage
        if result.usage.cache_read_input_tokens > 0 || result.usage.cache_creation_input_tokens > 0 {
            info!(
                cache_read = result.usage.cache_read_input_tokens,
                cache_write = result.usage.cache_creation_input_tokens,
                input = result.usage.input_tokens,
                "📦 Prompt cache: read {} tokens, wrote {} tokens",
                result.usage.cache_read_input_tokens,
                result.usage.cache_creation_input_tokens,
            );
        }

        // Remap tool names back in response
        if is_oauth {
            for block in &mut result.content {
                if let ContentBlock::ToolUse { name, .. } = block {
                    *name = from_claude_code_tool_name(name, &original_tools);
                }
            }
        }

        Ok(result)
    }

    /// Convert AnthropicRequest → OpenAI chat completions and execute
    async fn complete_anthropic_via_openai(&self, request: &AnthropicRequest) -> Result<AnthropicResponse, LlmError> {
        let (messages_json, tools_json) = anthropic_to_openai_request(request);

        let mut body = serde_json::json!({
            "model": request.model,
            "messages": messages_json,
            "max_tokens": request.max_tokens,
        });
        if let Some(temp) = request.temperature {
            body["temperature"] = serde_json::json!(temp);
        }
        if let Some(tools) = tools_json {
            body["tools"] = tools;
        }

        let response = self
            .http
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::from_api_response(status, message));
        }

        let result: serde_json::Value = response.json().await?;
        let response = openai_response_to_anthropic(&request.model, &result)?;

        // Log OpenAI prompt cache usage (automatic for prompts >1024 tokens, 50% discount)
        if response.usage.cache_read_input_tokens > 0 {
            info!(
                cache_read = response.usage.cache_read_input_tokens,
                input = response.usage.input_tokens,
                "📦 OpenAI prompt cache: read {} cached tokens (50% discount)",
                response.usage.cache_read_input_tokens,
            );
        }

        Ok(response)
    }

    /// Convert AnthropicRequest → Ollama /api/chat and execute
    async fn complete_anthropic_via_ollama(&self, request: &AnthropicRequest) -> Result<AnthropicResponse, LlmError> {
        let (messages_json, tools_json) = anthropic_to_ollama_request(request);

        let mut body = serde_json::json!({
            "model": request.model,
            "messages": messages_json,
            "stream": false,
        });

        // Ollama uses "options" for parameters
        let mut options = serde_json::Map::new();
        if let Some(temp) = request.temperature {
            options.insert("temperature".to_string(), serde_json::json!(temp));
        }
        // num_predict=-1 means unlimited output — lets thinking models (qwen3)
        // use as many tokens as needed for reasoning without starving the response.
        options.insert("num_predict".to_string(), serde_json::json!(-1));
        // Request 32K context so the agent has room for system prompt + conversation.
        // Ollama models default to small contexts (e.g. 8K) even if they support more.
        options.insert("num_ctx".to_string(), serde_json::json!(32768));
        if !options.is_empty() {
            body["options"] = serde_json::Value::Object(options);
        }

        // Ollama supports tools since v0.5
        if let Some(tools) = tools_json {
            body["tools"] = tools;
        }

        // Enable thinking separation for models that support it (qwen3, deepseek-r1, etc.)
        // This makes Ollama return thinking in a separate `thinking` field instead of
        // embedding <think>...</think> tags inside `content`.
        let model_lower = request.model.to_lowercase();
        if model_lower.contains("qwen3")
            || model_lower.contains("deepseek-r1")
            || model_lower.contains("qwq")
        {
            body["think"] = serde_json::json!(true);
        }

        let response = self.apply_ollama_auth(self
            .http
            .post(format!("{}/api/chat", self.base_url))
            .header("content-type", "application/json"))
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::from_api_response(status, message));
        }

        let result: serde_json::Value = response.json().await?;
        ollama_response_to_anthropic(&request.model, &result)
    }

    async fn complete_anthropic_simple(&self, request: &CompletionRequest) -> Result<String, LlmError> {
        // Extract system message
        let system_msg = request
            .messages
            .iter()
            .find(|m| m.role == Role::System)
            .map(|m| m.content.clone());

        // Convert to Anthropic format
        let messages: Vec<AnthropicMessage> = request
            .messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| AnthropicMessage {
                role: match m.role {
                    Role::User | Role::System => "user", // System filtered above
                    Role::Assistant => "assistant",
                }.to_string(),
                content: vec![ContentBlock::Text { text: m.content.clone() }],
            })
            .collect();

        let anthropic_request = AnthropicRequest {
            model: request.model.clone(),
            messages,
            max_tokens: request.max_tokens.unwrap_or(4096),
            temperature: request.temperature,
            system: system_msg,
            tools: None,
            stream: None,
            thinking: None,
            cache_control: None,
        };

        let result = self.complete_anthropic(&anthropic_request).await?;

        // Extract text from response
        let text = result
            .content
            .iter()
            .filter_map(|c| match c {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        Ok(text)
    }

    async fn complete_openai(&self, request: &CompletionRequest) -> Result<String, LlmError> {
        #[derive(Serialize)]
        struct OpenAIRequest<'a> {
            model: &'a str,
            messages: &'a [Message],
            #[serde(skip_serializing_if = "Option::is_none")]
            max_tokens: Option<u32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            temperature: Option<f32>,
        }

        #[derive(Deserialize)]
        struct OpenAIResponse {
            choices: Vec<Choice>,
        }

        #[derive(Deserialize)]
        struct Choice {
            message: Message,
        }

        let body = OpenAIRequest {
            model: &request.model,
            messages: &request.messages,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
        };

        let response = self
            .http
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::from_api_response(status, message));
        }

        let result: OpenAIResponse = response.json().await?;
        Ok(result
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default())
    }

    async fn complete_ollama(&self, request: &CompletionRequest) -> Result<String, LlmError> {
        // Ollama uses a slightly different format from OpenAI
        #[derive(Serialize)]
        struct OllamaMessage<'a> {
            role: &'a str,
            content: &'a str,
        }

        #[derive(Serialize)]
        struct OllamaRequest<'a> {
            model: &'a str,
            messages: Vec<OllamaMessage<'a>>,
            stream: bool,
            #[serde(skip_serializing_if = "Option::is_none")]
            options: Option<OllamaOptions>,
        }

        #[derive(Serialize)]
        struct OllamaOptions {
            #[serde(skip_serializing_if = "Option::is_none")]
            temperature: Option<f32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            num_predict: Option<u32>,
            /// Context window size - limits KV cache to reduce VRAM usage
            #[serde(skip_serializing_if = "Option::is_none")]
            num_ctx: Option<u32>,
        }

        #[derive(Deserialize)]
        struct OllamaResponse {
            message: OllamaResponseMessage,
        }

        #[derive(Deserialize)]
        struct OllamaResponseMessage {
            content: String,
        }

        let messages: Vec<OllamaMessage> = request
            .messages
            .iter()
            .map(|m| OllamaMessage {
                role: match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                },
                content: &m.content,
            })
            .collect();

        let options = if request.temperature.is_some() || request.max_tokens.is_some() || request.context_limit.is_some() {
            Some(OllamaOptions {
                temperature: request.temperature,
                num_predict: request.max_tokens,
                num_ctx: request.context_limit,
            })
        } else {
            None
        };

        let body = OllamaRequest {
            model: &request.model,
            messages,
            stream: false,
            options,
        };

        let response = self.apply_ollama_auth(self
            .http
            .post(format!("{}/api/chat", self.base_url))
            .header("content-type", "application/json"))
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::from_api_response(status, message));
        }

        let result: OllamaResponse = response.json().await?;
        Ok(result.message.content)
    }
}

// ============================================================================
// Token Estimation
// ============================================================================

/// Per-message framing overhead (role markers + delimiters), in tokens.
///
/// Both Anthropic and `OpenAI` wrap each message in a few control tokens
/// (`<|im_start|>role … <|im_end|>` and equivalents). Ignoring it under-counts
/// long conversations by several hundred tokens.
pub const MESSAGE_FRAMING_TOKENS: usize = 4;

/// Approximate char/token ratios used by the heuristic estimator.
///
/// These replace the old blanket `len()/4`:
/// - English/prose ASCII: ~4 chars/token
/// - Source code / JSON / identifiers: denser ~3.5 chars/token
/// - CJK / wide scripts: ~1 token per character
pub const CHARS_PER_TOKEN_ENGLISH: f32 = 4.0;
pub const CHARS_PER_TOKEN_CODE: f32 = 3.5;
/// Wide-script (CJK etc.) density, tokens per character (not chars/token).
pub const TOKENS_PER_WIDE_CHAR: f32 = 1.0;

/// Content family used to pick a char/token multiplier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenContentFamily {
    /// Natural-language English / Latin prose.
    English,
    /// Source code, JSON, XML, identifiers, punctuation-heavy text.
    Code,
    /// Auto-detect from content (default).
    Auto,
}

/// Estimate token count for a string using the family-aware heuristic.
///
/// Character-class aware: ASCII/Latin text tokenizes at roughly 4 chars per
/// token (English) or ~3.5 for code-like content, while CJK and other non-ASCII
/// scripts are far denser. The old `bytes / 4` heuristic badly under-counted CJK
/// — a single CJK char is 3 UTF-8 bytes yet ~1 token — which caused real context
/// overflows. Auto mode blends English/code ratios based on a simple
/// punctuation/identifier density scan.
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    estimate_tokens_for_family(text, TokenContentFamily::Auto)
}

/// Estimate tokens with an explicit content family.
#[must_use]
pub fn estimate_tokens_for_family(text: &str, family: TokenContentFamily) -> usize {
    if text.is_empty() {
        return 0;
    }

    let mut ascii_chars = 0usize;
    let mut wide_chars = 0usize;
    let mut code_markers = 0usize;

    for ch in text.chars() {
        if ch.is_ascii() {
            ascii_chars += 1;
            // Common code/json/identifier density signals.
            if matches!(
                ch,
                '{' | '}' | '[' | ']' | '(' | ')' | ';' | ':' | '_' | '<' | '>' | '=' | '|'
                    | '&' | '*' | '/' | '\\' | '#' | '@' | '$' | '`'
            ) {
                code_markers += 1;
            }
        } else {
            wide_chars += 1;
        }
    }

    let ratio = match family {
        TokenContentFamily::English => CHARS_PER_TOKEN_ENGLISH,
        TokenContentFamily::Code => CHARS_PER_TOKEN_CODE,
        TokenContentFamily::Auto => {
            // If ≥8% of ASCII chars are code-ish punctuation, treat as code.
            if ascii_chars > 0 && (code_markers.saturating_mul(100) / ascii_chars) >= 8 {
                CHARS_PER_TOKEN_CODE
            } else {
                CHARS_PER_TOKEN_ENGLISH
            }
        }
    };

    // Round the ASCII share up so short strings never estimate to zero.
    let ascii_tokens = if ascii_chars == 0 {
        0
    } else {
        // ceil(ascii_chars / ratio) without float hacks in the hot edge cases.
        ((ascii_chars as f32) / ratio).ceil() as usize
    };
    let wide_tokens = (wide_chars as f32 * TOKENS_PER_WIDE_CHAR).ceil() as usize;
    // At least 1 token for non-empty input.
    (ascii_tokens + wide_tokens).max(1)
}

/// Exact OpenAI BPE tokenization via `tiktoken-rs` when the `tiktoken` feature
/// is enabled. Falls back to the heuristic when tokenization fails or the
/// feature is off.
///
/// - prefers `o200k_base` (GPT-4o / GPT-5 / o-series) because it is the modern
///   default encoding;
/// - falls back to `cl100k_base` (GPT-4 / GPT-3.5) if o200k cannot init.
#[must_use]
pub fn estimate_tokens_exact(text: &str) -> usize {
    #[cfg(feature = "tiktoken")]
    {
        if let Some(n) = tiktoken_count(text) {
            return n;
        }
    }
    estimate_tokens(text)
}

/// Exact count with an explicit model hint (e.g. `"gpt-4o"`, `"gpt-4"`,
/// `"o3-mini"`). When the feature is off, or the model is unknown, falls back
/// to [`estimate_tokens`].
#[must_use]
pub fn estimate_tokens_for_model(text: &str, model: &str) -> usize {
    #[cfg(feature = "tiktoken")]
    {
        if let Some(n) = tiktoken_count_for_model(text, model) {
            return n;
        }
    }
    let _ = model;
    estimate_tokens(text)
}

#[cfg(feature = "tiktoken")]
fn tiktoken_count(text: &str) -> Option<usize> {
    // Prefer modern o200k (GPT-4o / GPT-5 / o*), then cl100k.
    if let Ok(bpe) = tiktoken_rs::o200k_base() {
        return Some(bpe.encode_with_special_tokens(text).len());
    }
    if let Ok(bpe) = tiktoken_rs::cl100k_base() {
        return Some(bpe.encode_with_special_tokens(text).len());
    }
    None
}

#[cfg(feature = "tiktoken")]
fn tiktoken_count_for_model(text: &str, model: &str) -> Option<usize> {
    // bpe_for_model is the preferred API in 0.12; falls through on unknown names.
    if let Ok(bpe) = tiktoken_rs::bpe_for_model(model) {
        return Some(bpe.encode_with_special_tokens(text).len());
    }
    // Heuristic family pick from the model name string.
    let lower = model.to_ascii_lowercase();
    let o200k = lower.contains("gpt-4o")
        || lower.contains("gpt-5")
        || lower.contains("gpt-4.1")
        || lower.starts_with('o')
        || lower.contains("codex")
        || lower.contains("chatgpt");
    if o200k {
        if let Ok(bpe) = tiktoken_rs::o200k_base() {
            return Some(bpe.encode_with_special_tokens(text).len());
        }
    }
    if let Ok(bpe) = tiktoken_rs::cl100k_base() {
        return Some(bpe.encode_with_special_tokens(text).len());
    }
    None
}

/// Estimate total tokens for a completion request
#[must_use]
pub fn estimate_request_tokens(request: &CompletionRequest) -> usize {
    let mut total = 0;

    // Simple messages (each carries role + delimiter framing).
    for msg in &request.messages {
        total += estimate_tokens(&msg.content) + MESSAGE_FRAMING_TOKENS;
    }

    // Anthropic messages (can have multiple content blocks)
    for msg in &request.anthropic_messages {
        total += MESSAGE_FRAMING_TOKENS;
        for block in &msg.content {
            match block {
                ContentBlock::Text { text } => total += estimate_tokens(text),
                ContentBlock::ToolUse { input, .. } => {
                    // Tool inputs are code/JSON-shaped.
                    total += estimate_tokens_for_family(
                        &input.to_string(),
                        TokenContentFamily::Code,
                    );
                }
                ContentBlock::ToolResult { content, .. } => {
                    total += estimate_tokens(content);
                }
                ContentBlock::Image { .. } => {
                    // Images are ~1000 tokens for small, more for large
                    total += 1000;
                }
                ContentBlock::Thinking { thinking, .. } => {
                    total += estimate_tokens(thinking);
                }
            }
        }
    }

    // Tools definitions
    for tool in &request.tools {
        total += estimate_tokens_for_family(&tool.to_string(), TokenContentFamily::Code);
    }

    total
}

/// Model rate limit info (tokens per minute)
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ModelLimits {
    pub input_tokens_per_minute: u32,
    pub output_tokens_per_minute: u32,
    /// Requests per minute (if known)
    pub requests_per_minute: Option<u32>,
    /// Retry-after seconds from last 429 (if any)
    pub retry_after_secs: Option<u64>,
}

impl Default for ModelLimits {
    fn default() -> Self {
        Self {
            input_tokens_per_minute: 20_000,
            output_tokens_per_minute: 8_000,
            requests_per_minute: None,
            retry_after_secs: None,
        }
    }
}

/// Rate limit info parsed from API response headers
#[derive(Debug, Clone)]
pub struct RateLimitHeaders {
    /// Tokens per minute limit
    pub limit_tokens: Option<u32>,
    /// Remaining tokens this minute
    pub remaining_tokens: Option<u32>,
    /// Requests per minute limit
    pub limit_requests: Option<u32>,
    /// Remaining requests this minute
    pub remaining_requests: Option<u32>,
    /// Seconds until limit resets
    pub reset_tokens_secs: Option<u64>,
    pub reset_requests_secs: Option<u64>,
    /// Retry-after (from 429 response)
    pub retry_after: Option<u64>,
}

impl RateLimitHeaders {
    /// Parse rate limit headers from an HTTP response.
    /// Handles both Anthropic and OpenAI header formats.
    #[must_use]
    pub fn from_headers(headers: &reqwest::header::HeaderMap) -> Self {
        let get_u32 = |name: &str| -> Option<u32> {
            headers.get(name)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse().ok())
        };
        let get_u64 = |name: &str| -> Option<u64> {
            headers.get(name)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse().ok())
        };

        Self {
            // Anthropic uses x-ratelimit-limit-tokens, x-ratelimit-remaining-tokens
            // OpenAI uses x-ratelimit-limit-tokens, x-ratelimit-remaining-tokens (same!)
            limit_tokens: get_u32("x-ratelimit-limit-tokens"),
            remaining_tokens: get_u32("x-ratelimit-remaining-tokens"),
            limit_requests: get_u32("x-ratelimit-limit-requests"),
            remaining_requests: get_u32("x-ratelimit-remaining-requests"),
            reset_tokens_secs: get_u64("x-ratelimit-reset-tokens"),
            reset_requests_secs: get_u64("x-ratelimit-reset-requests"),
            retry_after: get_u64("retry-after"),
        }
    }

    /// Convert to ModelLimits, using defaults where headers are missing
    #[must_use]
    pub fn to_model_limits(&self, defaults: &ModelLimits) -> ModelLimits {
        ModelLimits {
            input_tokens_per_minute: self.limit_tokens.unwrap_or(defaults.input_tokens_per_minute),
            output_tokens_per_minute: defaults.output_tokens_per_minute, // Usually not in headers
            requests_per_minute: self.limit_requests.or(defaults.requests_per_minute),
            retry_after_secs: self.retry_after.or(defaults.retry_after_secs),
        }
    }
}

impl ModelLimits {
    /// Get default limits for common models.
    /// These are conservative fallbacks — actual limits come from API headers.
    ///
    /// **Note:** These are baseline Tier 1 limits. Higher tiers get much more.
    /// The system will update these from response headers at runtime.
    #[must_use]
    pub fn for_model(model: &str) -> Self {
        match model {
            // Anthropic - Tier 1 baseline (very conservative)
            // See https://docs.anthropic.com/en/api/rate-limits
            m if m.contains("opus") => Self {
                input_tokens_per_minute: 20_000,   // Tier 1 baseline
                output_tokens_per_minute: 4_000,
                requests_per_minute: Some(50),
                retry_after_secs: None,
            },
            m if m.contains("sonnet") => Self {
                input_tokens_per_minute: 40_000,   // Tier 1 baseline
                output_tokens_per_minute: 8_000,
                requests_per_minute: Some(50),
                retry_after_secs: None,
            },
            m if m.contains("haiku") => Self {
                input_tokens_per_minute: 50_000,   // Tier 1 baseline
                output_tokens_per_minute: 10_000,
                requests_per_minute: Some(50),
                retry_after_secs: None,
            },
            // OpenAI - Tier 1 baseline
            m if m.contains("gpt-4o") => Self {
                input_tokens_per_minute: 30_000,
                output_tokens_per_minute: 10_000,
                requests_per_minute: Some(500),
                retry_after_secs: None,
            },
            m if m.contains("gpt-4") => Self {
                input_tokens_per_minute: 10_000,
                output_tokens_per_minute: 10_000,
                requests_per_minute: Some(500),
                retry_after_secs: None,
            },
            m if m.contains("gpt-3.5") => Self {
                input_tokens_per_minute: 200_000,
                output_tokens_per_minute: 50_000,
                requests_per_minute: Some(3500),
                retry_after_secs: None,
            },
            // Ollama - unlimited (local)
            m if m.contains("ollama") || m.contains("llama") || m.contains("mistral") || m.contains("qwen") => Self {
                input_tokens_per_minute: u32::MAX,
                output_tokens_per_minute: u32::MAX,
                requests_per_minute: None,
                retry_after_secs: None,
            },
            // Default conservative estimate
            _ => Self::default(),
        }
    }

    /// Check if a request would likely exceed rate limits
    #[must_use]
    pub fn would_exceed(&self, estimated_input_tokens: usize) -> bool {
        estimated_input_tokens as u32 > self.input_tokens_per_minute
    }

    /// Update limits from API response headers
    pub fn update_from_headers(&mut self, headers: &RateLimitHeaders) {
        if let Some(limit) = headers.limit_tokens {
            self.input_tokens_per_minute = limit;
        }
        if let Some(requests) = headers.limit_requests {
            self.requests_per_minute = Some(requests);
        }
        if let Some(retry) = headers.retry_after {
            self.retry_after_secs = Some(retry);
        }
    }
}

/// Helper trait for building requests fluently.
pub trait RequestBuilder {
    /// Set the model.
    #[must_use]
    fn with_model(self, model: impl Into<String>) -> Self;
    /// Add a message.
    #[must_use]
    fn with_message(self, message: Message) -> Self;
    /// Set max tokens.
    #[must_use]
    fn with_max_tokens(self, max_tokens: u32) -> Self;
    /// Set temperature.
    #[must_use]
    fn with_temperature(self, temperature: f32) -> Self;
}

impl RequestBuilder for CompletionRequest {
    fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    fn with_message(mut self, message: Message) -> Self {
        self.messages.push(message);
        self
    }

    fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }
}

impl CompletionRequest {
    /// Set context window limit (for Ollama num_ctx).
    /// This reduces VRAM usage by limiting the KV cache size,
    /// allowing more model layers to fit on GPU.
    #[must_use]
    pub fn with_context_limit(mut self, limit: u32) -> Self {
        self.context_limit = Some(limit);
        self
    }
}

// ============================================================================
// Embeddings Support
// ============================================================================

/// Embedding provider type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingProvider {
    OpenAI,
    Ollama,
}

/// Embedding client for generating vector embeddings
#[derive(Clone)]
pub struct EmbeddingClient {
    http: Client,
    provider: EmbeddingProvider,
    api_key: String,
    base_url: String,
    model: String,
}

impl EmbeddingClient {
    /// Create `OpenAI` embedding client
    pub fn openai(api_key: impl Into<String>) -> Self {
        Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| Client::new()),
            provider: EmbeddingProvider::OpenAI,
            api_key: api_key.into(),
            base_url: "https://api.openai.com".to_string(),
            model: "text-embedding-3-small".to_string(),
        }
    }

    /// Create `Ollama` embedding client (local, no API key needed)
    pub fn ollama(base_url: impl Into<String>) -> Self {
        Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(60)) // Ollama can be slower
                .build()
                .unwrap_or_else(|_| Client::new()),
            provider: EmbeddingProvider::Ollama,
            api_key: String::new(),
            base_url: base_url.into(),
            model: "nomic-embed-text".to_string(), // Good default for Ollama
        }
    }

    /// Create Ollama embedding client with default localhost URL
    pub fn ollama_default() -> Self {
        Self::ollama("http://localhost:11434")
    }

    /// Create client with custom model.
    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Override the base URL (e.g. for OpenRouter-compatible embedding endpoints).
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Get embeddings for a batch of texts.
    ///
    /// # Errors
    ///
    /// Returns `LlmError::Api` if the API returns an error.
    /// Returns `LlmError::Network` if the request fails.
    pub async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, LlmError> {
        match self.provider {
            EmbeddingProvider::OpenAI => self.embed_openai(texts).await,
            EmbeddingProvider::Ollama => self.embed_ollama(texts).await,
        }
    }

    async fn embed_openai(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, LlmError> {
        #[derive(Serialize)]
        struct EmbedRequest<'a> {
            model: &'a str,
            input: &'a [&'a str],
        }

        #[derive(Deserialize)]
        struct EmbedResponse {
            data: Vec<EmbedData>,
        }

        #[derive(Deserialize)]
        struct EmbedData {
            embedding: Vec<f32>,
        }

        let response = self
            .http
            .post(format!("{}/v1/embeddings", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&EmbedRequest {
                model: &self.model,
                input: texts,
            })
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::from_api_response(status, message));
        }

        // Heal malformed embedding JSON bodies (some proxies/wrappers garble arrays).
        let raw = response.text().await?;
        let result: EmbedResponse = match serde_json::from_str(&raw) {
            Ok(r) => r,
            Err(e) => {
                let healed = heal::heal_json(&raw)
                    .ok_or_else(|| LlmError::Json(format!("embedding response: {e}")))?;
                serde_json::from_value(healed)
                    .map_err(|e2| LlmError::Json(format!("embedding response after heal: {e2}")))?
            }
        };
        Ok(result.data.into_iter().map(|d| d.embedding).collect())
    }

    async fn embed_ollama(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, LlmError> {
        let mut results = Vec::with_capacity(texts.len());

        for text in texts {
            let embedding = self.embed_ollama_one(text).await?;
            if embedding.is_empty() {
                return Err(LlmError::Api {
                    status: 500,
                    message: format!(
                        "Ollama returned empty embedding for model '{}'. Is the model pulled?",
                        self.model
                    ),
                });
            }
            results.push(embedding);
        }

        Ok(results)
    }

    /// Embed a single text via Ollama, trying new API then legacy.
    async fn embed_ollama_one(&self, text: &str) -> Result<Vec<f32>, LlmError> {
        #[derive(Serialize)]
        struct NewReq<'a> { model: &'a str, input: &'a str }
        #[derive(Serialize)]
        struct LegacyReq<'a> { model: &'a str, prompt: &'a str }
        #[derive(Deserialize)]
        struct NewResp { #[serde(default)] embeddings: Vec<Vec<f32>> }
        #[derive(Deserialize)]
        struct LegacyResp { #[serde(default)] embedding: Vec<f32> }

        // Try new API: POST /api/embed { model, input }
        let mut req = self
            .http
            .post(format!("{}/api/embed", self.base_url))
            .header("Content-Type", "application/json");
        if !self.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key));
        }
        let response = req
            .json(&NewReq { model: &self.model, input: text })
            .send()
            .await?;

        if response.status().is_success() {
            let raw = response.text().await.unwrap_or_default();
            let resp: Option<NewResp> = serde_json::from_str(&raw)
                .ok()
                .or_else(|| heal::heal_json(&raw).and_then(|v| serde_json::from_value(v).ok()));
            if let Some(resp) = resp {
                if let Some(emb) = resp.embeddings.into_iter().next() {
                    if !emb.is_empty() {
                        return Ok(emb);
                    }
                }
            }
        }

        // Fall back to legacy API: POST /api/embeddings { model, prompt }
        let mut req = self
            .http
            .post(format!("{}/api/embeddings", self.base_url))
            .header("Content-Type", "application/json");
        if !self.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key));
        }
        let response = req
            .json(&LegacyReq { model: &self.model, prompt: text })
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::from_api_response(status, message));
        }

        let raw = response.text().await?;
        let result: LegacyResp = match serde_json::from_str(&raw) {
            Ok(r) => r,
            Err(e) => {
                let healed = heal::heal_json(&raw)
                    .ok_or_else(|| LlmError::Json(format!("ollama embedding: {e}")))?;
                serde_json::from_value(healed)
                    .map_err(|e2| LlmError::Json(format!("ollama embedding after heal: {e2}")))?
            }
        };
        Ok(result.embedding)
    }

    /// Get embedding for a single text.
    ///
    /// # Errors
    ///
    /// Returns `LlmError::Api` if the API returns an error or no embedding is returned.
    /// Returns `LlmError::Network` if the request fails.
    pub async fn embed_one(&self, text: &str) -> Result<Vec<f32>, LlmError> {
        let mut results = self.embed(&[text]).await?;
        results.pop().ok_or_else(|| LlmError::Api {
            status: 500,
            message: "No embedding returned".to_string(),
        })
    }
}

// ============================================================================
// Streaming Support
// ============================================================================

/// A streaming event from the Anthropic API
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Start of a new message. Anthropic reports prompt-side token usage here
    /// (including prompt-cache hits/writes); other providers report zeros.
    MessageStart {
        id: String,
        model: String,
        /// Non-cached prompt tokens billed for this message.
        input_tokens: u32,
        /// Prompt tokens served from the cache (a read, cheaper).
        cache_read_tokens: u32,
        /// Prompt tokens written into the cache (a creation, one-time cost).
        cache_creation_tokens: u32,
    },
    /// Start of a content block
    ContentBlockStart {
        index: usize,
        content_type: String,
        /// Tool use ID (only for tool_use blocks)
        tool_id: Option<String>,
        /// Tool name (only for tool_use blocks)
        tool_name: Option<String>,
    },
    /// Text delta within a content block
    TextDelta { index: usize, text: String },
    /// Thinking/reasoning delta (extended thinking mode)
    ThinkingDelta { index: usize, thinking: String },
    /// Signature delta for a thinking block (must be accumulated and sent back to Anthropic)
    SignatureDelta { index: usize, signature: String },
    /// Tool use delta (JSON fragment)
    ToolUseDelta { index: usize, partial_json: String },
    /// Content block finished
    ContentBlockStop { index: usize },
    /// Message finished
    MessageStop { stop_reason: String },
    /// Usage statistics
    MessageDelta {
        stop_reason: Option<String>,
        output_tokens: u32,
    },
    /// Ping (keepalive)
    Ping,
    /// Error event (non-recoverable)
    Error { message: String },
    /// Recoverable error mid-stream (e.g., 429 rate limit).
    /// Contains accumulated partial content that can be used to continue.
    RecoverableError {
        error: LlmError,
        /// Text accumulated before the error
        partial_text: String,
        /// In-progress tool calls (id, name, partial_json)
        partial_tool_calls: Vec<(String, String, String)>,
    },
    /// Rate limit headers received (allows updating limits cache)
    RateLimitInfo {
        limit_tokens: Option<u32>,
        remaining_tokens: Option<u32>,
        reset_secs: Option<u64>,
    },
}

/// Accumulated state during streaming (for recovery)
#[derive(Debug, Clone, Default)]
pub struct StreamAccumulator {
    /// Text content accumulated so far
    pub text: String,
    /// Thinking/reasoning content accumulated so far
    pub thinking: String,
    /// Accumulated signature for the thinking block (required by Anthropic in multi-turn)
    pub thinking_signature: String,
    /// Tool calls in progress: (index, id, name, partial_json)
    pub tool_calls: Vec<(usize, String, String, String)>,
}

impl StreamAccumulator {
    /// Create a new empty accumulator
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Process an event and accumulate content
    pub fn process(&mut self, event: &StreamEvent) {
        match event {
            StreamEvent::TextDelta { text, .. } => {
                self.text.push_str(text);
            }
            StreamEvent::ThinkingDelta { thinking, .. } => {
                self.thinking.push_str(thinking);
            }
            StreamEvent::SignatureDelta { signature, .. } => {
                self.thinking_signature.push_str(signature);
            }
            StreamEvent::ContentBlockStart { index, content_type, tool_id, tool_name } => {
                if content_type == "tool_use" {
                    if let (Some(id), Some(name)) = (tool_id, tool_name) {
                        self.tool_calls.push((*index, id.clone(), name.clone(), String::new()));
                    }
                }
            }
            StreamEvent::ToolUseDelta { index, partial_json } => {
                if let Some((_, _, _, json)) = self.tool_calls.iter_mut().find(|(i, _, _, _)| i == index) {
                    json.push_str(partial_json);
                }
            }
            _ => {}
        }
    }

    /// Get partial tool calls as (id, name, json) tuples
    #[must_use]
    pub fn partial_tool_calls(&self) -> Vec<(String, String, String)> {
        self.tool_calls.iter()
            .map(|(_, id, name, json)| (id.clone(), name.clone(), json.clone()))
            .collect()
    }

    /// Check if there's any accumulated content
    #[must_use]
    pub fn has_content(&self) -> bool {
        !self.text.is_empty() || !self.thinking.is_empty() || !self.tool_calls.is_empty()
    }
}

/// Anthropic SSE event types
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicSSE {
    MessageStart { message: MessageStartData },
    ContentBlockStart { index: usize, content_block: ContentBlockData },
    ContentBlockDelta { index: usize, delta: DeltaData },
    ContentBlockStop { index: usize },
    MessageDelta { delta: MessageDeltaData, usage: Option<UsageDelta> },
    MessageStop,
    Ping,
    Error { error: ErrorData },
}

#[derive(Debug, Deserialize)]
struct MessageStartData {
    id: String,
    model: String,
    #[serde(default)]
    usage: Option<MessageStartUsage>,
}

/// Prompt-side token usage reported in the Anthropic `message_start` event.
/// Field names mirror the wire JSON keys, so the shared `_input_tokens` suffix
/// is required (not a naming smell).
#[derive(Debug, Deserialize, Default)]
#[allow(clippy::struct_field_names)]
struct MessageStartUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct ContentBlockData {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DeltaData {
    TextDelta { text: String },
    ThinkingDelta { thinking: String },
    InputJsonDelta { partial_json: String },
    /// Signature delta for thinking blocks (must be accumulated and sent back)
    SignatureDelta { signature: String },
    /// Catch-all for unknown delta types so the
    /// surrounding `ContentBlockDelta` event still deserializes successfully.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
struct MessageDeltaData {
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsageDelta {
    output_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct ErrorData {
    message: String,
}

impl LlmClient {
    /// Stream an Anthropic-format request, dispatching to the appropriate provider.
    ///
    /// - Anthropic: Native SSE streaming via `/v1/messages`
    /// - OpenAI/OpenRouter/GitHub/ClaudeProxy: Converts to OpenAI SSE streaming
    /// - Ollama: Converts to Ollama streaming via `/api/chat`
    pub fn stream_anthropic(
        &self,
        request: &AnthropicRequest,
    ) -> impl Stream<Item = Result<StreamEvent, LlmError>> + '_ {
        let provider = self.provider;
        let request = request.clone();

        stream! {
            match provider {
                Provider::Anthropic => {
                    let raw = self.stream_anthropic_native(&request);
                    tokio::pin!(raw);
                    while let Some(item) = raw.next().await {
                        yield item;
                    }
                }
                Provider::OpenAI | Provider::OpenRouter | Provider::GitHubModels | Provider::ClaudeProxy => {
                    let raw = self.stream_anthropic_via_openai(&request);
                    tokio::pin!(raw);
                    while let Some(item) = raw.next().await {
                        yield item;
                    }
                }
                Provider::Ollama => {
                    let raw = self.stream_anthropic_via_ollama(&request);
                    tokio::pin!(raw);
                    while let Some(item) = raw.next().await {
                        yield item;
                    }
                }
            }
        }
    }

    /// Native Anthropic SSE streaming implementation
    fn stream_anthropic_native(
        &self,
        request: &AnthropicRequest,
    ) -> impl Stream<Item = Result<StreamEvent, LlmError>> + '_ {
        let mut request = request.clone();
        request.stream = Some(true);

        let http = self.http.clone();
        let api_key = self.api_key.clone();
        let base_url = self.base_url.clone();
        let is_oauth = is_oauth_token(&api_key);

        // Store original tools for reverse mapping
        let original_tools = request.tools.clone().unwrap_or_default();

        // Prepare OAuth request with array-based system prompt
        let oauth_request = if is_oauth {
            info!("Claude Code stealth mode: applying OAuth headers and array-based system prompt");

            let mut oauth_req = OAuthAnthropicRequest::from_request(&request, true);
            oauth_req.stream = Some(true);

            // Remap tool names to Claude Code format
            if let Some(ref mut tools) = oauth_req.tools {
                for tool in tools {
                    let old_name = tool.name.clone();
                    tool.name = to_claude_code_tool_name(&tool.name);
                    if old_name != tool.name {
                        debug!(old = %old_name, new = %tool.name, "Remapped tool name");
                    }
                }
            }

            debug!(
                system_blocks = oauth_req.system.len(),
                tool_names = ?oauth_req.tools.as_ref().map(|t| t.iter().map(|x| x.name.as_str()).collect::<Vec<_>>()),
                "OAuth streaming request prepared with array-based system prompt"
            );

            Some(oauth_req)
        } else {
            None
        };

        stream! {
            let request_builder = http
                .post(format!("{base_url}/v1/messages"));

            let request_builder = if is_oauth {
                let token = get_raw_token(&api_key);
                request_builder
                    .header("Authorization", format!("Bearer {token}"))
                    .header("accept", "application/json")
                    .header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14")
                    .header("user-agent", format!("claude-cli/{CLAUDE_CODE_VERSION} (external, cli)"))
                    .header("x-app", "cli")
                    .header("anthropic-dangerous-direct-browser-access", "true")
            } else {
                request_builder.header("x-api-key", &api_key)
            };

            debug!(
                is_oauth = is_oauth,
                model = %request.model,
                has_system = request.system.is_some(),
                tools_count = request.tools.as_ref().map(|t| t.len()).unwrap_or(0),
                "Sending Anthropic streaming request"
            );

            let response = if let Some(ref oauth_req) = oauth_request {
                request_builder
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json")
                    .json(oauth_req)
                    .send()
                    .await
            } else {
                request_builder
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json")
                    .json(&request)
                    .send()
                    .await
            };

            let response = match response {
                Ok(r) => {
                    debug!(status = %r.status(), "Anthropic API response received");
                    r
                },
                Err(e) => {
                    warn!(error = %e, "Anthropic API request failed");
                    yield Err(LlmError::Http(e.to_string()));
                    return;
                }
            };

            let rate_headers = RateLimitHeaders::from_headers(response.headers());
            if rate_headers.limit_tokens.is_some() || rate_headers.remaining_tokens.is_some() {
                yield Ok(StreamEvent::RateLimitInfo {
                    limit_tokens: rate_headers.limit_tokens,
                    remaining_tokens: rate_headers.remaining_tokens,
                    reset_secs: rate_headers.reset_tokens_secs,
                });
            }

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let message = response.text().await.unwrap_or_default();
                yield Err(LlmError::from_api_response(status, message));
                return;
            }

            let mut byte_stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut accumulator = StreamAccumulator::new();

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        let error = LlmError::Http(e.to_string());
                        if accumulator.has_content() {
                            yield Ok(StreamEvent::RecoverableError {
                                error,
                                partial_text: accumulator.text.clone(),
                                partial_tool_calls: accumulator.partial_tool_calls(),
                            });
                        } else {
                            yield Err(error);
                        }
                        return;
                    }
                };

                match std::str::from_utf8(&chunk) {
                    Ok(s) => buffer.push_str(s),
                    Err(e) => {
                        let error = LlmError::Stream(format!("Invalid UTF-8 in stream: {e}"));
                        if accumulator.has_content() {
                            yield Ok(StreamEvent::RecoverableError {
                                error,
                                partial_text: accumulator.text.clone(),
                                partial_tool_calls: accumulator.partial_tool_calls(),
                            });
                        } else {
                            yield Err(error);
                        }
                        return;
                    }
                }

                while let Some(event_str) = extract_sse_event(&mut buffer) {
                    if let Some(mut stream_event) = parse_sse_event(&event_str) {
                        if is_oauth {
                            if let StreamEvent::ContentBlockStart { tool_name: Some(ref mut name), .. } = stream_event {
                                *name = from_claude_code_tool_name(name, &original_tools);
                            }
                        }
                        accumulator.process(&stream_event);
                        yield Ok(stream_event);
                    }
                }
            }

            if !buffer.trim().is_empty() {
                if let Some(mut stream_event) = parse_sse_event(&buffer) {
                    if is_oauth {
                        if let StreamEvent::ContentBlockStart { tool_name: Some(ref mut name), .. } = stream_event {
                            *name = from_claude_code_tool_name(name, &original_tools);
                        }
                    }
                    accumulator.process(&stream_event);
                    yield Ok(stream_event);
                }
            }
        }
    }

    /// Stream an AnthropicRequest via OpenAI-compatible SSE streaming.
    /// Converts the request to OpenAI format, streams via /v1/chat/completions,
    /// and converts events back to Anthropic StreamEvent format.
    fn stream_anthropic_via_openai(
        &self,
        request: &AnthropicRequest,
    ) -> impl Stream<Item = Result<StreamEvent, LlmError>> + '_ {
        let http = self.http.clone();
        let api_key = self.api_key.clone();
        let base_url = self.base_url.clone();
        let request = request.clone();

        stream! {
            let (messages_json, tools_json) = anthropic_to_openai_request(&request);

            let mut body = serde_json::json!({
                "model": request.model,
                "messages": messages_json,
                "max_tokens": request.max_tokens,
                "stream": true,
            });
            if let Some(temp) = request.temperature {
                body["temperature"] = serde_json::json!(temp);
            }
            if let Some(tools) = tools_json {
                body["tools"] = tools;
            }

            let response = match http
                .post(format!("{base_url}/v1/chat/completions"))
                .header("Authorization", format!("Bearer {api_key}"))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    yield Err(LlmError::Http(e.to_string()));
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let message = response.text().await.unwrap_or_default();
                yield Err(LlmError::from_api_response(status, message));
                return;
            }

            // Reuse OpenAI SSE parsing logic (same format as stream_openai)
            let mut byte_stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut text_block_started = false;
            let mut tool_calls_started: std::collections::HashMap<usize, (String, String)> = std::collections::HashMap::new();

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(LlmError::Stream(e.to_string()));
                        return;
                    }
                };

                match std::str::from_utf8(&chunk) {
                    Ok(s) => buffer.push_str(s),
                    Err(e) => {
                        yield Err(LlmError::Stream(format!("Invalid UTF-8: {e}")));
                        return;
                    }
                }

                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim().to_string();
                    buffer = buffer[line_end + 1..].to_string();

                    if line.is_empty() || !line.starts_with("data: ") {
                        continue;
                    }

                    let data = &line[6..];

                    if data == "[DONE]" {
                        if text_block_started {
                            yield Ok(StreamEvent::ContentBlockStop { index: 0 });
                        }
                        for (idx, _) in &tool_calls_started {
                            yield Ok(StreamEvent::ContentBlockStop { index: *idx });
                        }
                        yield Ok(StreamEvent::MessageStop { stop_reason: "end_turn".to_string() });
                        return;
                    }

                    #[derive(Deserialize, Debug)]
                    struct StreamChunk {
                        choices: Vec<ChunkChoice>,
                    }
                    #[derive(Deserialize, Debug)]
                    struct ChunkChoice {
                        delta: Option<ChunkDelta>,
                        finish_reason: Option<String>,
                    }
                    #[derive(Deserialize, Debug)]
                    struct ChunkDelta {
                        content: Option<String>,
                        tool_calls: Option<Vec<ToolCallDelta>>,
                    }
                    #[derive(Deserialize, Debug)]
                    struct ToolCallDelta {
                        index: usize,
                        id: Option<String>,
                        function: Option<FunctionDelta>,
                    }
                    #[derive(Deserialize, Debug)]
                    struct FunctionDelta {
                        name: Option<String>,
                        arguments: Option<String>,
                    }

                    if let Ok(chunk) = serde_json::from_str::<StreamChunk>(data) {
                        for choice in chunk.choices {
                            if let Some(delta) = choice.delta {
                                if let Some(text) = delta.content {
                                    if !text.is_empty() {
                                        if !text_block_started {
                                            text_block_started = true;
                                            yield Ok(StreamEvent::ContentBlockStart {
                                                index: 0,
                                                content_type: "text".to_string(),
                                                tool_id: None,
                                                tool_name: None,
                                            });
                                        }
                                        yield Ok(StreamEvent::TextDelta { index: 0, text });
                                    }
                                }

                                if let Some(tool_calls) = delta.tool_calls {
                                    for tc in tool_calls {
                                        let block_index = tc.index + 1;
                                        if let (Some(id), Some(func)) = (&tc.id, &tc.function) {
                                            if let Some(name) = &func.name {
                                                if text_block_started {
                                                    yield Ok(StreamEvent::ContentBlockStop { index: 0 });
                                                    text_block_started = false;
                                                }
                                                tool_calls_started.insert(block_index, (id.clone(), name.clone()));
                                                yield Ok(StreamEvent::ContentBlockStart {
                                                    index: block_index,
                                                    content_type: "tool_use".to_string(),
                                                    tool_id: Some(id.clone()),
                                                    tool_name: Some(name.clone()),
                                                });
                                            }
                                        }
                                        if let Some(func) = &tc.function {
                                            if let Some(args) = &func.arguments {
                                                if !args.is_empty() {
                                                    yield Ok(StreamEvent::ToolUseDelta {
                                                        index: block_index,
                                                        partial_json: args.clone(),
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            if let Some(reason) = choice.finish_reason {
                                if text_block_started {
                                    yield Ok(StreamEvent::ContentBlockStop { index: 0 });
                                }
                                for (idx, _) in &tool_calls_started {
                                    yield Ok(StreamEvent::ContentBlockStop { index: *idx });
                                }
                                let stop_reason = match reason.as_str() {
                                    "tool_calls" => "tool_use",
                                    "stop" => "end_turn",
                                    other => other,
                                };
                                yield Ok(StreamEvent::MessageStop { stop_reason: stop_reason.to_string() });
                                return;
                            }
                        }
                    }
                }
            }

            // Emit close events if stream ended without finish_reason
            if text_block_started {
                yield Ok(StreamEvent::ContentBlockStop { index: 0 });
            }
            for (idx, _) in &tool_calls_started {
                yield Ok(StreamEvent::ContentBlockStop { index: *idx });
            }
            yield Ok(StreamEvent::MessageStop { stop_reason: "end_turn".to_string() });
        }
    }

    /// Stream an AnthropicRequest via Ollama `/api/chat` with `stream: true`.
    /// Ollama streams NDJSON (one JSON object per line, not SSE).
    fn stream_anthropic_via_ollama(
        &self,
        request: &AnthropicRequest,
    ) -> impl Stream<Item = Result<StreamEvent, LlmError>> + '_ {
        let http = self.http.clone();
        let base_url = self.base_url.clone();
        let api_key = self.api_key.clone();
        let request = request.clone();

        stream! {
            let (messages_json, tools_json) = anthropic_to_ollama_request(&request);

            let mut body = serde_json::json!({
                "model": request.model,
                "messages": messages_json,
                "stream": true,
            });

            let mut options = serde_json::Map::new();
            if let Some(temp) = request.temperature {
                options.insert("temperature".to_string(), serde_json::json!(temp));
            }
            // num_predict=-1 means unlimited output — lets thinking models use
            // as many tokens as needed for reasoning without starving the response.
            options.insert("num_predict".to_string(), serde_json::json!(-1));
            // Request 32K context (Ollama models default to small contexts).
            options.insert("num_ctx".to_string(), serde_json::json!(32768));
            if !options.is_empty() {
                body["options"] = serde_json::Value::Object(options);
            }
            if let Some(tools) = tools_json {
                body["tools"] = tools;
            }

            let mut req = http
                .post(format!("{base_url}/api/chat"))
                .header("content-type", "application/json");
            if !api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {api_key}"));
            }
            let response = match req
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    yield Err(LlmError::Http(e.to_string()));
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let message = response.text().await.unwrap_or_default();
                yield Err(LlmError::from_api_response(status, message));
                return;
            }

            // Ollama streams NDJSON: one JSON per line
            let mut byte_stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut thinking_block_started = false;
            let mut text_block_started = false;
            let mut tool_block_count = 0usize;
            // Ollama thinking uses block index 0 (same as Anthropic);
            // text starts at index 1 when thinking is present, or 0 otherwise.
            let mut next_block_index = 0usize;

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(LlmError::Stream(e.to_string()));
                        return;
                    }
                };

                match std::str::from_utf8(&chunk) {
                    Ok(s) => buffer.push_str(s),
                    Err(e) => {
                        yield Err(LlmError::Stream(format!("Invalid UTF-8: {e}")));
                        return;
                    }
                }

                // Process complete lines (NDJSON)
                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim().to_string();
                    buffer = buffer[line_end + 1..].to_string();

                    if line.is_empty() {
                        continue;
                    }

                    let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) else {
                        continue;
                    };

                    // Check for done
                    let done = obj["done"].as_bool().unwrap_or(false);

                    // Stream thinking content (qwen3 and other thinking models)
                    // Ollama returns thinking tokens in message.thinking.
                    // Emit proper ContentBlockStart/Stop to match Anthropic's block structure.
                    if let Some(thinking) = obj["message"]["thinking"].as_str() {
                        if !thinking.is_empty() {
                            if !thinking_block_started {
                                thinking_block_started = true;
                                yield Ok(StreamEvent::ContentBlockStart {
                                    index: next_block_index,
                                    content_type: "thinking".to_string(),
                                    tool_id: None,
                                    tool_name: None,
                                });
                            }
                            yield Ok(StreamEvent::ThinkingDelta { index: next_block_index, thinking: thinking.to_string() });
                        }
                    }

                    // Stream text content.
                    // When text starts arriving and thinking was active, close the thinking block first.
                    if let Some(content) = obj["message"]["content"].as_str() {
                        if !content.is_empty() {
                            // Close thinking block when transitioning to text
                            if thinking_block_started {
                                yield Ok(StreamEvent::ContentBlockStop { index: next_block_index });
                                thinking_block_started = false;
                                next_block_index += 1;
                            }
                            if !text_block_started {
                                text_block_started = true;
                                yield Ok(StreamEvent::ContentBlockStart {
                                    index: next_block_index,
                                    content_type: "text".to_string(),
                                    tool_id: None,
                                    tool_name: None,
                                });
                            }
                            yield Ok(StreamEvent::TextDelta { index: next_block_index, text: content.to_string() });
                        }
                    }

                    // Tool calls: Ollama emits these in a message with done=false,
                    // followed by a separate done=true message with empty content.
                    // Process tool_calls from ANY message, not just the done message.
                    if let Some(tool_calls) = obj["message"]["tool_calls"].as_array() {
                        if !tool_calls.is_empty() {
                            // Close any open thinking/text blocks before tool calls
                            if thinking_block_started {
                                yield Ok(StreamEvent::ContentBlockStop { index: next_block_index });
                                thinking_block_started = false;
                                next_block_index += 1;
                            }
                            if text_block_started {
                                yield Ok(StreamEvent::ContentBlockStop { index: next_block_index });
                                text_block_started = false;
                                next_block_index += 1;
                            }

                            for tc in tool_calls {
                                tool_block_count += 1;
                                let block_idx = next_block_index;
                                next_block_index += 1;
                                let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                                let args = tc["function"]["arguments"].to_string();
                                let id = tc["id"].as_str()
                                    .map(String::from)
                                    .unwrap_or_else(|| format!("toolu_{:08x}", tool_block_count));
                                yield Ok(StreamEvent::ContentBlockStart {
                                    index: block_idx,
                                    content_type: "tool_use".to_string(),
                                    tool_id: Some(id),
                                    tool_name: Some(name),
                                });
                                yield Ok(StreamEvent::ToolUseDelta {
                                    index: block_idx,
                                    partial_json: args,
                                });
                                yield Ok(StreamEvent::ContentBlockStop { index: block_idx });
                            }
                        }
                    }

                    if done {
                        // Close any remaining open blocks
                        if thinking_block_started {
                            yield Ok(StreamEvent::ContentBlockStop { index: next_block_index });
                            next_block_index += 1;
                        }
                        if text_block_started {
                            yield Ok(StreamEvent::ContentBlockStop { index: next_block_index });
                            // next_block_index not needed after this
                        }

                        // Emit output token count
                        if let Some(eval_count) = obj["eval_count"].as_u64() {
                            yield Ok(StreamEvent::MessageDelta {
                                stop_reason: None,
                                output_tokens: eval_count as u32,
                            });
                        }

                        let stop_reason = if tool_block_count > 0 { "tool_use" } else { "end_turn" };
                        yield Ok(StreamEvent::MessageStop { stop_reason: stop_reason.to_string() });
                        return;
                    }
                }
            }

            // If we get here without done=true, close gracefully
            if thinking_block_started {
                yield Ok(StreamEvent::ContentBlockStop { index: next_block_index });
            }
            if text_block_started {
                yield Ok(StreamEvent::ContentBlockStop { index: next_block_index });
            }
            yield Ok(StreamEvent::MessageStop { stop_reason: "end_turn".to_string() });
        }
    }

    /// Stream using OpenAI-compatible API (for OpenAI, OpenRouter, GitHub Models, Claude Proxy)
    /// Supports both text responses and tool calls.
    pub fn stream_openai(
        &self,
        request: &CompletionRequest,
    ) -> impl Stream<Item = Result<StreamEvent, LlmError>> + '_ {
        let http = self.http.clone();
        let api_key = self.api_key.clone();
        let base_url = self.base_url.clone();
        let request = request.clone();

        stream! {
            // === Request types ===
            #[derive(Serialize)]
            struct OpenAIMessage {
                role: String,
                content: String,
            }

            #[derive(Serialize)]
            struct OpenAITool {
                #[serde(rename = "type")]
                tool_type: String,
                function: OpenAIFunction,
            }

            #[derive(Serialize)]
            struct OpenAIFunction {
                name: String,
                description: String,
                parameters: serde_json::Value,
            }

            #[derive(Serialize)]
            struct OpenAIRequest {
                model: String,
                messages: Vec<OpenAIMessage>,
                #[serde(skip_serializing_if = "Option::is_none")]
                max_tokens: Option<u32>,
                #[serde(skip_serializing_if = "Option::is_none")]
                temperature: Option<f32>,
                stream: bool,
                #[serde(skip_serializing_if = "Option::is_none")]
                tools: Option<Vec<OpenAITool>>,
            }

            // === Response types ===
            #[derive(Deserialize, Debug)]
            struct StreamChunk {
                choices: Vec<ChunkChoice>,
            }

            #[derive(Deserialize, Debug)]
            struct ChunkChoice {
                delta: Option<ChunkDelta>,
                finish_reason: Option<String>,
            }

            #[derive(Deserialize, Debug)]
            struct ChunkDelta {
                content: Option<String>,
    #[serde(rename = "role")]
                _role: Option<String>,
                tool_calls: Option<Vec<ToolCallDelta>>,
            }

            #[derive(Deserialize, Debug)]
            struct ToolCallDelta {
                index: usize,
                id: Option<String>,
                function: Option<FunctionDelta>,
            }

            #[derive(Deserialize, Debug)]
            struct FunctionDelta {
                name: Option<String>,
                arguments: Option<String>,
            }

            // Convert messages to OpenAI format
            let messages: Vec<OpenAIMessage> = request
                .messages
                .iter()
                .map(|m| OpenAIMessage {
                    role: match m.role {
                        Role::System => "system".to_string(),
                        Role::User => "user".to_string(),
                        Role::Assistant => "assistant".to_string(),
                    },
                    content: m.content.clone(),
                })
                .collect();

            // Convert tools to OpenAI format
            let tools: Option<Vec<OpenAITool>> = if request.tools.is_empty() {
                None
            } else {
                Some(request.tools.iter().filter_map(|v| {
                    // Try to parse as ToolDefinition
                    if let Ok(tool_def) = serde_json::from_value::<ToolDefinition>(v.clone()) {
                        Some(OpenAITool {
                            tool_type: "function".to_string(),
                            function: OpenAIFunction {
                                name: tool_def.name,
                                description: tool_def.description,
                                parameters: tool_def.input_schema,
                            },
                        })
                    } else {
                        None
                    }
                }).collect())
            };

            let body = OpenAIRequest {
                model: request.model.clone(),
                messages,
                max_tokens: request.max_tokens,
                temperature: request.temperature,
                stream: true,
                tools,
            };

            let response = match http
                .post(format!("{base_url}/v1/chat/completions"))
                .header("Authorization", format!("Bearer {api_key}"))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    yield Err(LlmError::Http(e.to_string()));
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let message = response.text().await.unwrap_or_default();
                yield Err(LlmError::from_api_response(status, message));
                return;
            }

            let mut byte_stream = response.bytes_stream();
            let mut buffer = String::new();

            // Track content blocks (text at index 0, tool calls at higher indices)
            let mut text_block_started = false;
            let mut tool_calls_started: std::collections::HashMap<usize, (String, String)> = std::collections::HashMap::new(); // index -> (id, name)

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(LlmError::Stream(e.to_string()));
                        return;
                    }
                };

                match std::str::from_utf8(&chunk) {
                    Ok(s) => buffer.push_str(s),
                    Err(e) => {
                        yield Err(LlmError::Stream(format!("Invalid UTF-8: {e}")));
                        return;
                    }
                }

                // Parse SSE events from buffer
                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim().to_string();
                    buffer = buffer[line_end + 1..].to_string();

                    if line.is_empty() || !line.starts_with("data: ") {
                        continue;
                    }

                    let data = &line[6..]; // Strip "data: "

                    if data == "[DONE]" {
                        // Close any open blocks
                        if text_block_started {
                            yield Ok(StreamEvent::ContentBlockStop { index: 0 });
                        }
                        for (idx, _) in &tool_calls_started {
                            yield Ok(StreamEvent::ContentBlockStop { index: *idx });
                        }
                        yield Ok(StreamEvent::MessageStop { stop_reason: "end_turn".to_string() });
                        return;
                    }

                    if let Ok(chunk) = serde_json::from_str::<StreamChunk>(data) {
                        for choice in chunk.choices {
                            if let Some(delta) = choice.delta {
                                // Handle text content
                                if let Some(text) = delta.content {
                                    if !text.is_empty() {
                                        if !text_block_started {
                                            text_block_started = true;
                                            yield Ok(StreamEvent::ContentBlockStart {
                                                index: 0,
                                                content_type: "text".to_string(),
                                                tool_id: None,
                                                tool_name: None,
                                            });
                                        }
                                        yield Ok(StreamEvent::TextDelta { index: 0, text });
                                    }
                                }

                                // Handle tool calls
                                if let Some(tool_calls) = delta.tool_calls {
                                    for tc in tool_calls {
                                        // Tool index offset by 1 (text is at 0)
                                        let block_index = tc.index + 1;

                                        // Check if this is a new tool call
                                        if let (Some(id), Some(func)) = (&tc.id, &tc.function) {
                                            if let Some(name) = &func.name {
                                                // Close text block if open (tool calls come after text)
                                                if text_block_started {
                                                    yield Ok(StreamEvent::ContentBlockStop { index: 0 });
                                                    text_block_started = false;
                                                }

                                                // New tool call
                                                tool_calls_started.insert(block_index, (id.clone(), name.clone()));
                                                yield Ok(StreamEvent::ContentBlockStart {
                                                    index: block_index,
                                                    content_type: "tool_use".to_string(),
                                                    tool_id: Some(id.clone()),
                                                    tool_name: Some(name.clone()),
                                                });
                                            }
                                        }

                                        // Stream tool arguments
                                        if let Some(func) = &tc.function {
                                            if let Some(args) = &func.arguments {
                                                if !args.is_empty() {
                                                    yield Ok(StreamEvent::ToolUseDelta {
                                                        index: block_index,
                                                        partial_json: args.clone(),
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // Check for stop
                            if let Some(reason) = choice.finish_reason {
                                // Close any open blocks
                                if text_block_started {
                                    yield Ok(StreamEvent::ContentBlockStop { index: 0 });
                                }
                                for (idx, _) in &tool_calls_started {
                                    yield Ok(StreamEvent::ContentBlockStop { index: *idx });
                                }

                                let stop_reason = match reason.as_str() {
                                    "tool_calls" => "tool_use",
                                    "stop" => "end_turn",
                                    other => other,
                                };
                                yield Ok(StreamEvent::MessageStop { stop_reason: stop_reason.to_string() });
                                return;
                            }
                        }
                    }
                }
            }

            // If we got here without MessageStop, emit it
            if text_block_started {
                yield Ok(StreamEvent::ContentBlockStop { index: 0 });
            }
            for (idx, _) in &tool_calls_started {
                yield Ok(StreamEvent::ContentBlockStop { index: *idx });
            }
            yield Ok(StreamEvent::MessageStop { stop_reason: "end_turn".to_string() });
        }
    }

    /// Convenience method to stream a CompletionRequest.
    /// Converts to provider-specific format automatically.
    ///
    /// Returns `StreamEvent` directly (errors converted to `Error` or `RecoverableError` events).
    pub fn stream(
        &self,
        request: &CompletionRequest,
    ) -> impl Stream<Item = StreamEvent> + '_ {
        let provider = self.provider.clone();
        let request = request.clone();

        stream! {
            // Route to appropriate streaming method based on provider
            match provider {
                Provider::Anthropic => {
                    // Convert to Anthropic format
                    let system_msg = request
                        .messages
                        .iter()
                        .find(|m| m.role == Role::System)
                        .map(|m| m.content.clone());

                    let messages: Vec<AnthropicMessage> = request
                        .messages
                        .iter()
                        .filter(|m| m.role != Role::System)
                        .map(|m| AnthropicMessage {
                            role: match m.role {
                                Role::User | Role::System => "user",
                                Role::Assistant => "assistant",
                            }.to_string(),
                            content: vec![ContentBlock::Text { text: m.content.clone() }],
                        })
                        .collect();

                    let mut all_messages = messages;
                    all_messages.extend(request.anthropic_messages.clone());

                    let tools = if request.tools.is_empty() {
                        None
                    } else {
                        Some(request.tools.iter().filter_map(|v| {
                            serde_json::from_value::<ToolDefinition>(v.clone()).ok()
                        }).collect())
                    };

                    let anthropic_request = AnthropicRequest {
                        model: request.model.clone(),
                        messages: all_messages,
                        max_tokens: request.max_tokens.unwrap_or(4096),
                        temperature: if request.thinking.is_some() { None } else { request.temperature },
                        system: system_msg,
                        tools,
                        stream: Some(true),
                        thinking: request.thinking.clone(),
                        cache_control: None,
                    };

                    let raw_stream = self.stream_anthropic(&anthropic_request);
                    tokio::pin!(raw_stream);

                    while let Some(result) = raw_stream.next().await {
                        match result {
                            Ok(event) => yield event,
                            Err(e) => {
                                if e.should_fallback() {
                                    yield StreamEvent::RecoverableError {
                                        error: e,
                                        partial_text: String::new(),
                                        partial_tool_calls: Vec::new(),
                                    };
                                } else {
                                    yield StreamEvent::Error { message: e.to_string() };
                                }
                                return;
                            }
                        }
                    }
                }
                Provider::OpenAI | Provider::OpenRouter | Provider::GitHubModels | Provider::ClaudeProxy => {
                    // Use OpenAI-compatible streaming
                    let raw_stream = self.stream_openai(&request);
                    tokio::pin!(raw_stream);

                    while let Some(result) = raw_stream.next().await {
                        match result {
                            Ok(event) => yield event,
                            Err(e) => {
                                if e.should_fallback() {
                                    yield StreamEvent::RecoverableError {
                                        error: e,
                                        partial_text: String::new(),
                                        partial_tool_calls: Vec::new(),
                                    };
                                } else {
                                    yield StreamEvent::Error { message: e.to_string() };
                                }
                                return;
                            }
                        }
                    }
                }
                Provider::Ollama => {
                    // Ollama streaming: build AnthropicRequest and delegate to stream_anthropic
                    // which already handles Ollama NDJSON streaming via stream_anthropic_via_ollama
                    let system_msg = request
                        .messages
                        .iter()
                        .find(|m| m.role == Role::System)
                        .map(|m| m.content.clone());

                    let messages: Vec<AnthropicMessage> = request
                        .messages
                        .iter()
                        .filter(|m| m.role != Role::System)
                        .map(|m| AnthropicMessage {
                            role: match m.role {
                                Role::User | Role::System => "user",
                                Role::Assistant => "assistant",
                            }.to_string(),
                            content: vec![ContentBlock::Text { text: m.content.clone() }],
                        })
                        .collect();

                    let mut all_messages = messages;
                    all_messages.extend(request.anthropic_messages.clone());

                    let tools = if request.tools.is_empty() {
                        None
                    } else {
                        Some(request.tools.iter().filter_map(|v| {
                            serde_json::from_value::<ToolDefinition>(v.clone()).ok()
                        }).collect())
                    };

                    let anthropic_request = AnthropicRequest {
                        model: request.model.clone(),
                        messages: all_messages,
                        max_tokens: request.max_tokens.unwrap_or(4096),
                        temperature: request.temperature,
                        system: system_msg,
                        tools,
                        stream: Some(true),
                        thinking: request.thinking.clone(),
                        cache_control: None,
                    };

                    let raw_stream = self.stream_anthropic(&anthropic_request);
                    tokio::pin!(raw_stream);

                    while let Some(result) = raw_stream.next().await {
                        match result {
                            Ok(event) => yield event,
                            Err(e) => {
                                if e.should_fallback() {
                                    yield StreamEvent::RecoverableError {
                                        error: e,
                                        partial_text: String::new(),
                                        partial_tool_calls: Vec::new(),
                                    };
                                } else {
                                    yield StreamEvent::Error { message: e.to_string() };
                                }
                                return;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Stream with explicit accumulator for advanced recovery scenarios.
    ///
    /// Returns both events and the final accumulator state (even on error).
    pub fn stream_with_recovery(
        &self,
        request: &CompletionRequest,
    ) -> impl Stream<Item = (StreamEvent, StreamAccumulator)> + '_ {
        let base_stream = self.stream(request);

        stream! {
            let mut accumulator = StreamAccumulator::new();
            tokio::pin!(base_stream);

            while let Some(event) = base_stream.next().await {
                accumulator.process(&event);
                yield (event, accumulator.clone());
            }
        }
    }
}

// ============================================================================
// Anthropic ↔ OpenAI/Ollama Conversion Layer
// ============================================================================

/// How to encode `tool_calls[].function.arguments` on the wire.
///
/// OpenAI-compatible endpoints (/v1/chat/completions) expect a JSON-encoded
/// STRING. Ollama's native /api/chat expects a JSON OBJECT and rejects the
/// string form at request-decode time with HTTP 400
/// `Value looks like object, but can't find closing '}' symbol` — even when
/// the string contains valid JSON (verified live against Ollama 0.31.2).
#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolArgsWire {
    /// `"arguments": "{\"path\": ...}"` — OpenAI convention
    JsonString,
    /// `"arguments": {"path": ...}` — Ollama native convention
    JsonObject,
}

/// Convert an AnthropicRequest into OpenAI-compatible messages and tools JSON.
/// Returns (messages_json_array, tools_json_array_or_none).
fn anthropic_to_openai_request(request: &AnthropicRequest) -> (serde_json::Value, Option<serde_json::Value>) {
    anthropic_to_wire_request(request, ToolArgsWire::JsonString)
}

/// Convert an AnthropicRequest for Ollama's NATIVE /api/chat endpoint.
/// Identical to the OpenAI conversion except tool-call arguments stay a JSON
/// object — the string form 400s every request that echoes tool history.
fn anthropic_to_ollama_request(request: &AnthropicRequest) -> (serde_json::Value, Option<serde_json::Value>) {
    anthropic_to_wire_request(request, ToolArgsWire::JsonObject)
}

fn anthropic_to_wire_request(
    request: &AnthropicRequest,
    args_wire: ToolArgsWire,
) -> (serde_json::Value, Option<serde_json::Value>) {
    let mut messages = Vec::new();

    // System prompt → system message
    if let Some(ref system) = request.system {
        if !system.is_empty() {
            messages.push(serde_json::json!({
                "role": "system",
                "content": system,
            }));
        }
    }

    // Convert Anthropic messages to OpenAI messages
    for msg in &request.messages {
        let role = msg.role.as_str();
        match role {
            "user" => {
                // User messages: may contain text and/or tool_result blocks
                let mut text_parts = Vec::new();
                let mut tool_results = Vec::new();

                for block in &msg.content {
                    match block {
                        ContentBlock::Text { text } => text_parts.push(text.clone()),
                        ContentBlock::ToolResult { tool_use_id, content, is_error } => {
                            tool_results.push(serde_json::json!({
                                "role": "tool",
                                "tool_call_id": tool_use_id,
                                "content": if is_error.unwrap_or(false) {
                                    format!("Error: {content}")
                                } else {
                                    content.clone()
                                },
                            }));
                        }
                        _ => {}
                    }
                }

                // Emit text parts as a user message
                if !text_parts.is_empty() {
                    messages.push(serde_json::json!({
                        "role": "user",
                        "content": text_parts.join("\n"),
                    }));
                }
                // Emit tool results as separate "tool" role messages
                messages.extend(tool_results);
            }
            "assistant" => {
                // Assistant messages: may contain text and/or tool_use blocks
                let mut text_parts = Vec::new();
                let mut tool_calls = Vec::new();

                for block in &msg.content {
                    match block {
                        ContentBlock::Text { text } => text_parts.push(text.clone()),
                        ContentBlock::ToolUse { id, name, input } => {
                            let arguments = match args_wire {
                                ToolArgsWire::JsonString => {
                                    serde_json::Value::String(input.to_string())
                                }
                                // Ollama native: pass the object through. A
                                // non-object input would 400 the whole request,
                                // so degrade it to {} rather than lose the turn.
                                ToolArgsWire::JsonObject if input.is_object() => input.clone(),
                                ToolArgsWire::JsonObject => {
                                    warn!(
                                        tool = %name,
                                        "tool_use input is not a JSON object; sending {{}} to Ollama"
                                    );
                                    serde_json::json!({})
                                }
                            };
                            tool_calls.push(serde_json::json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": arguments,
                                }
                            }));
                        }
                        _ => {}
                    }
                }

                let mut assistant_msg = serde_json::json!({ "role": "assistant" });
                // Always include `content` — some OpenAI-compatible providers
                // (e.g. Arcee AI via OpenRouter) reject messages where it's missing,
                // even when `tool_calls` is present.
                if text_parts.is_empty() {
                    assistant_msg["content"] = serde_json::Value::Null;
                } else {
                    assistant_msg["content"] = serde_json::json!(text_parts.join("\n"));
                }
                if !tool_calls.is_empty() {
                    assistant_msg["tool_calls"] = serde_json::json!(tool_calls);
                }
                messages.push(assistant_msg);
            }
            _ => {}
        }
    }

    // Convert tools to OpenAI format
    let tools_json = request.tools.as_ref().and_then(|tools| {
        if tools.is_empty() {
            None
        } else {
            let converted: Vec<serde_json::Value> = tools.iter().map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            }).collect();
            Some(serde_json::json!(converted))
        }
    });

    (serde_json::json!(messages), tools_json)
}

/// Convert an OpenAI chat completion response JSON to AnthropicResponse
fn openai_response_to_anthropic(model: &str, response: &serde_json::Value) -> Result<AnthropicResponse, LlmError> {
    let choice = response["choices"]
        .as_array()
        .and_then(|c| c.first())
        .ok_or_else(|| LlmError::Api { status: 500, message: "No choices in response".to_string() })?;

    let message = &choice["message"];
    let mut content = Vec::new();

    // Text content
    if let Some(text) = message["content"].as_str() {
        if !text.is_empty() {
            content.push(ContentBlock::Text { text: text.to_string() });
        }
    }

    // Tool calls
    if let Some(tool_calls) = message["tool_calls"].as_array() {
        for tc in tool_calls {
            let id = tc["id"].as_str().unwrap_or("").to_string();
            let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
            let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
            let input = heal::heal_tool_args(args_str);
            content.push(ContentBlock::ToolUse { id, name, input });
        }
    }

    // Map finish_reason
    let finish_reason = choice["finish_reason"].as_str().unwrap_or("end_turn");
    let stop_reason = match finish_reason {
        "tool_calls" => "tool_use",
        "stop" => "end_turn",
        "length" => "max_tokens",
        other => other,
    };

    let input_tokens = response["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32;
    let output_tokens = response["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32;
    // OpenAI returns cached tokens in usage.prompt_tokens_details.cached_tokens
    let cache_read_input_tokens = response["usage"]["prompt_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0) as u32;

    Ok(AnthropicResponse {
        id: response["id"].as_str().unwrap_or("").to_string(),
        response_type: "message".to_string(),
        role: "assistant".to_string(),
        content,
        model: model.to_string(),
        stop_reason: Some(stop_reason.to_string()),
        usage: Usage { input_tokens, output_tokens, cache_creation_input_tokens: 0, cache_read_input_tokens },
    })
}

/// Convert an Ollama /api/chat response JSON to AnthropicResponse
/// Extract content inside `<think>...</think>` tags.
fn extract_think_content(text: &str) -> Option<String> {
    let start = text.find("<think>")?;
    let end = text.find("</think>")?;
    if end > start + 7 {
        Some(text[start + 7..end].trim().to_string())
    } else {
        None
    }
}

/// Strip all `<think>...</think>` blocks from text, returning the remaining content.
fn strip_think_tags(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;
    while let Some(start) = remaining.find("<think>") {
        result.push_str(&remaining[..start]);
        if let Some(end) = remaining[start..].find("</think>") {
            remaining = &remaining[start + end + 8..];
        } else {
            // Unclosed <think> tag — discard the rest (model is still thinking)
            break;
        }
    }
    result.push_str(remaining);
    result.trim().to_string()
}

fn ollama_response_to_anthropic(model: &str, response: &serde_json::Value) -> Result<AnthropicResponse, LlmError> {
    let message = &response["message"];
    let mut content = Vec::new();

    // Thinking content (qwen3 and other thinking models — Ollama separates this
    // into a `thinking` field when `think: true` is set in the request)
    if let Some(thinking) = message["thinking"].as_str() {
        if !thinking.is_empty() {
            content.push(ContentBlock::Thinking { thinking: thinking.to_string(), signature: None });
        }
    }

    // Text content — strip embedded <think>...</think> tags (fallback for when
    // Ollama doesn't separate thinking, e.g. older versions or models that embed tags)
    if let Some(raw_text) = message["content"].as_str() {
        // Extract thinking from tags if we didn't get it from the dedicated field
        if !content.iter().any(|b| matches!(b, ContentBlock::Thinking { .. })) {
            if let Some(thinking) = extract_think_content(raw_text) {
                if !thinking.is_empty() {
                    content.push(ContentBlock::Thinking { thinking, signature: None });
                }
            }
        }
        let text = strip_think_tags(raw_text);
        if !text.is_empty() {
            content.push(ContentBlock::Text { text });
        }
    }

    // Tool calls (Ollama v0.5+ format)
    if let Some(tool_calls) = message["tool_calls"].as_array() {
        for (i, tc) in tool_calls.iter().enumerate() {
            let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
            let input = tc["function"]["arguments"].clone();
            // Ollama doesn't provide tool call IDs, generate one
            let id = format!("toolu_{:08x}", i);
            content.push(ContentBlock::ToolUse { id, name, input });
        }
    }

    // Determine stop reason
    let done_reason = response["done_reason"].as_str().unwrap_or("stop");
    let stop_reason = if content.iter().any(|b| matches!(b, ContentBlock::ToolUse { .. })) {
        "tool_use"
    } else {
        match done_reason {
            "stop" => "end_turn",
            "length" => "max_tokens",
            other => other,
        }
    };

    // Ollama provides eval_count, prompt_eval_count
    let input_tokens = response["prompt_eval_count"].as_u64().unwrap_or(0) as u32;
    let output_tokens = response["eval_count"].as_u64().unwrap_or(0) as u32;

    Ok(AnthropicResponse {
        id: format!("ollama-{}", current_timestamp()),
        response_type: "message".to_string(),
        role: "assistant".to_string(),
        content,
        model: model.to_string(),
        stop_reason: Some(stop_reason.to_string()),
        usage: Usage { input_tokens, output_tokens, cache_creation_input_tokens: 0, cache_read_input_tokens: 0 },
    })
}

/// Extract a complete SSE event from the buffer.
fn extract_sse_event(buffer: &mut String) -> Option<String> {
    // SSE events are separated by double newlines
    buffer.find("\n\n").map(|pos| {
        let event = buffer[..pos].to_string();
        *buffer = buffer[pos + 2..].to_string();
        event
    })
}

/// Parse an SSE event string into a `StreamEvent`
fn parse_sse_event(event: &str) -> Option<StreamEvent> {
    // Extract data field from SSE event (we determine event type from JSON structure)
    let data = event
        .lines()
        .find_map(|line| line.strip_prefix("data: ").map(str::trim))?;

    // Parse the JSON data (skip malformed events)
    let sse = match serde_json::from_str::<AnthropicSSE>(data) {
        Ok(sse) => sse,
        Err(e) => {
            let end = data.floor_char_boundary(data.len().min(120));
            debug!(error = %e, data_prefix = &data[..end], "Skipping unrecognised SSE event");
            return None;
        }
    };
    match sse {
        AnthropicSSE::MessageStart { message } => {
            let usage = message.usage.unwrap_or_default();
            Some(StreamEvent::MessageStart {
                id: message.id,
                model: message.model,
                input_tokens: usage.input_tokens,
                cache_read_tokens: usage.cache_read_input_tokens,
                cache_creation_tokens: usage.cache_creation_input_tokens,
            })
        }
        AnthropicSSE::ContentBlockStart { index, content_block } => {
            Some(StreamEvent::ContentBlockStart {
                index,
                content_type: content_block.block_type,
                tool_id: content_block.id,
                tool_name: content_block.name,
            })
        }
        AnthropicSSE::ContentBlockDelta { index, delta } => match delta {
            DeltaData::TextDelta { text } => Some(StreamEvent::TextDelta { index, text }),
            DeltaData::ThinkingDelta { thinking } => Some(StreamEvent::ThinkingDelta { index, thinking }),
            DeltaData::InputJsonDelta { partial_json } => {
                Some(StreamEvent::ToolUseDelta { index, partial_json })
            }
            DeltaData::SignatureDelta { signature } => {
                Some(StreamEvent::SignatureDelta { index, signature })
            }
            DeltaData::Unknown => {
                debug!(index, "Ignoring unknown content_block_delta type");
                None
            }
        },
        AnthropicSSE::ContentBlockStop { index } => Some(StreamEvent::ContentBlockStop { index }),
        AnthropicSSE::MessageDelta { delta, usage } => Some(StreamEvent::MessageDelta {
            stop_reason: delta.stop_reason,
            output_tokens: usage.map_or(0, |u| u.output_tokens),
        }),
        AnthropicSSE::MessageStop => Some(StreamEvent::MessageStop {
            stop_reason: "end_turn".to_string(),
        }),
        AnthropicSSE::Ping => Some(StreamEvent::Ping),
        AnthropicSSE::Error { error } => Some(StreamEvent::Error {
            message: error.message,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_construction() {
        let msg = Message::user("Hello");
        assert!(matches!(msg.role, Role::User));
        assert_eq!(msg.content, "Hello");
    }

    /// Request with an echoed tool call in history — the shape that follows
    /// every tool execution in the agent loop.
    fn request_with_tool_history(input: serde_json::Value) -> AnthropicRequest {
        AnthropicRequest {
            model: "ornith:latest".to_string(),
            messages: vec![
                AnthropicMessage::user_text("check the lock file"),
                AnthropicMessage::assistant(vec![ContentBlock::ToolUse {
                    id: "call_1".to_string(),
                    name: "read_file".to_string(),
                    input,
                }]),
                AnthropicMessage::user(vec![ContentBlock::ToolResult {
                    tool_use_id: "call_1".to_string(),
                    content: "NO_LOCK".to_string(),
                    is_error: None,
                }]),
            ],
            max_tokens: 128,
            temperature: None,
            system: None,
            tools: None,
            stream: None,
            thinking: None,
            cache_control: None,
        }
    }

    fn first_tool_arguments(messages: &serde_json::Value) -> serde_json::Value {
        messages
            .as_array()
            .unwrap()
            .iter()
            .find_map(|m| m.get("tool_calls"))
            .and_then(|tc| tc.as_array())
            .and_then(|tc| tc.first())
            .map(|tc| tc["function"]["arguments"].clone())
            .expect("converted request must contain a tool_call")
    }

    #[test]
    fn openai_conversion_keeps_tool_arguments_as_json_string() {
        // OpenAI-compatible /v1/chat/completions requires the string form.
        let request = request_with_tool_history(serde_json::json!({"path": "D:/x"}));
        let (messages, _) = anthropic_to_openai_request(&request);
        let args = first_tool_arguments(&messages);
        assert_eq!(args, serde_json::json!("{\"path\":\"D:/x\"}"));
    }

    #[test]
    fn ollama_conversion_keeps_tool_arguments_as_object() {
        // Ollama's native /api/chat 400s on string-form arguments — even valid
        // JSON inside a string ("Value looks like object, but can't find
        // closing '}' symbol", verified live against Ollama 0.31.2).
        let request = request_with_tool_history(serde_json::json!({"path": "D:/x"}));
        let (messages, _) = anthropic_to_ollama_request(&request);
        let args = first_tool_arguments(&messages);
        assert_eq!(args, serde_json::json!({"path": "D:/x"}));
    }

    #[test]
    fn ollama_conversion_degrades_non_object_input_to_empty_object() {
        // A non-object input would 400 the whole request on the native
        // endpoint; losing the arguments beats losing the turn.
        let request = request_with_tool_history(serde_json::json!("not an object"));
        let (messages, _) = anthropic_to_ollama_request(&request);
        let args = first_tool_arguments(&messages);
        assert_eq!(args, serde_json::json!({}));
    }

    #[test]
    fn test_estimate_tokens_ascii_and_cjk() {
        assert_eq!(estimate_tokens(""), 0);
        // ASCII English: ~4 chars/token, rounded up so short strings aren't zero.
        assert_eq!(estimate_tokens("hi"), 1);
        assert_eq!(estimate_tokens(&"a".repeat(8)), 2);
        // CJK is denser: 4 chars ~ 4 tokens (byte-len/4 would wrongly give 3).
        let cjk = "你好世界"; // 4 chars, 12 UTF-8 bytes
        assert_eq!(cjk.len(), 12);
        assert_eq!(estimate_tokens(cjk), 4);
        // Mixed: 2 ASCII (→1) + 2 wide (→2) = 3.
        assert_eq!(estimate_tokens("hi你好"), 3);
    }

    #[test]
    fn test_estimate_tokens_code_vs_english_ratio() {
        // Pure prose ASCII letter run → English ratio (4 chars/token).
        let prose = "abcdefgh"; // 8 chars
        assert_eq!(
            estimate_tokens_for_family(prose, TokenContentFamily::English),
            2
        );
        // Same length treated as code → denser 3.5 chars/token → ceil(8/3.5)=3.
        assert_eq!(
            estimate_tokens_for_family(prose, TokenContentFamily::Code),
            3
        );
        // Auto-detect: punctuation-heavy JSON should pick code path.
        let jsonish = r#"{"a":1,"b":2,"c":3,"d":4}"#; // lots of braces/colons
        let auto = estimate_tokens_for_family(jsonish, TokenContentFamily::Auto);
        let as_code = estimate_tokens_for_family(jsonish, TokenContentFamily::Code);
        assert_eq!(auto, as_code);
    }

    #[test]
    fn test_estimate_request_tokens_adds_message_framing() {
        // Two 8-char ASCII messages: 2 tokens content each + framing per message.
        let request = CompletionRequest::default()
            .with_message(Message::user("aaaaaaaa"))
            .with_message(Message::user("bbbbbbbb"));
        let expected = 2 * (2 + MESSAGE_FRAMING_TOKENS);
        assert_eq!(estimate_request_tokens(&request), expected);
    }

    #[test]
    fn test_estimate_tokens_exact_smoke() {
        // Empty always 0; non-empty path must agree with some positive count
        // (exact under the tiktoken feature, heuristic otherwise).
        assert_eq!(estimate_tokens_exact(""), 0);
        let n = estimate_tokens_exact("hello world");
        assert!(n >= 1, "expected at least 1 token, got {n}");
        // Model-hinted path should also return a positive count.
        let m = estimate_tokens_for_model("hello world", "gpt-4o");
        assert!(m >= 1, "expected at least 1 token for gpt-4o, got {m}");
    }

    #[test]
    fn test_request_builder() {
        let request = CompletionRequest::default()
            .with_model("gpt-4")
            .with_message(Message::user("Hi"))
            .with_max_tokens(1000)
            .with_temperature(0.5);

        assert_eq!(request.model, "gpt-4");
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.max_tokens, Some(1000));
        assert_eq!(request.temperature, Some(0.5));
    }

    #[test]
    fn test_delta_data_unknown_variant() {
        // Truly unknown delta types should deserialize to Unknown
        let json = r#"{"type": "some_future_delta", "data": "xyz"}"#;
        let delta: DeltaData = serde_json::from_str(json).unwrap();
        assert!(matches!(delta, DeltaData::Unknown));
    }

    #[test]
    fn test_delta_data_signature_variant() {
        // signature_delta should now be recognized (required for multi-turn thinking)
        let json = r#"{"type": "signature_delta", "signature": "abc123"}"#;
        let delta: DeltaData = serde_json::from_str(json).unwrap();
        assert!(matches!(delta, DeltaData::SignatureDelta { .. }));
        if let DeltaData::SignatureDelta { signature } = delta {
            assert_eq!(signature, "abc123");
        }
    }

    #[test]
    fn test_delta_data_known_variants() {
        let text = r#"{"type": "text_delta", "text": "hello"}"#;
        let delta: DeltaData = serde_json::from_str(text).unwrap();
        assert!(matches!(delta, DeltaData::TextDelta { .. }));

        let json_delta = r#"{"type": "input_json_delta", "partial_json": "{\"key\":"}"#;
        let delta: DeltaData = serde_json::from_str(json_delta).unwrap();
        assert!(matches!(delta, DeltaData::InputJsonDelta { .. }));
    }

    #[test]
    fn test_content_block_delta_with_signature_delta() {
        // signature_delta should parse as SignatureDelta now
        let json = r#"{"type": "content_block_delta", "index": 1, "delta": {"type": "signature_delta", "signature": "sig"}}"#;
        let sse: AnthropicSSE = serde_json::from_str(json).unwrap();
        match sse {
            AnthropicSSE::ContentBlockDelta { index, delta } => {
                assert_eq!(index, 1);
                assert!(matches!(delta, DeltaData::SignatureDelta { .. }));
            }
            other => panic!("Expected ContentBlockDelta, got {:?}", other),
        }
    }

    #[test]
    fn test_content_block_delta_with_unknown_delta() {
        // Truly unknown delta types should still parse via Unknown fallback
        let json = r#"{"type": "content_block_delta", "index": 2, "delta": {"type": "future_delta_type"}}"#;
        let sse: AnthropicSSE = serde_json::from_str(json).unwrap();
        match sse {
            AnthropicSSE::ContentBlockDelta { index, delta } => {
                assert_eq!(index, 2);
                assert!(matches!(delta, DeltaData::Unknown));
            }
            other => panic!("Expected ContentBlockDelta, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_sse_event_signature_delta() {
        let event = "data: {\"type\": \"content_block_delta\", \"index\": 0, \"delta\": {\"type\": \"signature_delta\", \"signature\": \"abc\"}}";
        // Should now return a SignatureDelta event
        let result = parse_sse_event(event);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), StreamEvent::SignatureDelta { index: 0, .. }));
    }

    #[test]
    fn test_parse_sse_event_message_start_carries_usage() {
        // message_start reports prompt-side usage incl. cache read/write — the
        // streaming path must surface these for accurate cache-cost tracking.
        let event = "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-sonnet-4\",\"usage\":{\"input_tokens\":12,\"cache_read_input_tokens\":900,\"cache_creation_input_tokens\":34}}}";
        let result = parse_sse_event(event).expect("message_start should parse");
        match result {
            StreamEvent::MessageStart {
                input_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                ..
            } => {
                assert_eq!(input_tokens, 12);
                assert_eq!(cache_read_tokens, 900);
                assert_eq!(cache_creation_tokens, 34);
            }
            other => panic!("expected MessageStart, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_sse_event_message_start_without_usage_is_zero() {
        // Missing usage (or a non-Anthropic shape) must default to zeros, not fail.
        let event = "data: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"model\":\"x\"}}";
        let result = parse_sse_event(event).expect("message_start should parse");
        assert!(matches!(
            result,
            StreamEvent::MessageStart {
                input_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                ..
            }
        ));
    }

    fn model_info(context_window: usize, max_output_tokens: usize) -> ModelInfo {
        ModelInfo {
            id: "test".into(),
            context_window,
            max_output_tokens,
            supports_tools: false,
            supports_vision: false,
            embedding_dimension: None,
            cached_at: 0,
            provider: "test".into(),
        }
    }

    #[test]
    fn compression_threshold_stays_below_hard_limit_for_small_models() {
        // Small model, large output budget: 80%-of-context (6400) would exceed
        // the hard input cap (4000), so proactive compression must be pulled
        // below it instead of never firing.
        let small = model_info(8_000, 4_096);
        assert!(
            small.compression_threshold() < small.hard_input_limit(),
            "threshold {} must be below hard limit {}",
            small.compression_threshold(),
            small.hard_input_limit()
        );
    }

    #[test]
    fn compression_threshold_unchanged_for_large_models() {
        // Large context model: 80%-of-context is already the smaller value, so
        // the fix must not perturb it (no regression for cloud models).
        let large = model_info(200_000, 8_192);
        assert_eq!(large.compression_threshold(), 160_000);
        assert!(large.compression_threshold() < large.hard_input_limit());
    }

    #[test]
    fn compression_threshold_holds_when_output_exceeds_context() {
        // Degenerate provider report (max_output >= context): hard_input_limit
        // floors at 50% of context; the threshold must still sit at/below it.
        let degenerate = model_info(4_000, 8_000);
        assert!(degenerate.compression_threshold() <= degenerate.hard_input_limit());
    }

    #[test]
    fn model_context_window_uses_universal_floor_without_cache() {
        // No per-model table: uncached names all inherit the conservative floor.
        assert_eq!(model_context_window("claude-opus-4-8"), UNKNOWN_CONTEXT_WINDOW);
        assert_eq!(model_context_window("gpt-4"), UNKNOWN_CONTEXT_WINDOW);
        assert_eq!(model_context_window("some-unknown-model"), UNKNOWN_CONTEXT_WINDOW);
        let info = unknown_model_info("x", "test");
        assert_eq!(info.context_window, UNKNOWN_CONTEXT_WINDOW);
        assert_eq!(info.max_output_tokens, UNKNOWN_MAX_OUTPUT_TOKENS);
    }

    #[test]
    fn conversation_history_budget_respects_hard_limit() {
        let small = model_info(8_000, 4_096);
        let budget = small.conversation_history_budget(2_000, 2_000, 1_000);
        assert!(budget <= small.hard_input_limit());
        assert!(budget >= 1_000);
        let tiny = model_info(2_000, 1_000);
        // min_history must not exceed hard limit
        assert_eq!(
            tiny.conversation_history_budget(10_000, 10_000, 5_000),
            tiny.hard_input_limit()
        );
    }
}
