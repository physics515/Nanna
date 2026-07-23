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
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{info, warn};

/// Same-model retries for transient provider faults in the chat/heartbeat
/// path, before falling down the priority list. Mirrors the task harness's
/// `STEP_LLM_RETRIES`: a single-model priority list (the common local setup)
/// must survive a cold Ollama load, a dropped stream, or a degraded runner
/// without declaring every model exhausted.
const CHAT_TRANSIENT_RETRIES_MAX: usize = 3;

/// Escalating backoff before same-model retries `1..=CHAT_TRANSIENT_RETRIES_MAX`.
const CHAT_RETRY_BACKOFF_SECS: [u64; 3] = [2, 5, 10];

/// A chat message opens mission mode when its first word is MISSION
/// (case-insensitive; optional trailing colon). An explicit prefix — never a
/// heuristic — so ordinary chats can never accidentally run long-horizon.
fn message_is_mission(message: &str) -> bool {
    let first_word: String = message
        .trim_start()
        .chars()
        .take_while(|c| !c.is_whitespace())
        .collect();
    first_word.trim_end_matches(':').eq_ignore_ascii_case("mission")
}

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
    /// Maximum tool iterations per request. None = unlimited (default).
    pub max_iterations: Option<usize>,
    /// Iteration at which the first escalating wrap-up soft nudge is injected (default 500).
    pub nudge_after_iterations: usize,
    /// Interval (iterations) between escalating nudges after the first (default 100).
    pub nudge_interval_iterations: usize,
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
    /// Model to use for sub-agent tasks (optional).
    /// When set, sub-agents use this cheaper model instead of the primary.
    pub sub_agent_model: Option<String>,
    /// OpenRouter API key (passed to agents for summarization/extraction)
    pub openrouter_api_key: Option<String>,
    /// OpenAI API key (passed to agents for summarization/extraction)
    pub openai_api_key: Option<String>,
}

impl Default for AgentServiceConfig {
    fn default() -> Self {
        Self {
            model: String::new(), // Populated from config at startup
            model_priority: vec![], // Dynamically built from available credentials
            max_tokens: 8192,
            temperature: 0.7,
            max_iterations: None, // Unlimited — model stops when done
            nudge_after_iterations: 500,
            nudge_interval_iterations: 100,
            thinking_mode: ThinkingMode::Instant,
            summarization_priority: vec![],
            summarization_ollama_url: Some("http://localhost:11434".to_string()),
            model_routing: vec![],
            routing_first_turn_primary: true,
            sub_agent_model: None,
            openrouter_api_key: None,
            openai_api_key: None,
        }
    }
}

