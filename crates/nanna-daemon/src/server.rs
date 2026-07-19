//! Daemon Server - Main daemon orchestrator
//!
//! Combines IPC server, control plane, sessions, persistence, and all subsystems.

use crate::agent_service::{AgentService, AgentServiceConfig};
use crate::channels::{ChannelManager, ChannelsConfig};
use crate::control::ControlPlane;
use crate::embedding_router::{EmbeddingProviderInfo, EmbeddingRouter};
use crate::health::{DEFAULT_HEALTH_PORT, HealthServer, HealthState, PidFile};
use crate::ipc::{IpcServer, IpcServerConfig};
use crate::llm_router::LlmRouter;
use crate::memory_persistence::TursoMemoryPersistence;
use crate::persistence::PersistenceManager;
use crate::protocol::Response;
use crate::session::SessionManager;
use crate::webhook::{DEFAULT_WEBHOOK_PORT, WebhookConfig, WebhookServer};
use async_trait::async_trait;
use nanna_agent::ThinkingMode;
use nanna_channels::{
    ChannelId, IncomingMessage, MessageContent, MessageRouter as ChannelMessageRouter,
    Sender as ChannelSender, TelegramChannel,
};
use nanna_config::credentials::{self, SecureStore};
use nanna_llm::RequestBuilder;
use nanna_memory::MemoryService;
use nanna_scripting::ServiceFn;
use nanna_tools::{AgentSpawner, ParentChannel, SpawnResult, ToolRegistry};
use std::collections::HashMap;
use std::path::PathBuf;

/// Heartbeat check-in prompt for the daemon scheduler.
///
/// Deliberately does **not** command the model to `Read HEARTBEAT.md` — that
/// drove a `read_file` tool call which, with no active workspace, resolved to
/// `~/HEARTBEAT.md` and hard-errored (`os error 2`) every heartbeat. P17 has
/// since retired the bespoke `HEARTBEAT.md` entirely (recurrence lives in
/// scheduled-task config), so the prompt now frames the heartbeat as running
/// due scheduled work — never reading instruction files off disk.
const DAEMON_HEARTBEAT_PROMPT: &str = "Heartbeat check-in. Run any due scheduled tasks. Do not read files from disk looking for instructions, and do not infer or repeat old tasks from prior chats. Review your current state, and if nothing needs attention, reply HEARTBEAT_OK.";

/// Concrete implementation of AgentSpawner that lives in the daemon
/// where it can create Agent instances with isolated context.
struct AgentSpawnerImpl {
    router: Arc<crate::llm_router::LlmRouter>,
    tools: Arc<ToolRegistry>,
    agent_config: nanna_agent::AgentConfig,
    system_prompt: String,
    workspace_root: Option<PathBuf>,
    workspace_context: Option<String>,
    /// Shared model stats tracker (sub-agents contribute to the same stats)
    stats: Option<nanna_agent::ModelStatsTracker>,
}

#[async_trait]
impl AgentSpawner for AgentSpawnerImpl {
    async fn spawn(
        &self,
        prompt: &str,
        description: &str,
        max_iterations: Option<usize>,
    ) -> Result<SpawnResult, String> {
        use nanna_agent::{Agent, AgentContext, RunOptions};

        info!(description = description, max_iterations = ?max_iterations, "Spawning sub-agent");

        // Create isolated context with system prompt + workspace only
        let mut context = AgentContext::new(uuid::Uuid::new_v4().to_string())
            .with_system_prompt(&self.system_prompt);

        // Inject workspace context if available
        if let Some(ref ws_root) = self.workspace_root {
            context.workspace_root = Some(ws_root.clone());
        }
        if let Some(ref ws_ctx) = self.workspace_context {
            context.workspace_context = Some(ws_ctx.clone());
        }

        // Configure agent — sub-agents are full agents, no artificial iteration cap
        let mut config = self.agent_config.clone();
        config.max_iterations = max_iterations;

        // Use sub_agent_model if configured, otherwise fall back to primary model
        if let Some(ref sub_model) = self.agent_config.sub_agent_model {
            info!(
                sub_agent_model = %sub_model,
                primary_model = %config.model,
                "Using dedicated sub-agent model instead of primary"
            );
            config.model = sub_model.clone();
        }

        // Select the right LLM client via the router based on the model
        // The router dispatches to the correct provider (Anthropic, Ollama, OpenAI, etc.)
        let model = &config.model;
        let model_display = model.clone(); // Preserve full model name for reporting
        let llm_client = self.router.client_for_model(model).ok_or_else(|| {
            format!(
                "No provider available for model '{}'. Available providers: {:?}",
                model,
                self.router.available_providers()
            )
        })?;

        // Strip provider prefix from model name for the actual API call
        config.model = LlmRouter::strip_model_prefix(&config.model);

        let mut agent = Agent::new(config, llm_client, self.tools.clone()).with_context(context);

        // Share model stats tracker with sub-agents
        if let Some(ref tracker) = self.stats {
            agent = agent.with_stats(tracker.clone());
        }

        let options = RunOptions {
            max_iterations,
            is_sub_agent: true,
            all_tools_active: true,
            ..Default::default()
        };

        // Run without timeout — agent stops when done or cancelled
        let result = agent
            .run(prompt, options)
            .await
            .map_err(|e| format!("Sub-agent error: {e}"))?;

        info!(
            description = description,
            iterations = result.iterations,
            tool_calls = result.tool_calls.len(),
            input_tokens = result.input_tokens,
            output_tokens = result.output_tokens,
            "Sub-agent completed"
        );

        Ok(SpawnResult {
            text: result.text,
            iterations: result.iterations,
            tool_calls: result.tool_calls.len(),
            input_tokens: result.input_tokens,
            output_tokens: result.output_tokens,
            model: model_display,
        })
    }
}

/// Concrete implementation of ParentChannel that lives in the daemon.
/// Allows sub-agents to ask their parent questions.
///
/// Instead of blocking on mailbox polling, this makes a lightweight LLM call
/// with the parent session's conversation context to answer the sub-agent's
/// question directly. This avoids deadlocks (parent is blocked on the task
/// tool while the sub-agent waits for a reply).
struct ParentChannelImpl {
    sessions: Arc<SessionManager>,
    event_tx: Option<tokio::sync::broadcast::Sender<crate::protocol::Event>>,
    router: Arc<crate::llm_router::LlmRouter>,
    /// Model to use for answering sub-agent questions (e.g. cheap/fast model)
    model: String,
}

#[async_trait]
impl ParentChannel for ParentChannelImpl {
    async fn ask_parent(
        &self,
        sub_session_id: &str,
        question: &str,
        _timeout_secs: u64,
    ) -> Result<String, String> {
        // Look up the sub-session to find its parent and task context
        let sub_info = self
            .sessions
            .get_sub_session(sub_session_id)
            .await
            .ok_or_else(|| {
                format!(
                    "Sub-session '{}' not found — ask_parent is only available to sub-agents",
                    sub_session_id
                )
            })?;

        let parent_id = sub_info
            .parent_id
            .clone()
            .ok_or_else(|| "This sub-agent has no parent session".to_string())?;

        let label = sub_info.label.clone();
        let task = sub_info.task.clone();

        // Emit event for GUI visibility
        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(crate::protocol::Event::SubSessionQuestion {
                session_id: sub_session_id.to_string(),
                parent_id: Some(parent_id.clone()),
                label: label.clone(),
                question: question.to_string(),
            });
        }

        tracing::info!(
            sub_session = sub_session_id,
            parent = %parent_id,
            label = ?label,
            "Sub-agent asking parent: {}",
            question.chars().take(100).collect::<String>()
        );

        // Load parent session's recent conversation for context
        let parent_session = self
            .sessions
            .get(&parent_id)
            .await
            .ok_or_else(|| format!("Parent session '{}' not found", parent_id))?;

        let recent_messages: Vec<String> = parent_session
            .messages
            .iter()
            .rev()
            .take(20) // Last 20 messages for context
            .rev()
            .map(|m| {
                format!(
                    "[{}]: {}",
                    m.role.as_db_str(),
                    m.content.chars().take(500).collect::<String>()
                )
            })
            .collect();

        let context = recent_messages.join("\n");

        // Build a focused prompt to answer the sub-agent's question
        let prompt = format!(
            "You are answering a question from a sub-agent that was delegated a task.\n\n\
             ## Sub-agent task\n{}\n\n\
             ## Recent conversation context (parent session)\n{}\n\n\
             ## Sub-agent's question\n{}\n\n\
             Answer concisely and directly. Provide only the information the sub-agent needs \
             to continue its work. If you don't have enough context to answer, say so clearly.",
            task,
            if context.is_empty() {
                "(no prior conversation)".to_string()
            } else {
                context
            },
            question
        );

        // Make a lightweight LLM call to answer the question using parent context
        let model = &self.model;
        let llm_client = self
            .router
            .client_for_model(model)
            .ok_or_else(|| format!("No provider for model '{}'", model))?;

        let stripped_model = crate::llm_router::LlmRouter::strip_model_prefix(model);
        let request = nanna_llm::CompletionRequest {
            model: stripped_model,
            messages: vec![
                nanna_llm::Message::system(
                    "You are a helpful assistant answering questions from sub-agents. Be concise and precise.",
                ),
                nanna_llm::Message::user(&prompt),
            ],
            max_tokens: Some(2048),
            ..Default::default()
        };

        let answer = llm_client
            .complete(&request)
            .await
            .map_err(|e| format!("LLM call failed: {}", e))?;

        tracing::info!(
            sub_session = sub_session_id,
            answer_len = answer.len(),
            "Parent answered sub-agent question"
        );

        Ok(answer)
    }
}

use std::sync::Arc;
use std::time::Duration;

/// Build service closures for script tools.
///
/// These closures allow JS/TS tools to call back into Rust subsystems via `Nanna.service(name, params)`.
/// Shared session history that can be updated before each agent run.
/// This allows the `session.history` service to return messages for the current session.
pub type SharedSessionHistory = Arc<tokio::sync::RwLock<Vec<crate::session::SessionMessage>>>;

