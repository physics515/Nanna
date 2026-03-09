//! Agent Service - Wraps nanna-agent for the daemon
//!
//! Handles LLM calls, streaming, and tool execution.

use crate::llm_router::LlmRouter;
use crate::protocol::Event;
use crate::session::{MessageRole, SessionId, SessionMessage, ToolCallRecord};
use nanna_agent::{Agent, AgentConfig, ModelTier, RunOptions, ThinkingMode};
use nanna_llm::{AnthropicMessage, ModelInfo, ModelInfoCache};
use nanna_memory::MemoryService;
use nanna_tools::ToolRegistry;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{info, warn};

/// Per-session queue that serializes chat processing.
/// Tokio's Mutex is FIFO-fair, so messages are processed in arrival order.
struct SessionQueue {
    /// Serializes chat processing — tokio Mutex is FIFO-fair
    lock: tokio::sync::Mutex<()>,
    /// Messages waiting + currently processing (for frontend visibility)
    depth: AtomicUsize,
}

/// Configuration for the agent service
#[derive(Debug, Clone)]
pub struct AgentServiceConfig {
    /// Default model to use
    pub model: String,
    /// Model priority list for fallback (e.g., ["claude-sonnet-4-20250514", "claude-3-5-sonnet-20241022"])
    pub model_priority: Vec<String>,
    /// Maximum tokens per response
    pub max_tokens: u32,
    /// Temperature for sampling
    pub temperature: f32,
    /// Maximum tool iterations per request. None = unlimited.
    pub max_iterations: Option<usize>,
    /// Default thinking mode
    pub thinking_mode: ThinkingMode,
    /// Model priority list for summarization
    pub summarization_priority: Vec<String>,
    /// Ollama URL for summarization
    pub summarization_ollama_url: Option<String>,
    /// Model routing priority for cost optimization.
    /// Format: ["model:tier", ...] where tier is simple|medium|complex.
    pub model_routing: Vec<String>,
    /// Whether to use primary model for first iteration
    pub routing_first_turn_primary: bool,
}

impl Default for AgentServiceConfig {
    fn default() -> Self {
        Self {
            model: String::new(), // Populated from config at startup
            model_priority: vec![], // Dynamically built from available credentials
            max_tokens: 8192,
            temperature: 0.7,
            max_iterations: None, // Unlimited iterations — agent stops when done
            thinking_mode: ThinkingMode::Instant,
            summarization_priority: vec![],
            summarization_ollama_url: Some("http://localhost:11434".to_string()),
            model_routing: vec![],
            routing_first_turn_primary: true,
        }
    }
}

/// Active chat request state
struct ActiveChat {
    #[allow(dead_code)]
    // Used for future session tracking features
    session_id: SessionId,
    cancelled: bool,
    /// Shared cancellation flag passed into the agent loop for cooperative cancellation
    cancellation_flag: Arc<AtomicBool>,
    started_at: chrono::DateTime<chrono::Utc>,
    /// Accumulated streamed text (shared with on_text callback)
    accumulated_text: Arc<tokio::sync::RwLock<String>>,
    /// Tool calls currently in progress
    active_tool_calls: Arc<tokio::sync::RwLock<Vec<ActiveToolCallInfo>>>,
    /// Tool calls completed during this run (before final message)
    completed_tool_calls: Arc<tokio::sync::RwLock<Vec<CompletedToolCallInfo>>>,
}

/// Info about a tool call currently executing
#[derive(Debug, Clone, serde::Serialize)]
pub struct ActiveToolCallInfo {
    pub call_id: String,
    pub name: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
}

/// Info about a tool call that completed during the current run
#[derive(Debug, Clone, serde::Serialize)]
pub struct CompletedToolCallInfo {
    pub call_id: String,
    pub name: String,
    pub output: String,
    pub success: bool,
    pub duration_ms: u64,
}