/// Active chat request state
struct ActiveChat {
    // Used for future session tracking features
    _session_id: SessionId,
    cancelled: bool,
    /// Shared cancellation flag passed into the agent loop for cooperative cancellation
    cancellation_flag: Arc<AtomicBool>,
    started_at: chrono::DateTime<chrono::Utc>,
    /// Accumulated streamed text (shared with on_text callback)
    accumulated_text: Arc<tokio::sync::RwLock<String>>,
    /// Accumulated thinking/reasoning text (shared with on_thinking callback)
    accumulated_thinking: Arc<tokio::sync::RwLock<String>>,
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    pub accumulated_thinking: String,
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
    // Reserved for future model caching optimization
    _model_cache: Option<ModelInfoCache>,
    /// Cached model info for current model
    current_model_info: Arc<RwLock<Option<ModelInfo>>>,
    /// Per-session message queues for serialized chat processing
    session_queues: Arc<RwLock<HashMap<String, Arc<SessionQueue>>>>,
    /// Directory for per-iteration checkpoint files (legacy, kept for migration)
    checkpoint_dir: std::path::PathBuf,
    /// Shared session history for the `session.history` tool service.
    /// Populated before each agent run with the current session's messages.
    session_history: Option<crate::server::SharedSessionHistory>,
    /// Database storage for checkpoint persistence
    storage: Option<Arc<nanna_storage::Storage>>,
    /// Shared model-stats tracker. When set, every agent run records its
    /// per-model request stats here so the control plane persists them and
    /// the router reads them for health-aware routing. `None` = don't record.
    model_stats: Option<nanna_agent::ModelStatsTracker>,
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
        Self::with_data_dir(config, router, tools, memory, event_tx, None)
    }

    /// Create a new agent service with a specific data directory for checkpoints.
    pub fn with_data_dir(
        config: AgentServiceConfig,
        router: Arc<LlmRouter>,
        tools: Arc<ToolRegistry>,
        memory: Option<Arc<MemoryService>>,
        event_tx: broadcast::Sender<Event>,
        data_dir: Option<std::path::PathBuf>,
    ) -> Self {
        // Initialize model info cache in default location
        let model_cache = ModelInfoCache::default_location();
        if model_cache.is_none() {
            warn!("Could not determine cache directory for model info");
        }

        // Set up checkpoint directory
        let checkpoint_dir = data_dir
            .unwrap_or_else(|| std::env::temp_dir())
            .join("checkpoints");
        if let Err(e) = std::fs::create_dir_all(&checkpoint_dir) {
            warn!("Failed to create checkpoint directory {:?}: {}", checkpoint_dir, e);
        }

        Self {
            config: Arc::new(RwLock::new(config)),
            router,
            tools,
            memory,
            event_tx,
            active_chats: Arc::new(RwLock::new(HashMap::new())),
            _model_cache: model_cache,
            current_model_info: Arc::new(RwLock::new(None)),
            session_queues: Arc::new(RwLock::new(HashMap::new())),
            checkpoint_dir,
            session_history: None,
            storage: None,
            model_stats: None,
        }
    }

    /// Set the database storage for checkpoint persistence.
    pub fn with_storage(mut self, storage: Arc<nanna_storage::Storage>) -> Self {
        self.storage = Some(storage);
        self
    }

    /// Set the shared session history for the `session.history` tool service.
    pub fn with_session_history(mut self, history: crate::server::SharedSessionHistory) -> Self {
        self.session_history = Some(history);
        self
    }

    /// Set the shared model-stats tracker. Each agent run records into it, so
    /// the main daemon agent's request stats are captured (previously the
    /// `Agent` was built without a tracker and no live stats were recorded).
    #[must_use]
    pub fn with_stats(mut self, stats: nanna_agent::ModelStatsTracker) -> Self {
        self.model_stats = Some(stats);
        self
    }

    /// Get a reference to the tool registry
    pub fn tools(&self) -> &Arc<ToolRegistry> {
        &self.tools
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
    
    /// Create agent config from service config (pub: the long-horizon
    /// step runner builds fresh per-step agents from the same config)
    pub async fn agent_config(&self) -> AgentConfig {
        let config = self.config.read().await;
        AgentConfig {
            model: config.model.clone(),
            max_tokens: config.max_tokens,
            temperature: config.temperature,
            max_iterations: config.max_iterations,
            nudge_after_iterations: config.nudge_after_iterations,
            nudge_interval_iterations: config.nudge_interval_iterations,
            thinking_mode: config.thinking_mode,
            summarization_priority: config.summarization_priority.clone(),
            summarization_ollama_url: config.summarization_ollama_url.clone(),
            openrouter_api_key: config.openrouter_api_key.clone(),
            openai_api_key: config.openai_api_key.clone(),
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
    ) -> Result<ChatResult, ChatError> {
        self.chat_with_options(session_id, message, system_prompt, history, None, None, None, vec![], false).await
    }

    /// Chat with workspace scope for memory extraction
    pub async fn chat_in_workspace(
        &self,
        session_id: &str,
        message: &str,
        system_prompt: Option<String>,
        history: &[SessionMessage],
        workspace_id: Option<String>,
        attachments: Vec<(String, String)>,
    ) -> Result<ChatResult, ChatError> {
        self.chat_with_options(session_id, message, system_prompt, history, None, None, workspace_id, attachments, false).await
    }

    /// Chat with optional model and max_iterations overrides (used by sub-sessions).
    pub async fn chat_with_options(
        &self,
        session_id: &str,
        message: &str,
        system_prompt: Option<String>,
        history: &[SessionMessage],
        model_override: Option<String>,
        max_iterations_override: Option<usize>,
        workspace_id: Option<String>,
        attachments: Vec<(String, String)>,
        is_sub_agent: bool,
    ) -> Result<ChatResult, ChatError> {
        // Acquire per-session queue slot (FIFO ordering via tokio Mutex)
        let queue = self.get_or_create_queue(session_id).await;
        queue.depth.fetch_add(1, Ordering::Relaxed);

        // Wait for our turn — if another chat is running on this session,
        // we block here until it completes. Tokio Mutex is FIFO-fair.
        let _guard = queue.lock.lock().await;

        // Create shared state buffers for run state tracking
        let accumulated = Arc::new(tokio::sync::RwLock::new(String::new()));
        let accumulated_thinking = Arc::new(tokio::sync::RwLock::new(String::new()));
        let active_tools = Arc::new(tokio::sync::RwLock::new(Vec::<ActiveToolCallInfo>::new()));
        let completed_tools = Arc::new(tokio::sync::RwLock::new(Vec::<CompletedToolCallInfo>::new()));
        let cancellation_flag = Arc::new(AtomicBool::new(false));

        // Register this chat as active (for streaming state tracking)
        {
            let mut active = self.active_chats.write().await;
            active.insert(session_id.to_string(), ActiveChat {
                _session_id: session_id.to_string(),
                cancelled: false,
                cancellation_flag: cancellation_flag.clone(),
                started_at: chrono::Utc::now(),
                accumulated_text: accumulated.clone(),
                accumulated_thinking: accumulated_thinking.clone(),
                active_tool_calls: active_tools.clone(),
                completed_tool_calls: completed_tools.clone(),
            });
        }

        // Populate the shared session history for the `recall_messages` tool
        if let Some(ref session_history) = self.session_history {
            let mut hist = session_history.write().await;
            hist.clear();
            hist.extend(history.iter().cloned());
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
        let base_models: Vec<String> = if let Some(ref model) = model_override {
            // Explicit model override (e.g., from sub-session) — use only that model
            vec![model.clone()]
        } else {
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
        let mut rate_limited_providers = HashSet::new();

        // Index-based walk so a transient provider fault can retry the SAME
        // model (`continue` without advancing) before falling down the
        // priority list. `same_model_retries` counts CONSECUTIVE
        // no-progress faults: it is bounded by CHAT_TRANSIENT_RETRIES_MAX
        // and replenished when a failed attempt completed at least one tool
        // call first (a fault after real forward work is a new burst, not an
        // escalation of the last one). Total attempts are therefore bounded
        // by models_to_try.len() * (1 + CHAT_TRANSIENT_RETRIES_MAX) plus one
        // per completed-tool burst — the loop cannot spin without the model
        // doing real work between faults.
        let mut model_index = 0usize;
        let mut same_model_retries = 0usize;

        while model_index < models_to_try.len() {
            let model = &models_to_try[model_index];
            // Skip models whose provider was already rate-limited (shared bucket).
            // e.g., if Opus was rate-limited, skip Haiku (both Anthropic).
            let provider = crate::llm_router::ProviderId::from_model(model);
            if rate_limited_providers.contains(&provider) {
                info!("Skipping {} — provider {:?} is rate-limited", model, provider);
                tried_models.push(format!("{} (skipped: provider rate-limited)", model));
                model_index += 1;
                continue;
            }

            if same_model_retries == 0 {
                tried_models.push(model.clone());
            }
            info!(
                "Trying model: {} (attempt {}, same-model retry {})",
                model,
                tried_models.len(),
                same_model_retries
            );

            // Every attempt (fresh model OR same-model retry) starts from a
            // clean slate. Without this, a failed attempt's partial stream
            // stays in the shared buffers, is re-streamed alongside the
            // retry, and — if every model fails — the all-exhausted path
            // persists the concatenation of all partial attempts as one
            // assistant message.
            accumulated.write().await.clear();
            accumulated_thinking.write().await.clear();
            active_tools.write().await.clear();
            completed_tools.write().await.clear();

            // Notify clients which model we're using. Suppressed on
            // same-model retries: Event::Error{model_retry} already covers
            // those, and a re-emit would wipe the shown fallback reason.
            if same_model_retries == 0 {
                let fallback_reason = if tried_models.len() > 1 {
                    Some(last_error.clone())
                } else {
                    None
                };
                let _ = self.event_tx.send(Event::ModelSwitch {
                    model: model.clone(),
                    reason: fallback_reason,
                });
            }

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
                    same_model_retries = 0;
                    model_index += 1;
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

                // Configure context limits from model capabilities AND this
                // agent's actual output budget — the reserve tracks
                // max_tokens instead of the provider's max_output claim, so
                // small-output agents keep most of the window for input.
                // Claude interleaved thinking generates ON TOP of max_tokens;
                // its budget joins the reserve (Ollama bounds thinking inside
                // num_predict and needs no extra).
                let thinking_reserve_tokens = if agent_config.model.starts_with("claude") {
                    agent_config
                        .thinking_mode
                        .budget_tokens()
                        .unwrap_or(0) as usize
                } else {
                    0
                };
                context.configure_for_model_with_output(
                    &model_info,
                    agent_config.max_tokens as usize + thinking_reserve_tokens,
                );

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

                let mut agent = Agent::new(
                    agent_config,
                    llm_client,
                    self.tools.clone(),
                )
                .with_context(context);
                // Record per-model request stats into the shared tracker so the
                // control plane persists them and the router can route on them.
                if let Some(ref tracker) = self.model_stats {
                    agent = agent.with_stats(tracker.clone());
                }
                agent
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
                on_thinking: Some(Box::new({
                    let accumulated_thinking = accumulated_thinking.clone();
                    move |chunk: &str| {
                        // Accumulate thinking for run state recovery
                        if let Ok(mut buf) = accumulated_thinking.try_write() {
                            buf.push_str(chunk);
                        }
                        let _ = event_tx_thinking.send(Event::ThinkingDelta {
                            session_id: session_id_thinking.clone(),
                            delta: chunk.to_string(),
                        });
                    }
                })),
                on_tool_start: Some(Box::new(move |call_id: &str, name: &str, input: &serde_json::Value, model: Option<&str>| {
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
                        model: model.map(|m| m.to_string()),
                    });
                })),
                on_tool_end: {
                    let event_tx_tool_end = self.event_tx.clone();
                    let session_id_tool_end = session_id.to_string();
                    let active_tools_for_end = active_tools.clone();
                    let completed_tools_for_end = completed_tools.clone();
                    Some(Box::new(move |call_id: &str, name: &str, output: &str, success: bool, duration_ms: u64, data: Option<&serde_json::Value>| {
                        // Move from active to completed
                        if let Ok(mut active) = active_tools_for_end.try_write() {
                            active.retain(|t| t.call_id != call_id);
                        }
                        if let Ok(mut completed) = completed_tools_for_end.try_write() {
                            completed.push(CompletedToolCallInfo {
                                call_id: call_id.to_string(),
                                name: name.to_string(),
                                output: output.to_string(),
                                success,
                                duration_ms,
                            });
                        }
                        let _ = event_tx_tool_end.send(Event::ToolEnd {
                            session_id: session_id_tool_end.clone(),
                            call_id: call_id.to_string(),
                            output: output.to_string(),
                            success,
                            duration_ms,
                            data: data.cloned(),
                        });
                    }))
                },
                // Enable auto-extraction if memory service is available
                auto_extract_memories: has_memory,
                on_memory: if has_memory {
                    let ws_id_for_memory = workspace_id.clone();
                    Some(Box::new(move |memory: nanna_agent::ExtractedMemory| {
                        let mem_service = memory_for_extraction.clone();
                        let ws_id = ws_id_for_memory.clone();
                        Box::pin(async move {
                            if let Some(ref service) = mem_service {
                                let mut metadata = memory.tags.unwrap_or_default();
                                metadata.insert("category".to_string(), memory.category.clone());
                                // Derive importance from category. Memories never
                                // expire — all categories are permanent.
                                let importance: f32 = match memory.category.as_str() {
                                    "tool_result" => 1.5,
                                    "preference" | "identity" => 4.0,
                                    "fact" | "insight" => 3.5,
                                    "context" => 3.0,
                                    _ => 3.0,
                                };

                                // Skip low-signal content: errors, tiny results, or garbled output
                                let dominated_by_non_ascii = memory.content.chars().take(200)
                                    .filter(|c| !c.is_ascii_alphanumeric() && !c.is_ascii_whitespace() && !c.is_ascii_punctuation())
                                    .count() > 40;

                                if memory.content.starts_with("Error:")
                                    || memory.content.starts_with("Execution failed:")
                                    || memory.content.contains("Error: Execution failed")
                                    || memory.content.contains("Command failed")
                                    || memory.content.contains("Missing required parameter")
                                    || memory.content.contains("cannot find the path specified")
                                    || memory.content.contains("Parameter format not correct")
                                    || memory.content.contains("not recognized as an internal")
                                    || memory.content.contains("Bridge error:")
                                    || memory.content.contains("JS execution failed")
                                    || (memory.category == "tool_result" && memory.content.contains("Error"))
                                    || memory.content.len() < 20
                                    || dominated_by_non_ascii
                                {
                                    info!("Skipping low-signal memory [{}]: {}", memory.category, truncate(&memory.content, 50));
                                    return;
                                }

                                // Store with workspace scope if session has a workspace
                                if let Some(ref ws_id) = ws_id {
                                    if let Err(e) = service.remember_scoped(&memory.content, metadata, importance, Some(ws_id.clone())).await {
                                        warn!("Failed to auto-store scoped memory: {}", e);
                                    } else {
                                        info!("Auto-extracted memory [{}] (workspace: {}): {}", memory.category, ws_id, truncate(&memory.content, 50));
                                    }
                                } else if let Err(e) = service.remember_with_importance(&memory.content, metadata, importance).await {
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
                max_iterations: max_iterations_override,
                attachments: attachments.clone(),
                is_sub_agent,
                all_tools_active: is_sub_agent,
                // "One path": a chat whose message opens with MISSION runs
                // long-horizon — the loop auto-continues the model (visible
                // as mission_control tool chips) until it declares MISSION
                // COMPLETE or stalls. One user prompt, continuous work.
                mission_mode: !is_sub_agent && message_is_mission(message),
                on_checkpoint: {
                    let checkpoint_session_id = session_id.to_string();
                    let checkpoint_accumulated = accumulated.clone();
                    let checkpoint_completed = completed_tools.clone();
                    let checkpoint_storage = self.storage.clone();
                    Some(Box::new(move |messages: &[nanna_llm::AnthropicMessage], iteration: usize| {
                        // Snapshot accumulated text + tool calls to a checkpoint.
                        // This runs synchronously in the agent loop — keep it fast.
                        let text = checkpoint_accumulated.try_read()
                            .map(|t| t.clone())
                            .unwrap_or_default();
                        let tools: Vec<CompletedToolCallInfo> = checkpoint_completed.try_read()
                            .map(|t| t.clone())
                            .unwrap_or_default();

                        let checkpoint = serde_json::json!({
                            "session_id": checkpoint_session_id,
                            "iteration": iteration,
                            "accumulated_text": text,
                            "tool_calls": tools,
                            "message_count": messages.len(),
                            "timestamp": chrono::Utc::now().to_rfc3339(),
                        });

                        // Write checkpoint to database (best-effort, fire-and-forget)
                        if let Some(ref storage) = checkpoint_storage {
                            if let Ok(json) = serde_json::to_string(&checkpoint) {
                                let storage = storage.clone();
                                let sid = checkpoint_session_id.clone();
                                // Spawn async write — sync callback can't await
                                let _ = tokio::task::spawn(async move {
                                    if let Err(e) = storage.save_checkpoint(&sid, &json).await {
                                        tracing::warn!("Failed to save checkpoint: {}", e);
                                    }
                                });
                            }
                        }
                    }))
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

                    // Tool events already emitted in real-time via on_tool_end callback

                    // Emit completion event
                    let _ = self.event_tx.send(Event::MessageEnd {
                        session_id: session_id.to_string(),
                        message_id: message_id.clone(),
                        content: response.text.clone(),
                    });

                    info!("Success with model: {}", model);

                    // Clean up checkpoint — run completed successfully
                    if let Some(ref storage) = self.storage {
                        let storage = storage.clone();
                        let sid = session_id.to_string();
                        tokio::spawn(async move {
                            let _ = storage.delete_checkpoint(&sid).await;
                        });
                    }

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
                        partial: false,
                    });
                }
                Err(e) => {
                    let error_str = e.to_string();
                    warn!("Model {} failed: {}", model, error_str);
                    last_error = error_str.clone();

                    // Transient provider fault (timeout / 5xx / dropped
                    // stream / aborted generation)? Retry the SAME model with
                    // escalating backoff before falling down the priority
                    // list — a single-model list (the common local setup)
                    // otherwise dies on the first hiccup and heartbeats fail
                    // for hours. Mirrors the task harness's step-retry
                    // ladder; Ollama-served models additionally get runner
                    // surgery (provider-gated — never fires for cloud models).
                    // Rate-limit/overload errors (429/529) are excluded: the
                    // branch below honors the provider's Retry-After and the
                    // shared-bucket skip instead of hammering it. A cancelled
                    // chat must not heal either — no retry, no server surgery.
                    // A fault after real forward progress starts a NEW burst:
                    // replenish the same-model retry budget instead of letting
                    // it accumulate across the whole message. A single-prompt
                    // long-horizon mission gets exactly one user message — a
                    // cumulative budget that survives hours of successful tool
                    // rounds and then kills the run on the third hiccup is a
                    // per-mission bound dressed up as a per-fault one. Progress
                    // means at least one tool call completed this attempt
                    // (completed_tools is cleared at attempt start), so the
                    // heal loop cannot spin without real work between faults;
                    // consecutive no-progress faults still climb the
                    // unload→restart ladder and exhaust at the max.
                    if same_model_retries > 0 && crate::tasks::is_transient_llm_error(&error_str) {
                        let attempt_tool_calls = completed_tools.read().await.len();
                        if attempt_tool_calls > 0 {
                            info!(
                                "Attempt completed {attempt_tool_calls} tool calls before this fault — new transient burst, retry budget replenished"
                            );
                            same_model_retries = 0;
                        }
                    }

                    if crate::tasks::is_transient_llm_error(&error_str)
                        && !Self::is_rate_limit_error(&error_str)
                        && !cancellation_flag.load(Ordering::Relaxed)
                        && same_model_retries < CHAT_TRANSIENT_RETRIES_MAX
                    {
                        same_model_retries += 1;
                        let backoff_secs = CHAT_RETRY_BACKOFF_SECS[same_model_retries - 1];
                        warn!(
                            "Transient failure on {model} — retrying in {backoff_secs}s ({same_model_retries}/{CHAT_TRANSIENT_RETRIES_MAX})"
                        );
                        let _ = self.event_tx.send(Event::Error {
                            code: "model_retry".to_string(),
                            message: format!(
                                "Transient failure on {model}. Retrying ({same_model_retries}/{CHAT_TRANSIENT_RETRIES_MAX})..."
                            ),
                        });
                        tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                        // Re-check cancellation after the sleep: a cancel
                        // arriving mid-backoff must not bounce the shared
                        // Ollama server for a chat nobody is waiting on.
                        if provider == crate::llm_router::ProviderId::Ollama
                            && !cancellation_flag.load(Ordering::Relaxed)
                        {
                            // Second failure: unload the model to clear a
                            // degraded runner. Final retry: restart the
                            // server — the sticky degraded state survives
                            // unloads (verified live in the endurance runs).
                            if same_model_retries == 2 {
                                crate::tasks::reset_ollama_runner_for(model).await;
                            }
                            if same_model_retries == CHAT_TRANSIENT_RETRIES_MAX
                                && crate::tasks::ollama_restart_allowed()
                            {
                                crate::tasks::restart_ollama_server().await;
                            }
                        }
                        continue;
                    }

                    // Rate limit? Wait before falling through, and record the
                    // provider so we can skip other models on the same provider
                    // (they share the same rate limit bucket).
                    if Self::is_rate_limit_error(&error_str) {
                        let provider = crate::llm_router::ProviderId::from_model(model);
                        rate_limited_providers.insert(provider);
                        let retry_secs = Self::parse_retry_after(&error_str).unwrap_or(10);
                        // Cap wait at 60s
                        let wait_secs = retry_secs.min(60);
                        info!("Rate limited on {} (provider {:?}). Waiting {}s before trying next model...", model, provider, wait_secs);
                        let _ = self.event_tx.send(Event::Error {
                            code: "rate_limit".to_string(),
                            message: format!("Rate limited on {}. Waiting {}s...", model, wait_secs),
                        });
                        tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;
                    }

                    // Context length exceeded? The agent's internal retry already
                    // tried to truncate. If it still failed, skip to the next model
                    // but log it clearly — trying a *smaller* model won't help.
                    if Self::is_context_length_error(&error_str) {
                        warn!(
                            "Context length exceeded on {} even after emergency truncation. \
                             History may be too large for this model's context window.",
                            model
                        );
                    }

                    // Emit error as a warning so it's visible, but continue trying
                    let _ = self.event_tx.send(Event::Error {
                        code: "model_error".to_string(),
                        message: format!("Model {} failed: {}. Trying next model...", model, last_error),
                    });

                    info!("Model {} failed, trying next model in priority list", model);
                    same_model_retries = 0;
                    model_index += 1;
                }
            }
        }

        // Remove from active chats and decrement queue depth
        {
            let mut active = self.active_chats.write().await;
            active.remove(session_id);
        }
        queue.depth.fetch_sub(1, Ordering::Relaxed);

        // Collect any accumulated text from the failed run
        let partial_text = accumulated.read().await.clone();
        let partial_thinking = accumulated_thinking.read().await.clone();
        let completed = completed_tools.read().await.clone();

        let error_msg = format!("All models exhausted. Tried: {:?}. Last error: {}", tried_models, last_error);

        // All models failed
        let _ = self.event_tx.send(Event::Error {
            code: "all_models_exhausted".to_string(),
            message: error_msg.clone(),
        });

        // If there was accumulated work, return it as a partial result so the caller
        // can persist it. This prevents losing hours of streamed agent work.
        let partial_result = if !partial_text.is_empty() || !completed.is_empty() {
            let tool_records: Vec<ToolCallRecord> = completed.into_iter().map(|tc| ToolCallRecord {
                id: tc.call_id,
                name: tc.name,
                input: serde_json::Value::Null,
                output: Some(tc.output),
                success: Some(tc.success),
                duration_ms: Some(tc.duration_ms),
            }).collect();

            info!(
                text_len = partial_text.len(),
                tool_calls = tool_records.len(),
                "Preserving partial result from failed run"
            );

            Some(ChatResult {
                message_id: message_id.clone(),
                content: format!(
                    "{}\n\n---\n⚠️ *This response is incomplete — the run failed after producing the above. Error: {}*",
                    partial_text, last_error
                ),
                tool_calls: tool_records,
                input_tokens: 0,
                output_tokens: 0,
                reasoning: if partial_thinking.is_empty() { None } else { Some(partial_thinking) },
                partial: true,
            })
        } else {
            None
        };

        Err(ChatError {
            message: error_msg,
            partial_result,
        })
    }

    /// Check if an error indicates the context length was exceeded (400-class, not retryable
    /// on the same model without reducing context).
    fn is_context_length_error(error: &str) -> bool {
        let lower = error.to_lowercase();
        lower.contains("context_length_exceeded")
            || lower.contains("maximum context length")
            || lower.contains("reduce the length")
            || lower.contains("prompt is too long")
            || lower.contains("too many tokens")
    }

    /// Check if an error is a rate-limit / transient overload (may warrant retry with backoff)
    fn is_rate_limit_error(error: &str) -> bool {
        let error_lower = error.to_lowercase();

        // Rate limit errors
        if error_lower.contains("rate limit")
            || error_lower.contains("rate_limit")
            || error_lower.contains("429")
            || error_lower.contains("too many requests") {
            return true;
        }

        // Overloaded errors
        if error_lower.contains("overloaded")
            || error_lower.contains("529") {
            return true;
        }

        false
    }

    /// Parse retry-after seconds from an error message.
    /// Looks for patterns like "retry after X seconds", "try again in X", "retry-after: X"
    fn parse_retry_after(error: &str) -> Option<u64> {
        let lower = error.to_lowercase();

        // Pattern: "try again in X seconds" / "retry after X seconds" / "wait X seconds"
        for prefix in &["try again in ", "retry after ", "wait ", "retry-after: "] {
            if let Some(pos) = lower.find(prefix) {
                // Slice `lower` (where `pos` was found), NOT `error`: lowercasing
                // can change byte length, so `pos` may not be a char boundary in
                // `error` — indexing it there could panic on a non-ASCII message.
                // Digits are ASCII, so reading them from `lower` is equivalent.
                let after = &lower[pos + prefix.len()..];
                // Extract digits
                let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(secs) = num_str.parse::<u64>() {
                    return Some(secs);
                }
            }
        }

        // Pattern: "Please try again later" (generic — use default)
        None
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
    
    /// Check if a checkpoint exists for a session (indicates a crashed run).
    /// Note: This only checks legacy file-based checkpoints. DB checkpoints are
    /// checked at startup via `list_checkpoints()` which is async.
    pub fn has_checkpoint(&self, session_id: &str) -> bool {
        // Legacy fallback: check file
        let path = self.checkpoint_dir.join(format!("checkpoint-{}.json", session_id));
        path.exists()
    }

    /// Recover a crashed run from raw checkpoint JSON data (from DB or file).
    pub fn recover_checkpoint_from_data(&self, data: &str) -> Option<ChatResult> {
        let checkpoint: serde_json::Value = serde_json::from_str(data).ok()?;

        let text = checkpoint.get("accumulated_text")?.as_str()?.to_string();
        let iteration = checkpoint.get("iteration").and_then(|v| v.as_u64()).unwrap_or(0);
        let timestamp = checkpoint.get("timestamp").and_then(|v| v.as_str()).unwrap_or("unknown");

        if text.is_empty() {
            return None;
        }

        let tool_calls: Vec<ToolCallRecord> = checkpoint.get("tool_calls")
            .and_then(|v| serde_json::from_value::<Vec<CompletedToolCallInfo>>(v.clone()).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|tc: CompletedToolCallInfo| ToolCallRecord {
                id: tc.call_id,
                name: tc.name,
                input: serde_json::Value::Null,
                output: Some(tc.output),
                success: Some(tc.success),
                duration_ms: Some(tc.duration_ms),
            })
            .collect();

        info!(
            iteration = iteration,
            text_len = text.len(),
            tool_calls = tool_calls.len(),
            checkpoint_time = timestamp,
            "Recovered checkpoint from crashed run"
        );

        Some(ChatResult {
            message_id: uuid::Uuid::new_v4().to_string(),
            content: format!(
                "{}\n\n---\n⚠️ *This response was recovered from iteration {} of a crashed run (checkpoint: {}). The run did not complete normally.*",
                text, iteration, timestamp
            ),
            tool_calls,
            input_tokens: 0,
            output_tokens: 0,
            reasoning: None,
            partial: true,
        })
    }

    /// Recover a crashed run from its checkpoint (legacy file-based only).
    /// DB-based checkpoints are recovered at startup via async code in server.rs.
    /// Returns the partial content and tool calls, or None if no checkpoint exists.
    /// The checkpoint file is consumed (deleted) after recovery.
    pub fn recover_checkpoint(&self, session_id: &str) -> Option<ChatResult> {
        // Legacy file-based checkpoint
        let path = self.checkpoint_dir.join(format!("checkpoint-{}.json", session_id));
        let data = std::fs::read_to_string(&path).ok();
        if data.is_some() {
            let _ = std::fs::remove_file(&path);
        }
        self.recover_checkpoint_from_data(&data?)
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
                accumulated_thinking: chat.accumulated_thinking.read().await.clone(),
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
                accumulated_thinking: String::new(),
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
    
    /// Recall memories scoped to a workspace (workspace sees global + own, None sees all)
    pub async fn recall_memories_scoped(&self, query: &str, limit: usize, workspace_id: Option<&str>) -> Vec<MemoryContext> {
        if let Some(ref memory) = self.memory {
            match memory.recall_scoped(query, workspace_id).await {
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
                    warn!("Scoped memory recall failed: {}", e);
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
    /// If true, this result is partial (the run failed but work was done).
    /// The caller should still persist the content to avoid losing streamed work.
    pub partial: bool,
}

/// Error from a chat completion that includes any partial work done before failure.
#[derive(Debug, Clone)]
pub struct ChatError {
    pub message: String,
    /// Partial result with accumulated text from the failed run (if any work was done).
    /// Callers should persist this to avoid losing streamed work.
    pub partial_result: Option<ChatResult>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_detection_covers_known_shapes() {
        for s in [
            "429 Too Many Requests",
            "Rate limit exceeded",
            "error: rate_limit_error",
            "The service is Overloaded, try later",
            "HTTP 529",
            "TOO MANY REQUESTS",
        ] {
            assert!(AgentService::is_rate_limit_error(s), "should flag: {s}");
        }
        for s in ["400 bad request", "context_length_exceeded", "connection reset"] {
            assert!(!AgentService::is_rate_limit_error(s), "should NOT flag: {s}");
        }
    }

    #[test]
    fn context_length_detection_covers_known_shapes() {
        for s in [
            "context_length_exceeded",
            "This model's maximum context length is 8192 tokens",
            "Please reduce the length of the messages",
            "prompt is too long",
            "too many tokens in the request",
        ] {
            assert!(AgentService::is_context_length_error(s), "should flag: {s}");
        }
        assert!(!AgentService::is_context_length_error("429 rate limit"));
    }

    #[test]
    fn mission_detection_is_prefix_only() {
        assert!(message_is_mission("MISSION: build a CLI"));
        assert!(message_is_mission("mission build a CLI"));
        assert!(message_is_mission("  Mission:\ndo the thing"));
        // Mentioning missions mid-message must not trigger long-horizon.
        assert!(!message_is_mission("what's our mission statement?"));
        assert!(!message_is_mission("The MISSION was a success"));
        assert!(!message_is_mission("missionary"));
        assert!(!message_is_mission(""));
    }

    #[test]
    fn parse_retry_after_extracts_seconds() {
        assert_eq!(
            AgentService::parse_retry_after("Rate limited. Please try again in 30 seconds."),
            Some(30)
        );
        assert_eq!(
            AgentService::parse_retry_after("Retry after 5 seconds"),
            Some(5)
        );
        assert_eq!(
            AgentService::parse_retry_after("retry-after: 12"),
            Some(12)
        );
        assert_eq!(AgentService::parse_retry_after("please WAIT 60 s"), Some(60));
        // No number / no recognized pattern → None (caller uses a default).
        assert_eq!(AgentService::parse_retry_after("Please try again later"), None);
        assert_eq!(AgentService::parse_retry_after("try again in soon"), None);
    }

    #[test]
    fn parse_retry_after_survives_non_ascii_without_panic() {
        // Same-byte-length non-ASCII prefix (Cyrillic).
        let msg = "Ошибка: rate limit — please try again in 7 seconds";
        assert_eq!(AgentService::parse_retry_after(msg), Some(7));
        // Non-ASCII with no retry hint → None, still no panic.
        assert_eq!(AgentService::parse_retry_after("Överbelastad 日本語"), None);
        // 'İ' (U+0130, 2 bytes) lowercases to "i̇" (3 bytes) — GROWS. So the
        // offset `find` returns in the lowercased string is past the matching
        // byte in the original; slicing the original there would land mid-char
        // (panic) or extract the wrong digits. The fix (slice the lowercased
        // string) must still recover 42. This case is the regression guard.
        assert_eq!(
            AgentService::parse_retry_after("İ error: retry after 42 seconds"),
            Some(42)
        );
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        // ASCII: simple cut with ellipsis.
        assert_eq!(truncate("hello world", 5), "hello...");
        // Under the limit: unchanged, no ellipsis.
        assert_eq!(truncate("hi", 5), "hi");
        // Multi-byte: "a🚀bc" is a(1) + 🚀(4 bytes) + b + c. Cutting at byte 2
        // lands inside the emoji, so it must back off to byte 1 (after 'a')
        // rather than panic — result "a...".
        assert_eq!(truncate("a🚀bc", 2), "a...");
    }
}