fn build_script_services(
    memory: &Option<Arc<MemoryService>>,
    spawner: Option<Arc<dyn AgentSpawner + Send + Sync>>,
    session_history: SharedSessionHistory,
    workspace_id: Arc<tokio::sync::RwLock<Option<String>>>,
    storage: Option<Arc<nanna_storage::Storage>>,
) -> HashMap<String, ServiceFn> {
    use serde_json::{Value, json};

    let mut services: HashMap<String, ServiceFn> = HashMap::new();

    // Task store services (P15): the todo skill's backend. Only available
    // with storage — the skill falls back to its JSON file otherwise.
    if let Some(storage) = storage {
        services.extend(crate::tasks::build_task_services(
            storage,
            workspace_id.clone(),
        ));
    }

    // Memory services
    if let Some(mem) = memory {
        let mem_store = mem.clone();
        let ws_store = workspace_id.clone();
        services.insert(
            "memory.store".to_string(),
            Arc::new(move |params: Value| {
                let mem = mem_store.clone();
                let ws = ws_store.clone();
                Box::pin(async move {
                    let content = params
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let tags: HashMap<String, String> = params
                        .get("tags")
                        .and_then(|v| v.as_object())
                        .map(|obj| {
                            obj.iter()
                                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let importance = params
                        .get("importance")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(1.0) as f32;
                    let workspace = ws.read().await.clone();
                    match mem
                        .remember_scoped(&content, tags, importance, workspace)
                        .await
                    {
                        Ok((id, _)) => Ok(json!({"id": id})),
                        Err(e) => Err(e.to_string()),
                    }
                })
            }),
        );

        let mem_search = mem.clone();
        let ws_search = workspace_id.clone();
        services.insert(
            "memory.search".to_string(),
            Arc::new(move |params: Value| {
                let mem = mem_search.clone();
                let ws = ws_search.clone();
                Box::pin(async move {
                    let query = params
                        .get("query")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
                    let workspace = ws.read().await;
                    match mem.recall_scoped(&query, workspace.as_deref()).await {
                        Ok(results) => {
                            let items: Vec<Value> = results
                                .into_iter()
                                .take(limit)
                                .map(
                                    |r| json!({"id": r.id, "content": r.content, "score": r.score}),
                                )
                                .collect();
                            Ok(Value::Array(items))
                        }
                        Err(e) => {
                            // If embedding is not configured, return empty results
                            // instead of an error so the agent can continue gracefully
                            let msg = e.to_string();
                            if msg.contains("embedding") || msg.contains("No embedding function") {
                                tracing::debug!("Memory search skipped: {}", msg);
                                Ok(Value::Array(vec![]))
                            } else {
                                Err(msg)
                            }
                        }
                    }
                })
            }),
        );

        // Alias: some tool scripts may call memory.embed instead of memory.store
        let mem_embed = mem.clone();
        let ws_embed = workspace_id.clone();
        services.insert(
            "memory.embed".to_string(),
            Arc::new(move |params: Value| {
                let mem = mem_embed.clone();
                let ws = ws_embed.clone();
                Box::pin(async move {
                    let content = params
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let tags: HashMap<String, String> = params
                        .get("tags")
                        .and_then(|v| v.as_object())
                        .map(|obj| {
                            obj.iter()
                                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let importance = params
                        .get("importance")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(1.0) as f32;
                    let workspace = ws.read().await.clone();
                    match mem
                        .remember_scoped(&content, tags, importance, workspace)
                        .await
                    {
                        Ok((id, _)) => Ok(json!({"id": id})),
                        Err(e) => Err(e.to_string()),
                    }
                })
            }),
        );

        let mem_delete = mem.clone();
        services.insert(
            "memory.delete".to_string(),
            Arc::new(move |params: Value| {
                let mem = mem_delete.clone();
                Box::pin(async move {
                    let id = params
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    match mem.forget(&id).await {
                        Ok(()) => Ok(json!({"deleted": true})),
                        Err(e) => Err(e.to_string()),
                    }
                })
            }),
        );

        let mem_list = mem.clone();
        services.insert(
            "memory.list".to_string(),
            Arc::new(move |params: Value| {
                let mem = mem_list.clone();
                Box::pin(async move {
                    let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
                    let all = mem.list_all().await;
                    let items: Vec<Value> = all
                        .into_iter()
                        .take(limit)
                        .map(|e| json!({"id": e.id, "content": e.content, "weight": e.weight}))
                        .collect();
                    Ok(Value::Array(items))
                })
            }),
        );
    }

    // Agent spawner service
    if let Some(spawner) = spawner {
        services.insert(
            "agent.spawn".to_string(),
            Arc::new(move |params: Value| {
                let spawner = spawner.clone();
                Box::pin(async move {
                    let prompt = params
                        .get("prompt")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let description = params
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("sub-task")
                        .to_string();
                    let max_iterations = params
                        .get("max_iterations")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as usize);
                    match spawner.spawn(&prompt, &description, max_iterations).await {
                        Ok(result) => Ok(json!({
                            "text": result.text,
                            "iterations": result.iterations,
                            "tool_calls": result.tool_calls,
                            "model": result.model,
                        })),
                        Err(e) => Err(e),
                    }
                })
            }),
        );
    }

    // Embedded Python interpreter (no system Python required)
    {
        use nanna_scripting::python::PythonEngine;
        let python_engine = Arc::new(PythonEngine::new());
        services.insert(
            "python.exec".to_string(),
            Arc::new(move |params: Value| {
                let engine = python_engine.clone();
                Box::pin(async move {
                    let code = params
                        .get("code")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let timeout = params.get("timeout").and_then(|v| v.as_u64()).unwrap_or(30);
                    let workdir = params
                        .get("workdir")
                        .and_then(|v| v.as_str())
                        .map(String::from);

                    match engine.execute(&code, workdir.as_deref(), timeout).await {
                        Ok(result) => Ok(json!({
                            "stdout": result.stdout,
                            "stderr": result.stderr,
                            "success": result.success,
                            "error": result.error,
                            "duration_ms": result.duration_ms,
                        })),
                        Err(e) => Err(e.to_string()),
                    }
                })
            }),
        );
    }

    // Session history service — returns recent messages from the current session.
    // The SharedSessionHistory is populated before each agent run.
    {
        let history = session_history;
        services.insert(
            "session.history".to_string(),
            Arc::new(move |params: Value| {
                let history = history.clone();
                Box::pin(async move {
                    let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
                    let history = history.read().await;
                    let start = if history.len() > limit {
                        history.len() - limit
                    } else {
                        0
                    };
                    let messages: Vec<Value> = history[start..]
                        .iter()
                        .map(|msg| {
                            json!({
                                "role": format!("{:?}", msg.role).to_lowercase(),
                                "content": msg.content,
                                "timestamp": msg.timestamp.to_rfc3339(),
                            })
                        })
                        .collect();
                    Ok(json!(messages))
                })
            }),
        );
    }

    services
}
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

/// Configuration for the daemon server
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// IPC server configuration
    pub ipc: IpcServerConfig,
    /// Data directory
    pub data_dir: PathBuf,
    /// Log level
    pub log_level: String,
    /// Auto-save interval in seconds
    pub auto_save_interval_secs: u64,
    /// LLM configuration
    pub llm: LlmConfig,
    /// Agent configuration
    pub agent: AgentServiceConfig,
    /// Enable memory service (requires embedding provider)
    pub enable_memory: bool,
    /// Enable HTTP health server
    pub enable_health_server: bool,
    /// Health server port (default: 5148)
    pub health_port: u16,
    /// Enable PID file (prevents multiple instances)
    pub enable_pid_file: bool,
    /// Enable webhook server for inbound messages
    pub enable_webhook_server: bool,
    /// Webhook server port (default: 3000)
    pub webhook_port: u16,
    /// Webhook configuration
    pub webhook: WebhookConfig,
    /// Use TypeScript skill implementations instead of Rust builtins
    pub use_script_tools: bool,
    /// Directory containing tool scripts (resolved from env/config/default)
    pub tools_dir: Option<PathBuf>,
    /// Channel configurations (Telegram, Discord, Slack, etc.)
    pub channels: Option<nanna_config::ChannelsConfig>,
    /// Max fraction of memories the scheduled dream cycle may merge away in one
    /// run (mirrors `[memory] max_compression_ratio`). Threaded so automatic
    /// consolidation honors the same user setting the IPC-triggered path does.
    pub memory_max_compression_ratio: f32,
    /// Floor the scheduled dream cycle leaves after consolidating (mirrors
    /// `[memory] min_remaining_memories`).
    pub memory_min_remaining_memories: usize,
}

/// LLM provider configuration (multi-provider)
#[derive(Debug, Clone)]
pub struct LlmConfig {
    /// Primary provider (anthropic, openai, ollama) - used for single-provider mode
    pub provider: String,
    /// Anthropic API key
    pub anthropic_api_key: Option<String>,
    /// Anthropic OAuth access token (alternative to API key)
    pub anthropic_oauth_token: Option<String>,
    /// Whether to use OAuth token instead of API key for Anthropic
    pub anthropic_use_oauth: bool,
    /// OpenAI API key
    pub openai_api_key: Option<String>,
    /// OpenRouter API key
    pub openrouter_api_key: Option<String>,
    /// GitHub token (for GitHub Models)
    pub github_token: Option<String>,
    /// Ollama host
    pub ollama_host: String,
    /// Ollama API key (optional — for remote/authenticated instances)
    pub ollama_api_key: Option<String>,
    /// Legacy: API key field (for backwards compatibility)
    pub api_key: Option<String>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".to_string(),
            anthropic_api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
            anthropic_oauth_token: None,
            anthropic_use_oauth: false,
            openai_api_key: std::env::var("OPENAI_API_KEY").ok(),
            openrouter_api_key: std::env::var("OPENROUTER_API_KEY").ok(),
            github_token: std::env::var("GITHUB_TOKEN").ok(),
            ollama_host: "http://localhost:11434".to_string(),
            ollama_api_key: std::env::var("OLLAMA_API_KEY").ok(),
            api_key: None, // Legacy
        }
    }
}

impl Default for DaemonConfig {
    fn default() -> Self {
        let data_dir = directories::ProjectDirs::from("com", "nanna", "nanna-daemon")
            .map(|d| d.data_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("./data"));

        Self {
            ipc: IpcServerConfig::default(),
            data_dir,
            log_level: "info".to_string(),
            auto_save_interval_secs: 60,
            llm: LlmConfig::default(),
            agent: AgentServiceConfig::default(),
            enable_memory: true, // Enabled by default (requires embedding provider)
            enable_health_server: true,
            health_port: DEFAULT_HEALTH_PORT,
            enable_pid_file: true,
            enable_webhook_server: false, // Disabled by default (needs configuration)
            webhook_port: DEFAULT_WEBHOOK_PORT,
            webhook: WebhookConfig::default(),
            use_script_tools: true,
            tools_dir: None,
            channels: None,
            // Mirror ConsolidationConfig::default() (== nanna-config defaults).
            memory_max_compression_ratio: 0.50,
            memory_min_remaining_memories: 20,
        }
    }
}

/// Build the scheduled dream-cycle's [`ConsolidationConfig`] from the user's
/// memory settings, keeping automatic consolidation in lock-step with the
/// IPC-triggered path (see `control.rs`). The per-cluster content budget is
/// sized to the summarizer model's context window so the consolidation prompt
/// always fits it. Pure so it is unit-testable.
fn scheduled_consolidation_config(
    max_compression_ratio: f32,
    min_remaining_memories: usize,
    summarizer_context_window_tokens: usize,
) -> nanna_memory::ConsolidationConfig {
    nanna_memory::ConsolidationConfig {
        max_compression_ratio,
        min_remaining_memories,
        ..nanna_memory::ConsolidationConfig::default()
    }
    .with_summarizer_context_window(summarizer_context_window_tokens)
}

/// The main daemon server
pub struct DaemonServer {
    config: DaemonConfig,
    embedding: EmbeddingConfig,
    memory_path: Option<PathBuf>,
    _brave_api_key: Option<String>,
    sessions: Arc<SessionManager>,
    _control: Arc<ControlPlane>,
    ipc: Arc<IpcServer>,
    persistence: Arc<PersistenceManager>,
    shutdown_tx: broadcast::Sender<()>,
    /// PID file (prevents multiple instances)
    pid_file: Option<PidFile>,
    /// Log buffer for capturing daemon logs
    log_buffer: Option<crate::log_buffer::LogBuffer>,
    /// Shared storage for model stats persistence
    storage: Option<Arc<nanna_storage::Storage>>,
}