/// Snapshot of the current run state for a session
#[derive(Debug, Clone, serde::Serialize)]
pub struct RunStateSnapshot {
    pub is_running: bool,
    pub accumulated_text: String,
    pub active_tool_calls: Vec<ActiveToolCallInfo>,
    pub completed_tool_calls: Vec<CompletedToolCallInfo>,
    pub started_at: Option<String>,
    /// Total completed messages in session (for sync verification)
    pub message_count: usize,
    /// ID of the last completed message (for sync verification)
    pub last_message_id: Option<String>,
    /// Messages queued behind the current one at the daemon level
    pub queued_count: usize,
}

/// The agent service that handles LLM interactions
pub struct AgentService {
    config: Arc<RwLock<AgentServiceConfig>>,
    router: Arc<LlmRouter>,
    tools: Arc<ToolRegistry>,
    memory: Option<Arc<MemoryService>>,
    /// Event broadcaster for streaming to clients
    event_tx: broadcast::Sender<Event>,
    /// Currently active chats (session_id -> state)
    active_chats: Arc<RwLock<HashMap<SessionId, ActiveChat>>>,
    #[allow(dead_code)]
    // Reserved for future model caching optimization
    model_cache: Option<ModelInfoCache>,
    /// Cached model info for current model
    current_model_info: Arc<RwLock<Option<ModelInfo>>>,
    /// Per-session message queues for serialized chat processing
    session_queues: Arc<RwLock<HashMap<String, Arc<SessionQueue>>>>,
}