impl DaemonServer {
    /// Discover the embedding dimension by probing **the same router the live
    /// embed path uses**.
    ///
    /// Probing through the router (rather than a bespoke client built from
    /// `embedding.provider`) matters for two reasons:
    ///
    /// 1. The router resolves cloud keys from **config *or* env**, while a
    ///    bespoke client read only the env — so a key set in `config.toml`
    ///    probed as "missing" even though every real embed succeeded.
    /// 2. The router carries the Ollama fallback, so a probe survives an
    ///    unreachable/unkeyed cloud provider exactly like a real embed does.
    ///
    /// A failure here is **not** fatal — see the call site.
    async fn probe_embedding_dimension(router: &EmbeddingRouter) -> Result<usize, String> {
        let (embedding, _provider_changed) = router.embed_one("dimension probe").await?;
        if embedding.is_empty() {
            return Err("embedding provider returned an empty vector".to_string());
        }
        debug_assert!(!embedding.is_empty(), "probe returned a non-empty vector");
        Ok(embedding.len())
    }

    /// Create a new daemon server
    pub fn new(
        config: DaemonConfig,
        embedding: EmbeddingConfig,
        memory_path: Option<PathBuf>,
        brave_api_key: Option<String>,
    ) -> Self {
        let sessions = Arc::new(SessionManager::new());
        let control = Arc::new(ControlPlane::new(sessions.clone()));
        let ipc = Arc::new(IpcServer::new(config.ipc.clone()));
        let persistence = Arc::new(PersistenceManager::new(&config.data_dir));
        let (shutdown_tx, _) = broadcast::channel(1);

        // Create PID file if enabled
        let pid_file = if config.enable_pid_file {
            Some(PidFile::new(&config.data_dir))
        } else {
            None
        };

        Self {
            config,
            embedding,
            memory_path,
            _brave_api_key: brave_api_key,
            sessions,
            _control: control,
            ipc,
            persistence,
            shutdown_tx,
            pid_file,
            log_buffer: None,
            storage: None,
        }
    }

    /// Set the storage backend for model stats persistence and session persistence.
    pub fn set_storage(&mut self, storage: Arc<nanna_storage::Storage>) {
        // Replace the SessionManager with one that has storage
        let new_sessions = Arc::new(SessionManager::with_storage(storage.clone()));
        self.sessions = new_sessions.clone();
        // Update control plane reference
        self._control = Arc::new(ControlPlane::new(self.sessions.clone()));
        self.storage = Some(storage);
    }

    /// Get the shutdown sender (for signaling shutdown)
    pub fn shutdown_handle(&self) -> broadcast::Sender<()> {
        self.shutdown_tx.clone()
    }

    /// Get the IPC server address
    pub fn ipc_address(&self) -> String {
        self.ipc.address()
    }

    /// Run the daemon server
    pub async fn run(&mut self) -> Result<(), crate::DaemonError> {
        info!("Starting Nanna daemon...");
        info!("Data directory: {:?}", self.config.data_dir);

        // Ensure data directory exists
        std::fs::create_dir_all(&self.config.data_dir)?;

        // Acquire PID file to prevent multiple instances
        if let Some(ref pid_file) = self.pid_file {
            match pid_file.acquire() {
                Ok(()) => {
                    info!("PID file acquired at {:?}", pid_file.path());
                }
                Err(crate::health::PidFileError::AlreadyRunning(pid)) => {
                    error!("Another daemon instance is already running (PID: {})", pid);
                    return Err(crate::DaemonError::AlreadyRunning);
                }
                Err(e) => {
                    warn!("Failed to acquire PID file: {}. Continuing anyway.", e);
                }
            }
        }

        // Load sessions from Turso database
        {
            let loaded = self.sessions.load_from_db().await;
            info!("Loaded {} sessions from database", loaded);
        }

        // If no sessions loaded from DB, check for legacy sessions.json migration
        if self.sessions.count().await == 0 {
            if let Some((sessions, default_id)) = self.persistence.load_legacy_sessions().await {
                if !sessions.is_empty() {
                    info!(
                        "Migrating {} sessions from legacy sessions.json to database",
                        sessions.len()
                    );
                    for session in sessions {
                        self.sessions.restore(session).await;
                    }
                    if let Some(id) = default_id {
                        self.sessions.set_default(&id).await;
                    }
                    // Mark as migrated
                    self.persistence.mark_sessions_migrated().await;
                }
            }
        }

        // Create default session if none exist
        if self.sessions.count().await == 0 {
            let default_session = self.sessions.create(Some("Main".to_string())).await;
            info!("Created default session: {}", default_session.id);
        }

        // Initialize services
        let (tools, memory, agent, router, tools_dir, workspace_id_for_services, model_stats) =
            self.init_services().await?;

        // Recover any orphaned checkpoints from the database.
        if let Some(ref storage) = self.storage {
            match storage.list_checkpoints().await {
                Ok(checkpoint_ids) => {
                    for session_id in checkpoint_ids {
                        // Load checkpoint data from DB and parse it
                        if let Ok(Some(data)) = storage.load_checkpoint(&session_id).await {
                            if let Some(partial) = agent.recover_checkpoint_from_data(&data) {
                                let reasoning = partial.reasoning.clone();
                                self.sessions
                                    .add_full_message(
                                        &session_id,
                                        crate::session::MessageRole::Assistant,
                                        &partial.content,
                                        partial.tool_calls,
                                        reasoning,
                                    )
                                    .await;
                                info!("Recovered crashed run for session {}", session_id);
                            }
                        }
                        // Clean up the checkpoint
                        if let Err(e) = storage.delete_checkpoint(&session_id).await {
                            warn!(
                                "Failed to delete checkpoint for session {}: {}",
                                session_id, e
                            );
                        }
                    }
                }
                Err(e) => warn!("Failed to list checkpoints: {}", e),
            }

            // Also migrate any legacy checkpoint JSON files
            let checkpoint_dir = self.config.data_dir.join("checkpoints");
            if checkpoint_dir.exists() {
                if let Ok(entries) = std::fs::read_dir(&checkpoint_dir) {
                    for entry in entries.flatten() {
                        let filename = entry.file_name();
                        let name = filename.to_string_lossy();
                        if name.starts_with("checkpoint-") && name.ends_with(".json") {
                            let session_id = name
                                .strip_prefix("checkpoint-")
                                .and_then(|s| s.strip_suffix(".json"))
                                .unwrap_or("");
                            if !session_id.is_empty() {
                                if let Some(partial) = agent.recover_checkpoint(session_id) {
                                    let reasoning = partial.reasoning.clone();
                                    self.sessions
                                        .add_full_message(
                                            session_id,
                                            crate::session::MessageRole::Assistant,
                                            &partial.content,
                                            partial.tool_calls,
                                            reasoning,
                                        )
                                        .await;
                                    info!(
                                        "Recovered crashed run from legacy checkpoint for session {}",
                                        session_id
                                    );
                                }
                                // Remove the legacy file
                                let _ = std::fs::remove_file(entry.path());
                            }
                        }
                    }
                }
            }
        }

        // Scheduler: with daemon-first startup the daemon owns nanna.db, so it
        // is the cron runner (the GUI scheduler only runs in embedded mode).
        // Loads persisted jobs and runs heartbeat + memory consolidation,
        // mirroring the GUI's embedded schedule.
        let scheduler = {
            let scheduler_config = nanna_core::SchedulerConfig {
                heartbeat_interval: std::time::Duration::from_secs(1800),
                heartbeat_enabled: true,
                heartbeat_prompt: DAEMON_HEARTBEAT_PROMPT.to_string(),
                max_concurrent: 4,
                check_interval: std::time::Duration::from_secs(30),
                default_timezone: "UTC".to_string(),
            };
            let mut scheduler = nanna_core::Scheduler::new(scheduler_config);
            if let Some(ref storage) = self.storage {
                scheduler = scheduler.with_storage(storage.clone());
                match scheduler.load_jobs().await {
                    Ok(count) => info!("Loaded {count} cron jobs from storage"),
                    Err(e) => warn!("Failed to load cron jobs: {e}"),
                }
            } else {
                info!("Scheduler running without persistence (no storage backend)");
            }

            let deduped = scheduler.deduplicate_by_name("memory_consolidation").await;
            if deduped > 0 {
                info!("Removed {deduped} duplicate consolidation tasks");
            }
            if !scheduler.has_task_named("memory_consolidation").await {
                scheduler
                    .add_task(nanna_core::consolidation_task(Some(
                        std::time::Duration::from_secs(3600),
                    )))
                    .await;
                info!("Scheduled memory consolidation task (every 1 hour)");
            }

            // Task recurrence sweep (P15): the scheduler is the one recurrence
            // engine — recurring todo items are reopened here, not by a second
            // clock inside the task store.
            if self.storage.is_some() {
                let deduped = scheduler.deduplicate_by_name("task_recurrence_sweep").await;
                if deduped > 0 {
                    info!("Removed {deduped} duplicate recurrence sweep tasks");
                }
                if !scheduler.has_task_named("task_recurrence_sweep").await {
                    scheduler
                        .add_task(nanna_core::recurring_task(
                            "task_recurrence_sweep",
                            std::time::Duration::from_secs(300),
                            "Reopen recurring tasks whose next occurrence has arrived.",
                        ))
                        .await;
                    info!("Scheduled task recurrence sweep (every 5 minutes)");
                }
            }

            let agent_for_tasks = agent.clone();
            let memory_for_tasks = memory.clone();
            let router_for_tasks = router.clone();
            let storage_for_tasks = self.storage.clone();
            // Capture the user's memory-compression settings for the scheduled
            // dream cycle (Copy scalars, moved into the executor closure).
            let consolidation_max_ratio = self.config.memory_max_compression_ratio;
            let consolidation_min_remaining = self.config.memory_min_remaining_memories;
            let summarization_model = self
                .config
                .agent
                .summarization_priority
                .first()
                .cloned()
                .unwrap_or_else(|| self.config.agent.model.clone());
            let executor: nanna_core::TaskExecutor = Arc::new(move |task| {
                let agent = agent_for_tasks.clone();
                let memory = memory_for_tasks.clone();
                let router = router_for_tasks.clone();
                let storage = storage_for_tasks.clone();
                let summarization_model = summarization_model.clone();
                Box::pin(async move {
                    let start = std::time::Instant::now();
                    let started_at = chrono::Utc::now();
                    let (success, output, error) = match task.name.as_str() {
                        "memory_consolidation" => {
                            if let Some(ref memory) = memory {
                                info!("Running scheduled memory consolidation...");
                                let summarizer_info =
                                    router.get_model_info(&summarization_model).await;
                                let consolidation_config = scheduled_consolidation_config(
                                    consolidation_max_ratio,
                                    consolidation_min_remaining,
                                    summarizer_info.hard_input_limit(),
                                );
                                let summarize = |prompt: String| {
                                    let router = router.clone();
                                    let model = summarization_model.clone();
                                    async move {
                                        let request = nanna_llm::CompletionRequest::default()
                                            .with_message(nanna_llm::Message::user(&prompt));
                                        router
                                            .complete(&model, request)
                                            .await
                                            .map_err(|e| e.to_string())
                                    }
                                };
                                match memory.consolidate(&consolidation_config, summarize).await {
                                    Ok(result) => {
                                        info!(
                                            "Scheduled consolidation: {} processed, {} merged",
                                            result.memories_processed, result.memories_merged
                                        );
                                        (
                                            true,
                                            Some(format!(
                                                "Processed {} memories",
                                                result.memories_processed
                                            )),
                                            None,
                                        )
                                    }
                                    Err(e) => {
                                        error!("Scheduled consolidation failed: {e}");
                                        (false, None, Some(e.to_string()))
                                    }
                                }
                            } else {
                                (
                                    true,
                                    Some("Skipped (memory service unavailable)".to_string()),
                                    None,
                                )
                            }
                        }
                        "task_recurrence_sweep" => {
                            if let Some(ref storage) = storage {
                                let reopened = crate::tasks::sweep_recurrences(storage).await;
                                if reopened > 0 {
                                    info!("Recurrence sweep reopened {reopened} tasks");
                                }
                                (
                                    true,
                                    Some(format!("Reopened {reopened} recurring tasks")),
                                    None,
                                )
                            } else {
                                (true, Some("Skipped (no storage)".to_string()), None)
                            }
                        }
                        _ if task.payload.is_empty() => {
                            debug!("Skipping task with empty payload: {}", task.name);
                            (true, Some("Skipped (empty payload)".to_string()), None)
                        }
                        _ => {
                            // Heartbeat and cron jobs run as full agent prompts
                            // (tools, memory, model fallback) in a task-scoped
                            // session that is not persisted to the session store.
                            let session_id = format!("scheduled-{}", task.id);
                            match agent.chat(&session_id, &task.payload, None, &[]).await {
                                Ok(result) => {
                                    let heartbeat_ok = task.name == "heartbeat"
                                        && result.content.trim().contains("HEARTBEAT_OK");
                                    if heartbeat_ok {
                                        debug!("Heartbeat: OK (nothing to do)");
                                    } else {
                                        info!(
                                            "Scheduled task '{}' completed: {}",
                                            task.name,
                                            result.content.chars().take(200).collect::<String>()
                                        );
                                    }
                                    if task.target_channel.is_some() {
                                        warn!(
                                            "Task '{}' targets a channel; channel routing from the \
                                             daemon scheduler is not implemented yet",
                                            task.name
                                        );
                                    }
                                    (true, Some(result.content), None)
                                }
                                Err(e) => {
                                    error!("Scheduled task '{}' failed: {}", task.name, e.message);
                                    (false, None, Some(e.message))
                                }
                            }
                        }
                    };
                    nanna_core::TaskResult {
                        task_id: task.id.clone(),
                        task_name: task.name.clone(),
                        success,
                        output,
                        error,
                        duration_ms: start.elapsed().as_millis() as u64,
                        started_at,
                        finished_at: chrono::Utc::now(),
                    }
                })
            });
            scheduler = scheduler.with_executor(executor);
            scheduler.start();
            info!("Daemon scheduler started (heartbeat + cron runner)");
            Arc::new(tokio::sync::RwLock::new(scheduler))
        };

        // Create control plane with all services (including router for consolidation)
        let mut control = ControlPlane::with_all_services(
            self.sessions.clone(),
            agent,
            memory.clone(),
            Some(tools),
            Some(router),
        )
        .with_tools_dir(tools_dir)
        .with_event_tx(self.ipc.event_sender())
        .with_workspace_id(workspace_id_for_services)
        .with_scheduler(scheduler)
        .with_task_runs(Arc::new(crate::tasks::TaskRunManager::new()));
        if let Some(ref buf) = self.log_buffer {
            control = control.with_log_buffer(buf.clone());
        }
        // Make the tracker the agent + sub-agents record into (from
        // init_services) the canonical one the control plane owns. Must happen
        // BEFORE with_storage, which loads persisted stats via
        // import_from_storage — those now land in the shared tracker too.
        control.model_stats = model_stats;
        // Load persisted model stats from storage
        if let Some(ref storage) = self.storage {
            control = control.with_storage(storage.clone()).await;
        }

        // Load persisted workspaces from database
        if let Some(ref storage) = self.storage {
            match storage.workspaces().list().await {
                Ok(records) if !records.is_empty() => {
                    let mut registry = control.workspaces().write().await;
                    let mut active_id = None;
                    for record in &records {
                        let path = PathBuf::from(&record.path);
                        if path.exists() {
                            let mut ws = nanna_core::Workspace::new(&path);
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
                            warn!("Persisted workspace path no longer exists: {}", record.path);
                        }
                    }
                    if let Some(id) = active_id {
                        registry.set_active(&id);
                        // Seed the tool working directory from the persisted active
                        // workspace so tools resolve against it from boot — not just
                        // after an interactive SetActive or the first workspace-scoped
                        // chat. Without this, a fresh daemon with a persisted active
                        // workspace left `default_workdir` at None until the user
                        // re-selected it, so tools fell back to the home dir instead of
                        // running "in the workspace you're in".
                        let active_path = registry.get(&id).map(|ws| ws.path.clone());
                        drop(registry);
                        if let (Some(tools), Some(path)) = (control.tools(), active_path) {
                            tools.set_default_workdir(Some(path.clone())).await;
                            info!(
                                "Seeded tool working directory from active workspace: {:?}",
                                path
                            );
                        }
                    } else {
                        drop(registry);
                    }
                    info!("Restored {} workspaces from database", records.len());
                }
                Ok(_) => {}
                Err(e) => {
                    warn!("Failed to load workspaces from database: {}", e);
                }
            }
        }

        // Wire model stats tracker into the router for health-aware routing.
        // The control plane owns the canonical tracker; the router reads it.
        if let Some(ref router) = control.router() {
            router.set_stats(control.model_stats.clone()).await;
            info!("Stats-informed routing enabled on LLM router");
        }

        // Shared channel status manager — attached before the Arc wrap so
        // ChannelAction::Status and ChannelManager listeners see the same state.
        let channel_status_manager = Arc::new(nanna_channels::StatusManager::new());
        control.set_status_manager(Arc::clone(&channel_status_manager));

        let control = Arc::new(control);

        // Take the request receiver from IPC server
        let mut request_rx =
            self.ipc.take_request_receiver().await.ok_or_else(|| {
                crate::DaemonError::Ipc("Request receiver already taken".to_string())
            })?;

        let mut shutdown_rx = self.shutdown_tx.subscribe();

        // Spawn IPC server
        let ipc_server = self.ipc.clone();
        let ipc_handle = tokio::spawn(async move {
            if let Err(e) = ipc_server.run().await {
                error!("IPC server error: {}", e);
            }
        });

        // Spawn health HTTP server if enabled
        let _health_state = if self.config.enable_health_server {
            // Seed durable-memory-store health (load already ran in init_services),
            // so a corrupt/degraded store shows on /status, not just a boot log.
            let (mem_degraded, mem_corrupt) = if let Some(ref m) = memory {
                let h = m.store_health().await;
                (h.degraded, h.corrupt_rows)
            } else {
                (false, 0)
            };
            let state = HealthState::new(
                memory.is_some(),
                true, // agent is available
            )
            .with_memory_health(mem_degraded, mem_corrupt);
            let health_state = Arc::new(state);

            // Update session count
            let sessions_for_health = self.sessions.clone();
            let health_state_clone = health_state.clone();
            tokio::spawn(async move {
                loop {
                    let count = sessions_for_health.count().await;
                    health_state_clone.set_session_count(count).await;
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            });

            // Serve the SAME state the session-count loop above updates
            // (via `from_shared`), so `/status` reflects live counts instead of
            // a throwaway copy stuck at zero. The server logs its own
            // "listening" line from `run()` once the bind succeeds, so we don't
            // pre-log here (that duplicate line falsely implied a bind before
            // one had happened).
            let health_server = HealthServer::from_shared(
                health_state.clone(),
                &self.config.ipc.host,
                self.config.health_port,
            );
            health_server.spawn();

            Some(health_state)
        } else {
            None
        };

        // Start ChannelManager if any channels are configured.
        // This handles listener-based inbound (polling) and routes responses back out.
        let channel_manager = if let Some(ref channels_config) = self.config.channels {
            // Build a daemon-local ChannelsConfig from the nanna_config::ChannelsConfig.
            // We re-map from nanna_config types to the daemon-local types.
            let daemon_channels = build_daemon_channels_config(channels_config);

            let mut manager = ChannelManager::with_status_manager(
                Arc::clone(&control),
                Arc::clone(&channel_status_manager),
            );
            manager.configure(&daemon_channels).await;

            // Also register outbound channels for webhook-sourced providers that have
            // bot tokens in the channel config (Telegram, Discord, Slack).
            // The listener-based configure() already does this; this is a no-op guard.

            match manager.start().await {
                Ok(()) => {
                    info!("Channel manager started");
                    Some(Arc::new(manager))
                }
                Err(e) => {
                    error!("Failed to start channel manager: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Spawn webhook HTTP server if enabled
        if self.config.enable_webhook_server {
            let mut webhook_config = self.config.webhook.clone();
            webhook_config.host = self.config.ipc.host.clone();
            webhook_config.port = self.config.webhook_port;

            // Keep a copy of the config for outbound channel registration below
            let webhook_config_copy = webhook_config.clone();

            let (webhook_server, mut webhook_rx) = WebhookServer::new(webhook_config);

            // Spawn the webhook server
            tokio::spawn(async move {
                if let Err(e) = webhook_server.run().await {
                    error!("Webhook server error: {}", e);
                }
            });

            // Build a shared router for the webhook event processor.
            // If a ChannelManager is running, share its router so outbound channels
            // (bot tokens) are already registered.  Otherwise create a standalone
            // router that may only cover providers registered via webhook config.
            let webhook_router = if let Some(ref mgr) = channel_manager {
                mgr.router()
            } else {
                // No channel manager — create a standalone router.
                // Outbound channels can be registered here from webhook config if
                // bot tokens are provided.
                let standalone_router =
                    Arc::new(tokio::sync::RwLock::new(ChannelMessageRouter::new()));

                // Register outbound channels from webhook config credentials
                {
                    let mut router = standalone_router.write().await;
                    if let Some(ref token) = webhook_config_copy.telegram_token {
                        router.register("telegram", Box::new(TelegramChannel::new(token)));
                        info!("Registered Telegram outbound channel from webhook config");
                    }
                    if webhook_config_copy.discord_public_key.is_some() {
                        // discord_public_key is for verification; bot token for sending
                        // is not separately stored in WebhookConfig currently.
                        // Log a warning — users should configure channels.discord instead.
                        debug!(
                            "Discord public key found in webhook config; for outbound replies configure channels.discord with a bot_token"
                        );
                    }
                }

                standalone_router
            };

            // Spawn webhook event processor — routes events through the same pipeline
            // as channel listener messages.
            let control_for_webhooks = Arc::clone(&control);
            tokio::spawn(async move {
                while let Some(event) = webhook_rx.recv().await {
                    debug!("Webhook event from {}: {:?}", event.source, event.message);

                    if let Some(ref msg) = event.message {
                        // Convert WebhookMessage → IncomingMessage
                        let incoming = IncomingMessage {
                            id: msg
                                .message_id
                                .clone()
                                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                            channel: ChannelId::new(&event.source, &msg.chat_id),
                            sender: ChannelSender {
                                id: msg.sender_id.clone(),
                                name: msg.sender_name.clone(),
                                username: None,
                            },
                            content: MessageContent::Text {
                                text: msg.content.clone(),
                            },
                            timestamp: event.timestamp,
                            reply_to: None,
                        };

                        // Process through the same pipeline as channel listeners
                        let router_guard = webhook_router.read().await;
                        ChannelManager::process_message(
                            incoming,
                            &control_for_webhooks,
                            &router_guard,
                        )
                        .await;
                    }
                }
            });

            info!(
                "Webhook server listening on http://{}:{}",
                self.config.ipc.host, self.config.webhook_port
            );
        }

        // Sessions are now persisted via Turso write-through on every mutation.
        // No more periodic JSON auto-save — each create/message/delete/rename writes to DB immediately.

        // Spawn model + tool stats auto-save task (every 5 minutes)
        let stats_control = control.clone();
        let mut stats_shutdown = self.shutdown_tx.subscribe();
        let stats_save_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(300));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        stats_control.save_model_stats().await;
                        stats_control.save_tool_stats().await;
                    }
                    _ = stats_shutdown.recv() => {
                        // Final save on shutdown
                        stats_control.save_model_stats().await;
                        stats_control.save_tool_stats().await;
                        info!("Model + tool stats final save completed");
                        break;
                    }
                }
            }
        });