impl AgentService {
    /// Create a new agent service
    pub fn new(
        config: AgentServiceConfig,
        router: Arc<LlmRouter>,
        tools: Arc<ToolRegistry>,
        memory: Option<Arc<MemoryService>>,
        event_tx: broadcast::Sender<Event>,
    ) -> Self {
        // Initialize model info cache in default location
        let model_cache = ModelInfoCache::default_location();
        if model_cache.is_none() {
            warn!("Could not determine cache directory for model info");
        }

        Self {
            config: Arc::new(RwLock::new(config)),
            router,
            tools,
            memory,
            event_tx,
            active_chats: Arc::new(RwLock::new(HashMap::new())),
            model_cache,
            current_model_info: Arc::new(RwLock::new(None)),
            session_queues: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get model info for the current model, fetching if needed
    pub async fn get_model_info(&self) -> ModelInfo {
        let current_model = self.config.read().await.model.clone();

        // Check if we already have cached info for this model
        {
            let cached = self.current_model_info.read().await;
            if let Some(ref info) = *cached {
                if info.id == current_model && !info.is_expired() {
                    return info.clone();
                }
            }
        }

        // Fetch new info via router
        let info = self.router.get_model_info(&current_model).await;

        info!(
            model = %info.id,
            context_window = info.context_window,
            max_output = info.max_output_tokens,
            provider = %info.provider,
            "Fetched model info"
        );

        // Cache it
        {
            let mut cached = self.current_model_info.write().await;
            *cached = Some(info.clone());
        }

        info
    }

    /// Update model configuration at runtime (hot-reload from control plane)
    pub async fn update_config(&self, model: Option<String>, model_priority: Option<Vec<String>>) {
        let mut config = self.config.write().await;
        if let Some(m) = model {
            if config.model != m {
                info!(old = %config.model, new = %m, "Switching model");
                config.model = m;
            }
        }
        if let Some(p) = model_priority {
            info!(new_priority = ?p, "Updating model priority list");
            config.model_priority = p;
        }
        drop(config);

        // Clear cached model info to force refresh on next request
        let mut cached = self.current_model_info.write().await;
        *cached = None;
    }
    
    /// Create agent config from service config
    async fn agent_config(&self) -> AgentConfig {
        let config = self.config.read().await;
        AgentConfig {
            model: config.model.clone(),
            max_tokens: config.max_tokens,
            temperature: config.temperature,
            max_iterations: config.max_iterations,
            thinking_mode: config.thinking_mode,
            summarization_priority: config.summarization_priority.clone(),
            summarization_ollama_url: config.summarization_ollama_url.clone(),
            model_routing: config.model_routing.iter().map(|s| ModelTier::parse(s)).collect(),
            routing_first_turn_primary: config.routing_first_turn_primary,
            ..Default::default()
        }
    }
    
    /// Get or create the per-session queue for serializing chat processing
    async fn get_or_create_queue(&self, session_id: &str) -> Arc<SessionQueue> {
        // Fast path: read lock
        {
            let queues = self.session_queues.read().await;
            if let Some(queue) = queues.get(session_id) {
                return queue.clone();
            }
        }
        // Slow path: write lock to insert
        let mut queues = self.session_queues.write().await;
        queues
            .entry(session_id.to_string())
            .or_insert_with(|| {
                Arc::new(SessionQueue {
                    lock: tokio::sync::Mutex::new(()),
                    depth: AtomicUsize::new(0),
                })
            })
            .clone()
    }

    /// Get the current queue depth for a session (messages waiting + processing)
    pub async fn queue_depth(&self, session_id: &str) -> usize {
        let queues = self.session_queues.read().await;
        queues
            .get(session_id)
            .map(|q| q.depth.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Run a chat completion with automatic model fallback.
    ///
    /// `history` should contain all prior session messages **excluding** the
    /// current user message (which is passed as `message`). The agent loop
    /// appends `message` itself, so we only pre-populate the older turns.
    pub async fn chat(
        &self,
        session_id: &str,
        message: &str,
        system_prompt: Option<String>,
        history: &[SessionMessage],
    ) -> Result<ChatResult, String> {
        // Acquire per-session queue slot (FIFO ordering via tokio Mutex)
        let queue = self.get_or_create_queue(session_id).await;
        queue.depth.fetch_add(1, Ordering::Relaxed);

        // Wait for our turn — if another chat is running on this session,
        // we block here until it completes. Tokio Mutex is FIFO-fair.
        let _guard = queue.lock.lock().await;

        // Create shared state buffers for run state tracking
        let accumulated = Arc::new(tokio::sync::RwLock::new(String::new()));
        let active_tools = Arc::new(tokio::sync::RwLock::new(Vec::<ActiveToolCallInfo>::new()));
        let completed_tools = Arc::new(tokio::sync::RwLock::new(Vec::<CompletedToolCallInfo>::new()));
        let cancellation_flag = Arc::new(AtomicBool::new(false));

        // Register this chat as active (for streaming state tracking)
        {
            let mut active = self.active_chats.write().await;
            active.insert(session_id.to_string(), ActiveChat {
                session_id: session_id.to_string(),
                cancelled: false,
                cancellation_flag: cancellation_flag.clone(),
                started_at: chrono::Utc::now(),
                accumulated_text: accumulated.clone(),
                active_tool_calls: active_tools.clone(),
                completed_tool_calls: completed_tools.clone(),
            });
        }

        let message_id = uuid::Uuid::new_v4().to_string();

        // Emit start event
        let _ = self.event_tx.send(Event::MessageStart {
            session_id: session_id.to_string(),
            message_id: message_id.clone(),
        });

        // Build model list to try (priority list, falling back to single model).
        // This is rebuilt fresh on every chat() call — no persistent "last successful model"
        // state. Each invocation (including heartbeats) always starts from the top of the
        // priority list, so transient failures don't permanently shift the preferred model.
        let base_models: Vec<String> = {
            let config = self.config.read().await;
            if config.model_priority.is_empty() {
                vec![config.model.clone()]
            } else {
                config.model_priority.clone()
            }
        };

        // Apply health-aware reordering: healthy models first, skip unhealthy ones
        let models_to_try = self.router.health_sorted_models(&base_models).await;

        let mut last_error = String::from("No models available");
        let mut tried_models = Vec::new();

        for model in &models_to_try {
            tried_models.push(model.clone());
            info!("Trying model: {} (attempt {})", model, tried_models.len());

            // Notify clients which model we're using
            let fallback_reason = if tried_models.len() > 1 {
                Some(last_error.clone())
            } else {
                None
            };
            let _ = self.event_tx.send(Event::ModelSwitch {
                model: model.clone(),
                reason: fallback_reason,
            });

            // Get the correct LLM client for this model from the router
            let llm_client = match self.router.client_for_model(model) {
                Some(client) => client,
                None => {
                    let detected = crate::llm_router::ProviderId::from_model(model);
                    warn!(
                        "No provider for model: {} (detected: {:?}, available: {:?})",
                        model, detected, self.router.available_providers()
                    );
                    last_error = format!("No provider for model: {} (detected: {:?})", model, detected);
                    continue;
                }
            };

            // Create agent config with this model
            let mut agent_config = self.agent_config().await;
            agent_config.model = LlmRouter::strip_model_prefix(model);

            // Get model info for context configuration
            let model_info = self.router.get_model_info(model).await;

            // Create agent with context configured for the model
            let agent = {
                let mut context = nanna_agent::AgentContext::new(session_id.to_string());

                // Configure context limits based on model capabilities
                context.configure_for_model(&model_info);

                // Set system prompt if provided
                if let Some(ref prompt) = system_prompt {
                    context = context.with_system_prompt(prompt);
                }

                // Load prior conversation history into context
                for msg in history {
                    let anthropic_msg = match msg.role {
                        MessageRole::User => AnthropicMessage::user_text(&msg.content),
                        MessageRole::Assistant => AnthropicMessage::assistant_text(&msg.content),
                        // System/Tool messages are not standard conversation turns;
                        // skip them to avoid confusing the LLM message alternation
                        MessageRole::System | MessageRole::Tool => continue,
                    };
                    context.messages.push(anthropic_msg);
                }

                Agent::new(
                    agent_config,
                    llm_client,
                    self.tools.clone(),
                )
                .with_context(context)
            };

            // Create run options with streaming callbacks
            let session_id_for_stream = session_id.to_string();
            let event_tx = self.event_tx.clone();

            // Clone memory service for auto-extraction callback
            let memory_for_extraction = self.memory.clone();
            let has_memory = memory_for_extraction.is_some();

            let accumulated_for_cb = accumulated.clone();
            let event_tx_thinking = self.event_tx.clone();
            let session_id_thinking = session_id.to_string();
            let event_tx_tool_start = self.event_tx.clone();
            let session_id_tool_start = session_id.to_string();
            let active_tools_for_cb = active_tools.clone();
            let options = RunOptions {
                cancellation_flag: Some(cancellation_flag.clone()),
                on_text: Some(Box::new(move |chunk: &str| {
                    // Accumulate text for run state recovery
                    if let Ok(mut buf) = accumulated_for_cb.try_write() {
                        buf.push_str(chunk);
                    }
                    let _ = event_tx.send(Event::MessageDelta {
                        session_id: session_id_for_stream.clone(),
                        message_id: String::new(),
                        delta: chunk.to_string(),
                    });
                })),
                on_thinking: Some(Box::new(move |chunk: &str| {
                    let _ = event_tx_thinking.send(Event::ThinkingDelta {
                        session_id: session_id_thinking.clone(),
                        delta: chunk.to_string(),
                    });
                })),
                on_tool_start: Some(Box::new(move |call_id: &str, name: &str, input: &serde_json::Value| {
                    // Track active tool calls for run state recovery
                    if let Ok(mut tools) = active_tools_for_cb.try_write() {
                        tools.push(ActiveToolCallInfo {
                            call_id: call_id.to_string(),
                            name: name.to_string(),
                            started_at: chrono::Utc::now(),
                        });
                    }
                    let _ = event_tx_tool_start.send(Event::ToolStart {
                        session_id: session_id_tool_start.clone(),
                        call_id: call_id.to_string(),
                        name: name.to_string(),
                        input: input.clone(),
                    });
                })),
                // Enable auto-extraction if memory service is available
                auto_extract_memories: has_memory,
                on_memory: if has_memory {
                    Some(Box::new(move |memory: nanna_agent::ExtractedMemory| {
                        let mem_service = memory_for_extraction.clone();
                        Box::pin(async move {
                            if let Some(ref service) = mem_service {
                                let mut metadata = memory.tags.unwrap_or_default();
                                metadata.insert("category".to_string(), memory.category.clone());
                                // Derive importance from category
                                let importance = match memory.category.as_str() {
                                    "tool_result" => 2.0, // Lower importance for ephemeral tool output
                                    "preference" | "identity" => 4.0,
                                    "fact" | "insight" => 3.5,
                                    "context" => 3.0,
                                    _ => 3.0,
                                };
                                if let Err(e) = service.remember_with_importance(&memory.content, metadata, importance).await {
                                    warn!("Failed to auto-store memory: {}", e);
                                } else {
                                    info!("Auto-extracted memory [{}]: {}", memory.category, truncate(&memory.content, 50));
                                }
                            }
                        })
                    }))
                } else {
                    None
                },
                ..Default::default()
            };

            // Run the agent
            let result = agent.run(message, options).await;

            match result {
                Ok(response) => {
                    // Remove from active chats and decrement queue depth
                    {
                        let mut active = self.active_chats.write().await;
                        active.remove(session_id);
                    }
                    queue.depth.fetch_sub(1, Ordering::Relaxed);

                    // Emit tool events and populate completed_tool_calls
                    for tc in &response.tool_calls {
                        if let Ok(mut completed) = completed_tools.try_write() {
                            completed.push(CompletedToolCallInfo {
                                call_id: tc.id.clone(),
                                name: tc.name.clone(),
                                output: tc.output.clone(),
                                success: tc.success,
                                duration_ms: tc.duration_ms,
                            });
                        }
                        let _ = self.event_tx.send(Event::ToolEnd {
                            session_id: session_id.to_string(),
                            call_id: tc.id.clone(),
                            output: tc.output.clone(),
                            success: tc.success,
                            duration_ms: tc.duration_ms,
                        });
                    }

                    // Emit completion event
                    let _ = self.event_tx.send(Event::MessageEnd {
                        session_id: session_id.to_string(),
                        message_id: message_id.clone(),
                        content: response.text.clone(),
                    });

                    info!("Success with model: {}", model);

                    return Ok(ChatResult {
                        message_id,
                        content: response.text,
                        tool_calls: response.tool_calls.into_iter().map(|tc| ToolCallRecord {
                            id: tc.id,
                            name: tc.name,
                            input: tc.input,
                            output: Some(tc.output),
                            success: Some(tc.success),
                            duration_ms: Some(tc.duration_ms),
                        }).collect(),
                        input_tokens: response.input_tokens,
                        output_tokens: response.output_tokens,
                        reasoning: response.reasoning.map(|r| r.content),
                    });
                }
                Err(e) => {
                    let error_str = e.to_string();
                    warn!("Model {} failed: {}", model, error_str);
                    last_error = error_str.clone();

                    // Emit error as a warning so it's visible, but continue trying
                    let _ = self.event_tx.send(Event::Error {
                        code: "model_error".to_string(),
                        message: format!("Model {} failed: {}. Trying next model...", model, error_str),
                    });

                    // Always fall back to next model in priority list
                    info!("Model {} failed, trying next model in priority list", model);
                }
            }
        }

        // Remove from active chats and decrement queue depth
        {
            let mut active = self.active_chats.write().await;
            active.remove(session_id);
        }
        queue.depth.fetch_sub(1, Ordering::Relaxed);

        // All models failed
        let _ = self.event_tx.send(Event::Error {
            code: "all_models_exhausted".to_string(),
            message: format!("All models failed. Tried: {:?}. Last error: {}", tried_models, last_error),
        });
        Err(format!("All models exhausted. Tried: {:?}. Last error: {}", tried_models, last_error))
    }

    /// Check if an error is a rate-limit / transient overload (may warrant retry with backoff)
    #[allow(dead_code)]
    fn is_rate_limit_error(error: &str) -> bool {
        let error_lower = error.to_lowercase();

        // Rate limit errors
        if error_lower.contains("rate limit")
            || error_lower.contains("429")
            || error_lower.contains("too many requests") {
            return true;
        }

        // Overloaded errors
        if error_lower.contains("overloaded")
            || error_lower.contains("529")
            || error_lower.contains("503")
            || error_lower.contains("502") {
            return true;
        }

        // Temporary server errors
        if error_lower.contains("temporarily unavailable")
            || error_lower.contains("service unavailable")
            || error_lower.contains("timeout") {
            return true;
        }

        false
    }
    
    /// Cancel an active chat
    pub async fn cancel(&self, session_id: &str) -> bool {
        let mut active = self.active_chats.write().await;
        if let Some(chat) = active.get_mut(session_id) {
            chat.cancelled = true;
            chat.cancellation_flag.store(true, Ordering::Relaxed);
            true
        } else {
            false
        }
    }
    
    /// Check if a chat is active
    pub async fn is_active(&self, session_id: &str) -> bool {
        let active = self.active_chats.read().await;
        active.contains_key(session_id)
    }

    /// Get a snapshot of the current run state for a session.
    /// Includes sync metadata (message_count, last_message_id) from SessionManager.
    pub async fn get_run_state(
        &self,
        session_id: &str,
        sessions: &crate::session::SessionManager,
    ) -> RunStateSnapshot {
        // Get message count + last ID from session for sync verification
        let (message_count, last_message_id) = if let Some(session) = sessions.get(session_id).await {
            let count = session.messages.len();
            let last_id = session.messages.last().map(|m| m.id.clone());
            (count, last_id)
        } else {
            (0, None)
        };

        // Queue depth: subtract 1 for the currently-processing message (if any)
        let raw_depth = self.queue_depth(session_id).await;

        let active = self.active_chats.read().await;
        match active.get(session_id) {
            Some(chat) => RunStateSnapshot {
                is_running: true,
                accumulated_text: chat.accumulated_text.read().await.clone(),
                active_tool_calls: chat.active_tool_calls.read().await.clone(),
                completed_tool_calls: chat.completed_tool_calls.read().await.clone(),
                started_at: Some(chat.started_at.to_rfc3339()),
                message_count,
                last_message_id,
                queued_count: raw_depth.saturating_sub(1), // exclude the active one
            },
            None => RunStateSnapshot {
                is_running: false,
                accumulated_text: String::new(),
                active_tool_calls: vec![],
                completed_tool_calls: vec![],
                started_at: None,
                message_count,
                last_message_id,
                queued_count: raw_depth, // nothing active, all are waiting
            },
        }
    }
    
    /// Get memory context for a query
    pub async fn recall_memories(&self, query: &str, limit: usize) -> Vec<MemoryContext> {
        if let Some(ref memory) = self.memory {
            match memory.recall(query).await {
                Ok(results) => {
                    results.into_iter()
                        .take(limit)
                        .map(|r| MemoryContext {
                            id: r.id,
                            content: r.content,
                            score: r.score,
                        })
                        .collect()
                }
                Err(e) => {
                    warn!("Memory recall failed: {}", e);
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        }
    }
    
    /// Store a memory
    pub async fn remember(&self, content: &str, metadata: HashMap<String, String>) -> Result<String, String> {
        if let Some(ref memory) = self.memory {
            memory.remember_with_importance(content, metadata, 3.0)
                .await
                .map(|(id, _)| id)
                .map_err(|e| e.to_string())
        } else {
            Err("Memory service not configured".to_string())
        }
    }
}

/// Result of a chat completion
#[derive(Debug, Clone)]
pub struct ChatResult {
    pub message_id: String,
    pub content: String,
    pub tool_calls: Vec<ToolCallRecord>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub reasoning: Option<String>,
}

/// Memory context for injection
#[derive(Debug, Clone)]
pub struct MemoryContext {
    pub id: String,
    pub content: String,
    pub score: f32,
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}