        // Spawn sub-agent check-in task: periodically check running sub-agents
        // and drain parent mailboxes so questions don't go stale.
        // When a sub-agent uses ask_parent, the ParentChannelImpl handles it directly
        // via an LLM call. This task handles any orphaned mailbox messages and provides
        // visibility into long-running sub-agents.
        {
            let sessions = self.sessions.clone();
            let ipc_events = self.ipc.event_sender();
            let mut checkin_shutdown = self.shutdown_tx.subscribe();
            tokio::spawn(async move {
                // Check every 30 seconds
                let mut interval = tokio::time::interval(Duration::from_secs(30));
                loop {
                    tokio::select! {
                        _ = interval.tick() => {
                            let running = sessions.list_sub_sessions(None).await;
                            let running: Vec<_> = running.into_iter()
                                .filter(|s| matches!(s.state, crate::session::SubSessionState::Running | crate::session::SubSessionState::Spawning))
                                .collect();

                            if running.is_empty() {
                                continue;
                            }

                            // Check each parent's mailbox for pending questions
                            for sub in &running {
                                if let Some(ref parent_id) = sub.parent_id {
                                    let messages = sessions.drain_mailbox(parent_id).await;
                                    for msg in messages {
                                        // Re-emit as events for any listening clients (GUI)
                                        let _ = ipc_events.send(crate::protocol::Event::SubSessionQuestion {
                                            session_id: sub.session_id.clone(),
                                            parent_id: Some(parent_id.clone()),
                                            label: sub.label.clone(),
                                            question: msg.content,
                                        });
                                    }
                                }
                            }
                        }
                        _ = checkin_shutdown.recv() => break,
                    }
                }
            });
        }

        info!(
            "Daemon ready. IPC server listening on ws://{}",
            self.ipc.address()
        );

        // Main event loop
        //
        // Every request is dispatched as a tokio task so the loop is purely
        // a router — it never blocks.  The multi-threaded runtime (default
        // for `Runtime::new()`) schedules tasks across worker threads, so
        // concurrent requests (e.g. session creation while an agent is
        // running) execute in parallel.
        loop {
            tokio::select! {
                Some((client_id, request)) = request_rx.recv() => {
                    let control = control.clone();
                    let ipc = self.ipc.clone();

                    tokio::spawn(async move {
                        let request_id = request.id.clone();
                        let result = control.handle(&client_id, request.action).await;
                        let response = Response::success(request_id, result);
                        if let Err(e) = ipc.send_response(&client_id, response).await {
                            warn!("Failed to send response to client {}: {}", client_id, e);
                        }
                    });
                }

                _ = shutdown_rx.recv() => {
                    info!("Shutdown signal received");
                    break;
                }
            }
        }

        // Cleanup
        info!("Shutting down daemon...");
        self.ipc.shutdown();

        // Keep channel_manager alive until the end of run() so the spawned
        // listener task doesn't get prematurely shut down via shutdown_tx drop.
        // Explicit drop here makes the intent clear.
        drop(channel_manager);

        // Wait for stats auto-save task to complete final save
        let _ = tokio::time::timeout(Duration::from_secs(5), stats_save_handle).await;

        ipc_handle.abort();

        // Release PID file
        if let Some(ref pid_file) = self.pid_file {
            pid_file.release();
        }

        info!("Daemon stopped");
        Ok(())
    }

    /// Initialize all services
    async fn init_services(
        &self,
    ) -> Result<
        (
            Arc<ToolRegistry>,
            Option<Arc<MemoryService>>,
            Arc<AgentService>,
            Arc<LlmRouter>,
            Option<PathBuf>,                          // tools_dir
            Arc<tokio::sync::RwLock<Option<String>>>, // workspace_id for script services
            nanna_agent::ModelStatsTracker,           // shared model-stats tracker
        ),
        crate::DaemonError,
    > {
        // Create LLM router with all available providers
        let mut router = LlmRouter::new();
        let store = SecureStore::new();

        // Add Anthropic if credentials available
        if self.config.llm.anthropic_use_oauth {
            if let Some(ref oauth_token) = self.config.llm.anthropic_oauth_token {
                info!("Adding Anthropic provider (OAuth)");
                router = router.with_anthropic_oauth(oauth_token);
            }
        } else if let Some(ref api_key) = self.config.llm.anthropic_api_key {
            info!("Adding Anthropic provider (API key from config)");
            router = router.with_anthropic(api_key);
        } else if let Ok(api_key) = store.get(credentials::keys::ANTHROPIC_API_KEY) {
            info!("Adding Anthropic provider (API key from keyring)");
            router = router.with_anthropic(&api_key);
        } else if let Ok(loaded) = nanna_config::ClaudeCredentialManager::new().load() {
            info!("Adding Anthropic provider (Claude CLI OAuth)");
            router = router.with_anthropic_oauth(&loaded.credential.access_token);
        }

        // Add OpenAI if credentials available
        if let Some(ref api_key) = self.config.llm.openai_api_key {
            info!("Adding OpenAI provider (from config)");
            router = router.with_openai(api_key);
        } else if let Ok(api_key) = store.get(credentials::keys::OPENAI_API_KEY) {
            info!("Adding OpenAI provider (from keyring)");
            router = router.with_openai(&api_key);
        }

        // Add OpenRouter if credentials available
        if let Some(ref api_key) = self.config.llm.openrouter_api_key {
            info!("Adding OpenRouter provider (from config)");
            router = router.with_openrouter(api_key);
        } else if let Ok(api_key) = store.get(credentials::keys::OPENROUTER_API_KEY) {
            info!("Adding OpenRouter provider (from keyring)");
            router = router.with_openrouter(&api_key);
        }

        // Add GitHub Models if token available
        if let Some(ref token) = self.config.llm.github_token {
            info!("Adding GitHub Models provider (from config)");
            router = router.with_github_models(token);
        } else if let Ok(token) = store.get(credentials::keys::GITHUB_TOKEN) {
            info!("Adding GitHub Models provider (from keyring)");
            router = router.with_github_models(&token);
        }

        // Add Ollama (optionally with API key for remote instances)
        info!("Adding Ollama provider at {}", self.config.llm.ollama_host);
        if let Some(ref key) = self.config.llm.ollama_api_key {
            if !key.is_empty() {
                router = router.with_ollama_authenticated(&self.config.llm.ollama_host, key);
            } else {
                router = router.with_ollama(&self.config.llm.ollama_host);
            }
        } else {
            router = router.with_ollama(&self.config.llm.ollama_host);
        }

        let available = router.available_providers();
        if available.is_empty() {
            return Err(crate::DaemonError::Config(
                "No LLM providers configured. Please set up at least one provider (Anthropic, OpenAI, OpenRouter, or Ollama).".to_string()
            ));
        }
        info!(
            "LLM router initialized with {} providers: {:?}",
            available.len(),
            available
        );
        let router = Arc::new(router);

        // Create empty tool registry — all tools loaded from disk
        let tools = Arc::new(ToolRegistry::new());

        // Resolve tools directory (env var > config > dev fallback > {data_dir}/tools/)
        let tools_dir = if self.config.use_script_tools {
            let config_dir = self.config.tools_dir.as_deref();
            let resolved = nanna_tools::skills::defaults::resolve_tools_dir(config_dir)
                .unwrap_or_else(|| self.config.data_dir.join("tools"));

            // Bootstrap default skills into the tools directory if needed.
            // In debug builds this is a no-op (tools load from source tree).
            // In release builds, embedded skills are extracted on first run.
            let bootstrapped = nanna_tools::skills::defaults::bootstrap_default_skills(&resolved);
            if bootstrapped > 0 {
                info!(
                    "Bootstrapped {} default skills into {:?}",
                    bootstrapped, resolved
                );
            }

            if resolved.is_dir() {
                nanna_tools::skills::defaults::ensure_permissions(&resolved);
                info!("Tools directory: {:?}", resolved);
            } else {
                warn!("Tools directory does not exist: {:?}", resolved);
            }
            Some(resolved)
        } else {
            None
        };

        // Initialize memory service with embeddings if enabled
        let memory: Option<Arc<MemoryService>> = if self.config.enable_memory {
            // Build primary embedding client
            let primary_client = match self.embedding.provider.as_str() {
                "openai" => {
                    let api_key = std::env::var("OPENAI_API_KEY").ok();
                    api_key.map(|key| {
                        info!("Primary embeddings: OpenAI {}", self.embedding.model);
                        (
                            EmbeddingProviderInfo {
                                name: "openai".into(),
                                model: self.embedding.model.clone(),
                            },
                            Arc::new(
                                nanna_llm::EmbeddingClient::openai(&key)
                                    .with_model(&self.embedding.model),
                            ),
                        )
                    })
                }
                "openrouter" => {
                    let api_key = self
                        .config
                        .llm
                        .openrouter_api_key
                        .clone()
                        .or_else(|| std::env::var("OPENROUTER_API_KEY").ok());
                    api_key.map(|key| {
                        info!("Primary embeddings: OpenRouter {}", self.embedding.model);
                        (
                            EmbeddingProviderInfo {
                                name: "openrouter".into(),
                                model: self.embedding.model.clone(),
                            },
                            Arc::new(
                                nanna_llm::EmbeddingClient::openai(&key)
                                    .with_model(&self.embedding.model)
                                    .with_base_url("https://openrouter.ai/api"),
                            ),
                        )
                    })
                }
                "ollama" | _ => {
                    info!(
                        "Primary embeddings: Ollama {} at {}",
                        self.embedding.model, self.embedding.ollama_host
                    );
                    Some((
                        EmbeddingProviderInfo {
                            name: "ollama".into(),
                            model: self.embedding.model.clone(),
                        },
                        Arc::new(
                            nanna_llm::EmbeddingClient::ollama(&self.embedding.ollama_host)
                                .with_model(&self.embedding.model),
                        ),
                    ))
                }
            };

            match primary_client {
                Some((primary_info, primary)) => {
                    // Build the embedding router with fallback providers
                    let mut embed_router = EmbeddingRouter::new(primary_info.clone(), primary);

                    // Add fallback providers from available credentials
                    if primary_info.name != "openai" {
                        if let Some(api_key) = std::env::var("OPENAI_API_KEY").ok() {
                            let fallback_model = "text-embedding-3-small".to_string();
                            info!("Adding OpenAI embedding fallback: {}", fallback_model);
                            embed_router = embed_router.with_fallback(
                                EmbeddingProviderInfo {
                                    name: "openai".into(),
                                    model: fallback_model.clone(),
                                },
                                Arc::new(
                                    nanna_llm::EmbeddingClient::openai(&api_key)
                                        .with_model(&fallback_model),
                                ),
                            );
                        }
                    }
                    if primary_info.name != "openrouter" {
                        if let Some(api_key) = self
                            .config
                            .llm
                            .openrouter_api_key
                            .clone()
                            .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
                        {
                            let fallback_model = "openai/text-embedding-3-small".to_string();
                            info!("Adding OpenRouter embedding fallback: {}", fallback_model);
                            embed_router = embed_router.with_fallback(
                                EmbeddingProviderInfo {
                                    name: "openrouter".into(),
                                    model: fallback_model.clone(),
                                },
                                Arc::new(
                                    nanna_llm::EmbeddingClient::openai(&api_key)
                                        .with_model(&fallback_model)
                                        .with_base_url("https://openrouter.ai/api"),
                                ),
                            );
                        }
                    }
                    if primary_info.name != "ollama" {
                        let fallback_model = "nomic-embed-text".to_string();
                        info!(
                            "Adding Ollama embedding fallback: {} at {}",
                            fallback_model, self.embedding.ollama_host
                        );
                        embed_router = embed_router.with_fallback(
                            EmbeddingProviderInfo {
                                name: "ollama".into(),
                                model: fallback_model.clone(),
                            },
                            Arc::new(
                                nanna_llm::EmbeddingClient::ollama(&self.embedding.ollama_host)
                                    .with_model(&fallback_model),
                            ),
                        );
                    }

                    info!(
                        "Embedding router: {} providers configured",
                        embed_router.provider_count()
                    );
                    let embed_router = Arc::new(embed_router);

                    // Create embedding function that routes through the EmbeddingRouter.
                    // Tracks the router generation to detect provider switches.
                    let router_for_fn = embed_router.clone();
                    let last_generation =
                        Arc::new(std::sync::atomic::AtomicU64::new(embed_router.generation()));
                    // Placeholder for memory service — set after construction via lazy init
                    let memory_for_reembed: Arc<tokio::sync::OnceCell<Arc<MemoryService>>> =
                        Arc::new(tokio::sync::OnceCell::new());
                    let mem_cell_for_fn = memory_for_reembed.clone();

                    let embed_fn: nanna_memory::EmbedFn = Arc::new(move |text: &str| {
                        let router = router_for_fn.clone();
                        let text = text.to_string();
                        let gen_tracker = last_generation.clone();
                        let mem_cell = mem_cell_for_fn.clone();
                        Box::pin(async move {
                            let (embedding, provider_changed) = router.embed_one(&text).await?;

                            // If the provider changed, check if we need to re-embed
                            if provider_changed {
                                let new_gen = router.generation();
                                let old_gen =
                                    gen_tracker.swap(new_gen, std::sync::atomic::Ordering::Relaxed);
                                if new_gen != old_gen {
                                    // Provider switched — trigger re-embed in background
                                    if let Some(mem) = mem_cell.get() {
                                        let mem = mem.clone();
                                        tokio::spawn(async move {
                                            tracing::info!(
                                                "Embedding provider changed (gen {} → {}), probing dimension and re-embedding if needed...",
                                                old_gen,
                                                new_gen
                                            );
                                            match mem.probe_and_align_dimension().await {
                                                Ok(dim) => tracing::info!(
                                                    "Dimension probe complete: {} dims",
                                                    dim
                                                ),
                                                Err(e) => {
                                                    tracing::warn!("Dimension probe failed: {}", e)
                                                }
                                            }
                                        });
                                    }
                                }
                            }

                            Ok(embedding)
                        })
                    });

                    // Seed the embedding dimension by probing the router.
                    //
                    // A probe failure must NOT stop the daemon: Nanna is
                    // offline-capable by default, so an unreachable or unkeyed
                    // embedding provider degrades memory — it does not refuse
                    // to boot. The seed only has to be a valid positive
                    // dimension: real vectors always come from the provider,
                    // and the background `probe_and_align_dimension` below
                    // corrects the store (re-embedding any mismatched entries)
                    // as soon as a provider answers. Probing here is purely an
                    // optimization — when it succeeds the store is right
                    // immediately and nothing is ever re-embedded.
                    let seed_dimension = nanna_memory::MemoryServiceConfig::default().dimension;
                    let dimension = match Self::probe_embedding_dimension(&embed_router).await {
                        Ok(dim) => {
                            info!(
                                "Memory service using probed dimension {} for model {}",
                                dim, self.embedding.model
                            );
                            dim
                        }
                        Err(e) => {
                            warn!(
                                "Could not probe the embedding dimension ({e}). Starting anyway with a \
                                 provisional dimension of {seed_dimension}; memory will re-align \
                                 automatically once an embedding provider is reachable. To enable \
                                 embeddings, run a local Ollama with `ollama pull {}` or set an \
                                 OpenAI/OpenRouter key.",
                                self.embedding.model
                            );
                            seed_dimension
                        }
                    };
                    assert!(dimension > 0, "embedding dimension must be positive");
                    let config = nanna_memory::MemoryServiceConfig {
                        dimension,
                        ..Default::default()
                    };

                    // Wire up Turso persistence if storage is available.
                    // The persistence adapter is constructed here and attached to the
                    // MemoryService so all writes are automatically mirrored to Turso.
                    let memory_service = if let Some(ref storage) = self.storage {
                        let repo = storage.memories();
                        let db = Arc::new(TursoMemoryPersistence::new(repo));
                        nanna_memory::MemoryService::new(config)
                            .with_embed_fn(embed_fn)
                            .with_persistence(db)
                    } else {
                        warn!(
                            "No storage backend available — memory will NOT be persisted to Turso"
                        );
                        nanna_memory::MemoryService::new(config).with_embed_fn(embed_fn)
                    };

                    // One-time migration: if memories.json exists and Turso is empty,
                    // load from JSON into in-memory cache then save each entry to Turso.
                    let json_path = self.memory_path.as_ref();
                    let should_migrate = if let (Some(path), Some(storage)) =
                        (json_path, &self.storage)
                    {
                        if path.exists() {
                            match storage.memories().count().await {
                                Ok(0) => true,
                                Ok(n) => {
                                    info!(
                                        "Turso already has {} memories — skipping JSON migration",
                                        n
                                    );
                                    false
                                }
                                Err(e) => {
                                    warn!("Could not check Turso memory count: {}", e);
                                    false
                                }
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if should_migrate {
                        let path = json_path.unwrap();
                        info!(
                            "Migrating memories from {:?} to Turso (one-time migration)",
                            path
                        );
                        match memory_service.load(path).await {
                            Ok(()) => {
                                let count = memory_service.count().await;
                                info!("Loaded {} memories from JSON, flushing to Turso...", count);
                                // Flush all entries to Turso
                                match memory_service.flush_to_db().await {
                                    Ok(n) => info!("Flushed {} memories to Turso", n),
                                    Err(e) => warn!("Failed to flush memories to Turso: {}", e),
                                }
                                // Rename the JSON file so we don't re-migrate next time
                                let migrated_path = path.with_extension("json.migrated");
                                if let Err(e) = tokio::fs::rename(path, &migrated_path).await {
                                    warn!("Could not rename migrated JSON file: {}", e);
                                } else {
                                    info!(
                                        "Renamed {:?} → {:?} (migration complete)",
                                        path, migrated_path
                                    );
                                }
                            }
                            Err(e) => {
                                warn!(
                                    "JSON migration failed: {}. Will attempt to load from Turso.",
                                    e
                                );
                            }
                        }
                    }

                    // Load from Turso into the in-memory cache (normal startup path).
                    // Skipped if we just migrated (the entries are already in-memory from the JSON load above).
                    if !should_migrate {
                        match memory_service.load_from_db().await {
                            Ok(count) => {
                                info!("Loaded {} memories from Turso", count);
                            }
                            Err(nanna_memory::MemoryError::Persistence(ref e))
                                if e.contains("No persistence backend") =>
                            {
                                // No storage configured — silently skip
                            }
                            Err(e) => {
                                warn!("Failed to load memories from Turso: {}", e);
                            }
                        }
                    }

                    info!("Memory service initialized with Turso persistence and embedding router");
                    let memory_arc = Arc::new(memory_service);

                    // Probe the actual embedding dimension from the model IN THE
                    // BACKGROUND. The probe's first embed call can take ~a minute
                    // when the local embedding model is cold (Ollama loads it on
                    // demand), and it used to block startup past the GUI's
                    // daemon-ready timeout — forcing an embedded fallback while an
                    // orphaned daemon kept running. `probe_and_align_dimension`
                    // takes `&self` on the Arc'd service specifically so it can run
                    // at runtime; a mismatched dimension is corrected (and entries
                    // re-embedded) as soon as the probe completes.
                    {
                        let memory_for_probe = memory_arc.clone();
                        let model_name = self.embedding.model.clone();
                        tokio::spawn(async move {
                            match memory_for_probe.probe_and_align_dimension().await {
                                Ok(actual_dim) => {
                                    if actual_dim == dimension {
                                        debug!(
                                            "Embedding dimension confirmed: {actual_dim} for model {model_name}"
                                        );
                                    } else {
                                        info!(
                                            "Embedding dimension corrected: {dimension} → {actual_dim} for model {model_name}"
                                        );
                                    }
                                }
                                Err(e) => {
                                    warn!(
                                        "Could not probe embedding dimension (model may be loading): {e}. \
                                         Using static dimension {dimension}."
                                    );
                                }
                            }
                        });
                    }

                    // Wire the memory service into the embed_fn's OnceCell
                    // so provider-switch re-embedding can find it
                    let _ = memory_for_reembed.set(memory_arc.clone());

                    Some(memory_arc)
                }
                None => {
                    warn!("Memory service disabled: no embedding provider available");
                    warn!(
                        "To enable memory: set OPENAI_API_KEY or use Ollama with embeddings model"
                    );
                    None
                }
            }
        } else {
            info!("Memory service disabled in config");
            None
        };

        // Shared session history for the recall_messages tool service
        let session_history: SharedSessionHistory = Arc::new(tokio::sync::RwLock::new(Vec::new()));

        // One shared model-stats tracker for the whole daemon: the main agent
        // (AgentService) and every sub-agent (AgentSpawnerImpl) record into it,
        // and the control plane makes it canonical (persists it + feeds the
        // router). Cloning shares state (Arc<RwLock<_>> inside).
        let model_stats = nanna_agent::ModelStatsTracker::new();

        // Build script services and load all tools from disk
        let workspace_id_for_services: Arc<tokio::sync::RwLock<Option<String>>> =
            Arc::new(tokio::sync::RwLock::new(None));
        {
            let spawner_arc: Option<Arc<dyn AgentSpawner + Send + Sync>> = if !router
                .available_providers()
                .is_empty()
            {
                // Build model routing for sub-agents from the priority list.
                // Cheapest model (last in priority) handles Simple tasks,
                // most capable (first) handles Complex. This gives sub-agents
                // intelligent per-iteration routing while the main agent always
                // uses the primary model.
                let sub_agent_routing = build_sub_agent_routing(&self.config.agent.model_priority);
                if !sub_agent_routing.is_empty() {
                    info!(
                        "Sub-agent routing: {:?}",
                        sub_agent_routing
                            .iter()
                            .map(|t| format!("{}:{:?}", t.model, t.tier))
                            .collect::<Vec<_>>()
                    );
                }

                Some(Arc::new(AgentSpawnerImpl {
                    router: router.clone(),
                    tools: tools.clone(),
                    agent_config: nanna_agent::AgentConfig {
                        model: self.config.agent.model.clone(),
                        max_tokens: self.config.agent.max_tokens,
                        temperature: self.config.agent.temperature,
                        max_iterations: None, // Unlimited — model stops when done
                        thinking_mode: self.config.agent.thinking_mode,
                        summarization_priority: self.config.agent.summarization_priority.clone(),
                        summarization_ollama_url: self
                            .config
                            .agent
                            .summarization_ollama_url
                            .clone(),
                        sub_agent_model: self.config.agent.sub_agent_model.clone(),
                        model_routing: sub_agent_routing,
                        routing_first_turn_primary: true,
                        openrouter_api_key: self.config.agent.openrouter_api_key.clone(),
                        openai_api_key: self.config.agent.openai_api_key.clone(),
                        ..Default::default()
                    },
                    system_prompt: nanna_agent::prompts::DEFAULT_SYSTEM_PROMPT.to_string(),
                    workspace_root: None,
                    workspace_context: None,
                    stats: Some(model_stats.clone()),
                }))
            } else {
                None
            };

            let services = build_script_services(
                &memory,
                spawner_arc,
                session_history.clone(),
                workspace_id_for_services.clone(),
                self.storage.clone(),
            );

            if let Some(ref dir) = tools_dir {
                if dir.is_dir() {
                    let loaded = tools.load_skills_with_services(dir, &services).await;
                    info!("Loaded {} tools from {:?}", loaded, dir);
                }
            }
        }

        // Register common aliases for Claude Code compatibility (after tools are loaded)
        tools.register_alias("read", "read_file").await;
        tools.register_alias("Read", "read_file").await;
        tools.register_alias("write", "write_file").await;
        tools.register_alias("Write", "write_file").await;
        tools.register_alias("bash", "exec").await;
        tools.register_alias("Bash", "exec").await;
        tools.register_alias("glob", "list_dir").await;
        tools.register_alias("Glob", "list_dir").await;
        tools.register_alias("ls", "list_dir").await;

        {
            let tool_count = tools.definitions().await.len();
            info!("Tool registry: {} tools (including aliases)", tool_count);
        }

        // Register discover_tools (JS/TS skill with registry access)
        if let Some(ref dir) = tools_dir {
            if let Some(source) = nanna_tools::skills::defaults::load_discover_tools_source(dir) {
                let wrapper = nanna_tools::skills::ScriptedToolWrapper::from_source(
                    "discover_tools",
                    &source,
                )
                .expect("discover_tools skill must parse")
                .with_registry(Arc::downgrade(&tools));
                tools.register(wrapper).await;
                info!("Registered discover_tools skill from {:?}", dir);
            } else {
                warn!("discover_tools not found in tools directory");
            }
        }

        // Register ask_parent tool for sub-agent ↔ parent communication
        {
            let parent_channel: Arc<dyn ParentChannel + Send + Sync> =
                Arc::new(ParentChannelImpl {
                    sessions: self.sessions.clone(),
                    event_tx: Some(self.ipc.event_sender()),
                    router: router.clone(),
                    model: self.config.agent.model.clone(),
                });
            tools
                .register(nanna_tools::AskParentTool::new(
                    parent_channel,
                    tools.clone(),
                ))
                .await;
            info!("Registered ask_parent tool for sub-agent communication");
        }

        // Create agent service with multi-provider router
        let event_tx = self.ipc.event_sender();
        let mut agent_service = AgentService::with_data_dir(
            self.config.agent.clone(),
            router.clone(),
            tools.clone(),
            memory.clone(),
            event_tx,
            Some(self.config.data_dir.clone()),
        )
        .with_session_history(session_history)
        .with_stats(model_stats.clone());
        if let Some(ref storage) = self.storage {
            agent_service = agent_service.with_storage(storage.clone());
        }
        let agent = Arc::new(agent_service);

        info!("Agent service initialized");

        Ok((
            tools,
            memory,
            agent,
            router,
            tools_dir,
            workspace_id_for_services,
            model_stats,
        ))
    }
}

/// Embedding configuration for the daemon
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    /// Provider (ollama, openai, openrouter)
    pub provider: String,
    /// Model name
    pub model: String,
    /// Ollama host (if using Ollama)
    pub ollama_host: String,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: "ollama".to_string(),
            model: "nomic-embed-text".to_string(),
            ollama_host: "http://localhost:11434".to_string(),
        }
    }
}

/// Builder for DaemonServer
pub struct DaemonBuilder {
    config: DaemonConfig,
    embedding: EmbeddingConfig,
    memory_path: Option<PathBuf>,
    brave_api_key: Option<String>,
    log_buffer: Option<crate::log_buffer::LogBuffer>,
}

impl DaemonBuilder {
    pub fn new() -> Self {
        Self {
            config: DaemonConfig::default(),
            embedding: EmbeddingConfig::default(),
            memory_path: None,
            brave_api_key: None,
            log_buffer: None,
        }
    }

    /// Create builder from Nanna config file
    pub fn from_nanna_config() -> Result<Self, crate::DaemonError> {
        use nanna_config::Config;

        let config = match Config::load() {
            Ok(cfg) => {
                info!("Loaded Nanna config successfully");
                cfg.with_env_overrides()
            }
            Err(e) => {
                warn!(
                    "Failed to load Nanna config: {}, using defaults with env overrides",
                    e
                );
                Config::default().with_env_overrides()
            }
        };

        let mut builder = Self::new();

        // Set LLM configuration - copy all provider credentials
        builder.config.llm.provider = config.llm.provider.clone();
        builder.config.llm.anthropic_api_key = config.llm.api_key.clone(); // Anthropic API key
        builder.config.llm.anthropic_oauth_token = config.llm.anthropic_oauth_token.clone();
        builder.config.llm.anthropic_use_oauth = config.llm.anthropic_use_oauth;
        builder.config.llm.openai_api_key = config.llm.openai_api_key.clone();
        builder.config.llm.openrouter_api_key = config.llm.openrouter_api_key.clone();
        builder.config.llm.github_token = config.llm.github_token.clone();
        // Ollama host is stored in memory config
        builder.config.llm.ollama_host = config.memory.ollama_host.clone();

        // Set embedding configuration from Nanna memory config
        builder.embedding.provider = config.memory.embedding_provider.clone();
        builder.embedding.model = config.memory.embedding_model.clone();
        builder.embedding.ollama_host = config.memory.ollama_host.clone();

        // Thread the memory-compression settings so the scheduled dream cycle
        // honors them (previously only the IPC-triggered path did).
        builder.config.memory_max_compression_ratio = config.memory.max_compression_ratio;
        builder.config.memory_min_remaining_memories = config.memory.min_remaining_memories;

        // Set data directory from Nanna config (same location as GUI)
        match nanna_config::Config::default_data_dir() {
            Ok(data_dir) => {
                info!("Using Nanna data directory: {:?}", data_dir);
                builder.config.data_dir = data_dir.clone();
                builder.memory_path = Some(data_dir.join("memories.json"));
            }
            Err(e) => {
                warn!("Could not determine Nanna data dir: {}, using default", e);
            }
        }

        // Set agent configuration from loaded config
        // Use user-configured model priority list for fallback
        builder.config.agent.model_priority = config.llm.model_priority.clone();
        info!("Model priority list: {:?}", config.llm.model_priority);

        if let Some(model) = config.llm.model_priority.first() {
            builder.config.agent.model = model.to_string();
        } else {
            builder.config.agent.model = config.llm.model.clone();
        }

        // Set summarization configuration
        builder.config.agent.summarization_priority = config.llm.summarization_priority.clone();
        builder.config.agent.summarization_ollama_url = config.llm.ollama_url.clone();

        // Pass API keys to agent config so summarization can use OpenRouter/OpenAI
        builder.config.agent.openrouter_api_key = config.llm.openrouter_api_key.clone();
        builder.config.agent.openai_api_key = config.llm.openai_api_key.clone();

        // Set thinking mode from config
        if config.agent.thinking_enabled {
            builder.config.agent.thinking_mode = ThinkingMode::Medium;
        }

        // Agent-loop iteration policy: unbounded by default (long-horizon worker),
        // with late escalating soft nudges. All three are user-configurable.
        builder.config.agent.max_iterations = config.agent.max_iterations;
        builder.config.agent.nudge_after_iterations = config.agent.nudge_after_iterations;
        builder.config.agent.nudge_interval_iterations = config.agent.nudge_interval_iterations;

        // Set model routing configuration
        builder.config.agent.model_routing = config.llm.model_routing.clone();
        builder.config.agent.routing_first_turn_primary = config.llm.routing_first_turn_primary;
        builder.config.agent.sub_agent_model = config.llm.sub_agent_model.clone();
        if !config.llm.model_routing.is_empty() {
            info!("Model routing enabled: {:?}", config.llm.model_routing);
        }
        if let Some(ref sub_model) = config.llm.sub_agent_model {
            info!("Sub-agent model: {}", sub_model);
        }

        // Set Brave API key for web search
        builder.brave_api_key = config.tools.brave_api_key.clone();

        // Set script tools flag and tools directory
        builder.config.use_script_tools = config.tools.use_script_tools;
        builder.config.tools_dir = config.tools.tools_dir.clone();

        // Load channel configuration (Telegram, Discord, Slack, etc.)
        let has_channels = config.channels.telegram.is_some()
            || config.channels.discord.is_some()
            || config.channels.slack.is_some()
            || config.channels.signal.is_some()
            || config.channels.whatsapp.is_some();
        if has_channels {
            builder.config.channels = Some(config.channels.clone());
            info!("Channel configuration loaded");
        }

        // Log configured providers
        let mut providers = Vec::new();
        if builder.config.llm.anthropic_api_key.is_some()
            || builder.config.llm.anthropic_oauth_token.is_some()
        {
            providers.push("anthropic");
        }
        if builder.config.llm.openai_api_key.is_some() {
            providers.push("openai");
        }
        if builder.config.llm.openrouter_api_key.is_some() {
            providers.push("openrouter");
        }
        if builder.config.llm.github_token.is_some() {
            providers.push("github");
        }
        providers.push("ollama"); // Always available

        info!(
            "Daemon config loaded: model={}, embedding={}:{}, providers=[{}], brave_key={}",
            builder.config.agent.model,
            builder.embedding.provider,
            builder.embedding.model,
            providers.join(", "),
            if builder.brave_api_key.is_some() {
                "set"
            } else {
                "none"
            }
        );

        Ok(builder)
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.config.ipc.port = port;
        self
    }

    pub fn with_host(mut self, host: impl Into<String>) -> Self {
        self.config.ipc.host = host.into();
        self
    }

    pub fn with_data_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.data_dir = path.into();
        self
    }

    pub fn with_log_level(mut self, level: impl Into<String>) -> Self {
        self.config.log_level = level.into();
        self
    }

    pub fn with_auto_save_interval(mut self, secs: u64) -> Self {
        self.config.auto_save_interval_secs = secs;
        self
    }

    pub fn with_llm_provider(mut self, provider: impl Into<String>) -> Self {
        self.config.llm.provider = provider.into();
        self
    }

    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.config.llm.api_key = Some(key.into());
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.config.agent.model = model.into();
        self
    }

    pub fn with_memory(mut self, enable: bool) -> Self {
        self.config.enable_memory = enable;
        self
    }

    pub fn with_health_server(mut self, enable: bool) -> Self {
        self.config.enable_health_server = enable;
        self
    }

    pub fn with_health_port(mut self, port: u16) -> Self {
        self.config.health_port = port;
        self
    }

    pub fn with_pid_file(mut self, enable: bool) -> Self {
        self.config.enable_pid_file = enable;
        self
    }

    pub fn with_webhook_server(mut self, enable: bool) -> Self {
        self.config.enable_webhook_server = enable;
        self
    }

    pub fn with_webhook_port(mut self, port: u16) -> Self {
        self.config.webhook_port = port;
        self
    }

    pub fn with_webhook_config(mut self, config: WebhookConfig) -> Self {
        self.config.webhook = config;
        self
    }

    pub fn with_script_tools(mut self, enable: bool) -> Self {
        self.config.use_script_tools = enable;
        self
    }

    pub fn with_log_buffer(mut self, buffer: crate::log_buffer::LogBuffer) -> Self {
        self.log_buffer = Some(buffer);
        self
    }

    pub async fn build(self) -> DaemonServer {
        let mut server = DaemonServer::new(
            self.config,
            self.embedding,
            self.memory_path,
            self.brave_api_key,
        );
        server.log_buffer = self.log_buffer;

        // Initialize Turso storage for model stats persistence
        let db_path = server.config.data_dir.join("nanna.db");
        let storage_config = nanna_storage::StorageConfig {
            path: db_path.to_string_lossy().to_string(),
        };
        match nanna_storage::Storage::new(&storage_config).await {
            Ok(storage) => {
                info!("Storage initialized at {:?}", db_path);
                server.set_storage(Arc::new(storage));
            }
            Err(e) => {
                warn!(
                    "Failed to initialize storage: {}. Model stats will not persist.",
                    e
                );
            }
        }

        server
    }
}

impl Default for DaemonBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Helper: convert nanna_config::ChannelsConfig → daemon-local ChannelsConfig
// =============================================================================

/// Map the high-level `nanna_config::ChannelsConfig` (with its richer field names)
/// to the daemon-local `ChannelsConfig` used by `ChannelManager::configure()`.
///
/// Fields that exist in `nanna_config` but not in the daemon-local type are
/// silently dropped — the local type only covers what `ChannelManager` actually
/// needs at runtime.
/// Build model routing tiers for sub-agents from the model priority list.
///
/// Given a priority list like ["claude-opus-4-6", "openrouter/free", "ollama/qwen3.5:9b"],
/// assigns tiers so the cheapest model (last) handles Simple tasks, the most capable
/// (first) handles Complex, and everything in between gets Medium.
///
/// With 1 model: no routing (always use that model).
/// With 2 models: first=Complex, second=Simple.
/// With 3+: first=Complex, last=Simple, middle=Medium.
fn build_sub_agent_routing(model_priority: &[String]) -> Vec<nanna_agent::ModelTier> {
    use nanna_agent::{ModelTier, TaskComplexity};

    if model_priority.len() <= 1 {
        return vec![]; // No routing with a single model
    }

    let mut tiers = Vec::new();

    // Reversed: cheapest first (routing picks the first model whose tier >= complexity)
    for (i, model) in model_priority.iter().enumerate().rev() {
        let tier = if i == 0 {
            TaskComplexity::Complex // Most capable
        } else if i == model_priority.len() - 1 {
            TaskComplexity::Simple // Cheapest
        } else {
            TaskComplexity::Medium
        };
        tiers.push(ModelTier {
            model: model.clone(),
            tier,
        });
    }

    tiers
}

fn build_daemon_channels_config(src: &nanna_config::ChannelsConfig) -> ChannelsConfig {
    use crate::channels::{
        DiscordConfig as DaemonDiscord, SlackConfig as DaemonSlack,
        TelegramConfig as DaemonTelegram,
    };

    ChannelsConfig {
        telegram: src.telegram.as_ref().map(|tg| DaemonTelegram {
            bot_token: tg.bot_token.clone(),
            // nanna_config::TelegramConfig uses webhook_url; treat presence of it as
            // "use webhooks" mode (listener-based polling is disabled when webhook URL set).
            allowed_chats: tg.allowed_users.clone().unwrap_or_default(),
            use_webhooks: tg.webhook_url.is_some(),
        }),
        discord: src.discord.as_ref().map(|dc| DaemonDiscord {
            bot_token: dc.bot_token.clone(),
            allowed_guilds: vec![], // nanna_config::DiscordConfig has no allowed_guilds yet
            intents: None,
        }),
        slack: src.slack.as_ref().and_then(|sl| {
            // Slack Socket Mode listener requires an app_token; fall back gracefully
            sl.app_token.as_ref().map(|app_token| DaemonSlack {
                app_token: app_token.clone(),
                bot_token: sl.bot_token.clone(),
                allowed_channels: vec![], // nanna_config::SlackConfig has no allowed_channels yet
            })
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_heartbeat_prompt_does_not_command_file_read() {
        // The daemon overrides the scheduler default with its own prompt; guard
        // it too so neither site reintroduces the erroring `Read HEARTBEAT.md`.
        let p = DAEMON_HEARTBEAT_PROMPT.to_lowercase();
        assert!(!p.contains("read heartbeat"), "must not command a file read: {p}");
        assert!(!p.contains(".md"), "must not reference a bespoke .md file: {p}");
        assert!(p.contains("heartbeat_ok"), "must keep the HEARTBEAT_OK sentinel: {p}");
    }

    /// Serve exactly one HTTP request with a canned body, then exit.
    ///
    /// Bound on port 0 so the OS assigns a free port — the test never races a
    /// real Ollama or another test for a fixed port. Returns the base URL.
    async fn spawn_one_shot_embedding_server(body: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral port");
        let addr = listener.local_addr().expect("read back the bound addr");
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                // Read whatever the client sends; we only need the socket
                // drained enough to reply.
                let mut buf = [0_u8; 4096];
                let _ = stream.read(&mut buf).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.flush().await;
            }
        });
        format!("http://{addr}")
    }

    fn ollama_router_at(base_url: &str) -> EmbeddingRouter {
        EmbeddingRouter::new(
            EmbeddingProviderInfo {
                name: "ollama".into(),
                model: "nomic-embed-text".into(),
            },
            Arc::new(nanna_llm::EmbeddingClient::ollama(base_url).with_model("nomic-embed-text")),
        )
    }

    /// The probe reports the dimension the provider actually returned — it does
    /// not consult any per-model dimension table.
    #[tokio::test]
    async fn probe_reports_the_dimension_the_provider_returns() {
        let base =
            spawn_one_shot_embedding_server(r#"{"embeddings":[[0.1,0.2,0.3,0.4,0.5]]}"#).await;
        let dim = DaemonServer::probe_embedding_dimension(&ollama_router_at(&base))
            .await
            .expect("probe succeeds against a responsive provider");
        assert_eq!(dim, 5, "dimension comes from the response vector's length");
    }

    /// A provider that answers with an empty vector is an error, not a
    /// zero-dimension memory store.
    #[tokio::test]
    async fn probe_rejects_an_empty_embedding_vector() {
        let base = spawn_one_shot_embedding_server(r#"{"embeddings":[[]]}"#).await;
        let err = DaemonServer::probe_embedding_dimension(&ollama_router_at(&base))
            .await
            .expect_err("an empty vector must not pass as a valid dimension");
        assert!(!err.is_empty(), "the failure carries a reason");
    }

    /// The regression this fixes: an unreachable/unkeyed provider makes the
    /// probe fail, and the caller must be able to carry on. The probe returns
    /// `Err` rather than panicking, so boot can degrade to a provisional
    /// dimension instead of aborting.
    #[tokio::test]
    async fn probe_fails_cleanly_when_no_provider_answers() {
        // Bind then immediately drop the listener, so the port is (almost
        // certainly) closed — a stand-in for "no embedding provider running".
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        drop(listener);

        let err =
            DaemonServer::probe_embedding_dimension(&ollama_router_at(&format!("http://{addr}")))
                .await
                .expect_err("probe must fail when nothing is listening");
        assert!(
            err.contains("embedding providers failed"),
            "the router reports exhausting its providers, got: {err}"
        );
    }

    /// The seed the daemon falls back to must itself be a usable dimension —
    /// this is the value memory runs on until a provider answers.
    #[test]
    fn provisional_seed_dimension_is_positive() {
        assert!(
            nanna_memory::MemoryServiceConfig::default().dimension > 0,
            "a zero seed would make every add() fail before the probe realigns"
        );
    }

    #[test]
    fn scheduled_consolidation_config_threads_user_memory_settings() {
        // The scheduled dream cycle must use the user's compression settings,
        // not ConsolidationConfig::default(), so automatic and IPC-triggered
        // consolidation behave identically.
        let cfg = scheduled_consolidation_config(0.25, 100, 8_192);
        assert!((cfg.max_compression_ratio - 0.25).abs() < f32::EPSILON);
        assert_eq!(cfg.min_remaining_memories, 100);
        // Untouched fields keep their defaults (e.g. the member-count cap).
        let default = nanna_memory::ConsolidationConfig::default();
        assert_eq!(cfg.max_cluster_memories, default.max_cluster_memories);
        assert!((cfg.cluster_threshold - default.cluster_threshold).abs() < f32::EPSILON);
    }

    #[test]
    fn scheduled_consolidation_config_sizes_content_budget_to_the_model() {
        // A large-context summarizer gets a proportionally larger per-cluster
        // content budget than a small one (so big models consolidate more per
        // pass) — the whole point of threading the model's context window.
        let small = scheduled_consolidation_config(0.5, 20, 8_192);
        let large = scheduled_consolidation_config(0.5, 20, 200_000);
        assert!(large.max_cluster_content_bytes > small.max_cluster_content_bytes);
        assert_eq!(
            large.max_cluster_content_bytes,
            nanna_memory::cluster_content_bytes_for_context(200_000)
        );
    }

    #[test]
    fn daemon_config_default_mirrors_consolidation_defaults() {
        let daemon = DaemonConfig::default();
        let cons = nanna_memory::ConsolidationConfig::default();
        assert!(
            (daemon.memory_max_compression_ratio - cons.max_compression_ratio).abs() < f32::EPSILON
        );
        assert_eq!(
            daemon.memory_min_remaining_memories,
            cons.min_remaining_memories
        );
    }
}
