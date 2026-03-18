//! Daemon Server - Main daemon orchestrator
//!
//! Combines IPC server, control plane, sessions, persistence, and all subsystems.

use crate::agent_service::{AgentService, AgentServiceConfig};
use crate::control::ControlPlane;
use crate::memory_persistence::SqliteMemoryPersistence;
use crate::health::{HealthServer, HealthState, PidFile, DEFAULT_HEALTH_PORT};
use crate::ipc::{IpcServer, IpcServerConfig};
use crate::llm_router::LlmRouter;
use crate::persistence::PersistenceManager;
use crate::protocol::Response;
use crate::session::SessionManager;
use crate::webhook::{WebhookConfig, WebhookServer, DEFAULT_WEBHOOK_PORT};
use nanna_config::credentials::{self, SecureStore};
use nanna_llm::LlmClient;
use nanna_memory::MemoryService;
use nanna_tools::{
    ToolRegistry, AgentSpawner, SpawnResult,
};
use nanna_scripting::ServiceFn;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;

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
        max_iterations: usize,
    ) -> Result<SpawnResult, String> {
        use nanna_agent::{Agent, AgentContext, RunOptions};

        info!(description = description, max_iterations = max_iterations, "Spawning sub-agent");

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

        // Configure agent with overridden max_iterations
        let mut config = self.agent_config.clone();
        config.max_iterations = Some(max_iterations);

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
        let llm_client = self.router.client_for_model(model)
            .ok_or_else(|| {
                format!(
                    "No provider available for model '{}'. Available providers: {:?}",
                    model,
                    self.router.available_providers()
                )
            })?;

        // Strip provider prefix from model name for the actual API call
        config.model = LlmRouter::strip_model_prefix(&config.model);

        let mut agent = Agent::new(config, llm_client, self.tools.clone())
            .with_context(context);

        // Share model stats tracker with sub-agents
        if let Some(ref tracker) = self.stats {
            agent = agent.with_stats(tracker.clone());
        }

        let options = RunOptions {
            max_iterations: Some(max_iterations),
            ..Default::default()
        };

        // Run without timeout — agent stops when done or cancelled
        let result = agent.run(prompt, options)
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
        })
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
) -> HashMap<String, ServiceFn> {
    use serde_json::{json, Value};

    let mut services: HashMap<String, ServiceFn> = HashMap::new();

    // Memory services
    if let Some(mem) = memory {
        let mem_store = mem.clone();
        services.insert("memory.store".to_string(), Arc::new(move |params: Value| {
            let mem = mem_store.clone();
            Box::pin(async move {
                let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let tags: HashMap<String, String> = params.get("tags")
                    .and_then(|v| v.as_object())
                    .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect())
                    .unwrap_or_default();
                let importance = params.get("importance").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
                match mem.remember_with_importance(&content, tags, importance).await {
                    Ok((id, _)) => Ok(json!({"id": id})),
                    Err(e) => Err(e.to_string()),
                }
            })
        }));

        let mem_search = mem.clone();
        services.insert("memory.search".to_string(), Arc::new(move |params: Value| {
            let mem = mem_search.clone();
            Box::pin(async move {
                let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
                match mem.recall(&query).await {
                    Ok(results) => {
                        let items: Vec<Value> = results.into_iter().take(limit).map(|r| {
                            json!({"id": r.id, "content": r.content, "score": r.score})
                        }).collect();
                        Ok(Value::Array(items))
                    }
                    Err(e) => Err(e.to_string()),
                }
            })
        }));

        let mem_delete = mem.clone();
        services.insert("memory.delete".to_string(), Arc::new(move |params: Value| {
            let mem = mem_delete.clone();
            Box::pin(async move {
                let id = params.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                match mem.forget(&id).await {
                    Ok(()) => Ok(json!({"deleted": true})),
                    Err(e) => Err(e.to_string()),
                }
            })
        }));

        let mem_list = mem.clone();
        services.insert("memory.list".to_string(), Arc::new(move |params: Value| {
            let mem = mem_list.clone();
            Box::pin(async move {
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
                let all = mem.list_all().await;
                let items: Vec<Value> = all.into_iter().take(limit).map(|e| {
                    json!({"id": e.id, "content": e.content, "weight": e.weight})
                }).collect();
                Ok(Value::Array(items))
            })
        }));
    }

    // Agent spawner service
    if let Some(spawner) = spawner {
        services.insert("agent.spawn".to_string(), Arc::new(move |params: Value| {
            let spawner = spawner.clone();
            Box::pin(async move {
                let prompt = params.get("prompt").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let description = params.get("description").and_then(|v| v.as_str()).unwrap_or("sub-task").to_string();
                let max_iterations = params.get("max_iterations").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
                match spawner.spawn(&prompt, &description, max_iterations).await {
                    Ok(result) => Ok(json!({
                        "text": result.text,
                        "iterations": result.iterations,
                        "tool_calls": result.tool_calls,
                    })),
                    Err(e) => Err(e),
                }
            })
        }));
    }

    // Session history service — returns recent messages from the current session.
    // The SharedSessionHistory is populated before each agent run.
    {
        let history = session_history;
        services.insert("session.history".to_string(), Arc::new(move |params: Value| {
            let history = history.clone();
            Box::pin(async move {
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
                let history = history.read().await;
                let start = if history.len() > limit { history.len() - limit } else { 0 };
                let messages: Vec<Value> = history[start..].iter().map(|msg| {
                    json!({
                        "role": format!("{:?}", msg.role).to_lowercase(),
                        "content": msg.content,
                        "timestamp": msg.timestamp.to_rfc3339(),
                    })
                }).collect();
                Ok(json!(messages))
            })
        }));
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
        }
    }
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
    /// Get the embedding dimension for the configured embedding model
    ///
    /// Retrieves dimension from ModelInfoCache (which queries the provider API if not cached).
    /// Falls back to static lookup if the API doesn't provide dimension info.
    async fn get_embedding_dimension(&self) -> usize {
        use nanna_llm::ModelInfoCache;
        use nanna_memory::MemoryServiceConfig;

        // Create an LLM client for the embedding provider to fetch model info
        let llm_client = match self.embedding.provider.as_str() {
            "openai" => {
                let api_key = std::env::var("OPENAI_API_KEY").ok();
                api_key.map(|key| LlmClient::openai(&key))
            }
            "ollama" | _ => {
                Some(LlmClient::ollama(&self.embedding.ollama_host))
            }
        };

        let Some(client) = llm_client else {
            // No client available, use static lookup
            info!("No embedding client available, using static dimension lookup for {}", self.embedding.model);
            return MemoryServiceConfig::dimension_for_model(&self.embedding.model);
        };

        // Get model info from cache or API
        let cache = ModelInfoCache::default_location();
        let model_info = client.get_model_info(&self.embedding.model, cache.as_ref()).await;

        // Return embedding dimension from cache/API if available, otherwise fall back to static lookup
        model_info.embedding_dimension.unwrap_or_else(|| {
            debug!("No embedding dimension from cache/API for {}, using static lookup", self.embedding.model);
            MemoryServiceConfig::dimension_for_model(&self.embedding.model)
        })
    }

    /// Create a new daemon server
    pub fn new(config: DaemonConfig, embedding: EmbeddingConfig, memory_path: Option<PathBuf>, brave_api_key: Option<String>) -> Self {
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
    
    /// Set the storage backend for model stats persistence.
    pub fn set_storage(&mut self, storage: Arc<nanna_storage::Storage>) {
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
        
        // Load persisted sessions
        match self.persistence.load_sessions_with_fallback().await {
            Ok((sessions, default_id)) => {
                info!("Loaded {} persisted sessions", sessions.len());
                for session in sessions {
                    self.sessions.restore(session).await;
                }
                if let Some(id) = default_id {
                    self.sessions.set_default(&id).await;
                }
            }
            Err(e) => {
                warn!("Failed to load persisted sessions: {}", e);
            }
        }
        
        // Create default session if none exist
        if self.sessions.count().await == 0 {
            let default_session = self.sessions.create(Some("Main".to_string())).await;
            info!("Created default session: {}", default_session.id);
        }
        
        // Initialize services
        let (tools, memory, agent, router, tools_dir) = self.init_services().await?;

        // Create control plane with all services (including router for consolidation)
        let mut control = ControlPlane::with_all_services(
            self.sessions.clone(),
            agent,
            memory.clone(),
            Some(tools),
            Some(router),
        )
        .with_tools_dir(tools_dir)
        .with_event_tx(self.ipc.event_sender());
        if let Some(ref buf) = self.log_buffer {
            control = control.with_log_buffer(buf.clone());
        }
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
                                warn!("Failed to load workspace context for {}: {}", record.path, e);
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

        let control = Arc::new(control);
        
        // Take the request receiver from IPC server
        let mut request_rx = self.ipc.take_request_receiver().await
            .ok_or_else(|| crate::DaemonError::Ipc("Request receiver already taken".to_string()))?;
        
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
            let state = HealthState::new(
                memory.is_some(),
                true, // agent is available
            );
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
            
            let health_server = HealthServer::new(
                HealthState::new(memory.is_some(), true),
                &self.config.ipc.host,
                self.config.health_port,
            );
            health_server.spawn();
            info!("Health server listening on http://{}:{}", self.config.ipc.host, self.config.health_port);
            
            Some(health_state)
        } else {
            None
        };
        
        // Spawn webhook HTTP server if enabled
        if self.config.enable_webhook_server {
            let mut webhook_config = self.config.webhook.clone();
            webhook_config.host = self.config.ipc.host.clone();
            webhook_config.port = self.config.webhook_port;
            
            let (webhook_server, mut webhook_rx) = WebhookServer::new(webhook_config);
            
            // Spawn the webhook server
            tokio::spawn(async move {
                if let Err(e) = webhook_server.run().await {
                    error!("Webhook server error: {}", e);
                }
            });
            
            // Spawn webhook event processor
            let _control_for_webhooks = control.clone();
            let _sessions_for_webhooks = self.sessions.clone();
            tokio::spawn(async move {
                while let Some(event) = webhook_rx.recv().await {
                    debug!("Webhook event from {}: {:?}", event.source, event.message);
                    
                    // Route webhook message to appropriate session
                    if let Some(ref msg) = event.message {
                        // Create or get session for this chat
                        let session_key = format!("{}:{}", event.source, msg.chat_id);
                        
                        // For now, just log the message
                        // TODO: Route to control plane for agent processing
                        info!(
                            "Webhook message from {} in {}: {}",
                            msg.sender_name.as_deref().unwrap_or(&msg.sender_id),
                            session_key,
                            msg.content.chars().take(100).collect::<String>()
                        );
                    }
                }
            });
            
            info!("Webhook server listening on http://{}:{}", self.config.ipc.host, self.config.webhook_port);
        }
        
        // Spawn session auto-save task
        let persistence = self.persistence.clone();
        let sessions_for_save = self.sessions.clone();
        let save_interval = Duration::from_secs(self.config.auto_save_interval_secs);
        let save_shutdown = self.shutdown_tx.subscribe();

        let save_handle = tokio::spawn(async move {
            let sessions_map = sessions_for_save.sessions_map();
            let default_session = sessions_for_save.default_session_id();
            persistence.auto_save_loop(sessions_map, default_session, save_interval, save_shutdown).await;
        });

        // Memory is now persisted via SQLite write-through on every mutation.
        // The old periodic JSON auto-save timer is removed.

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

        info!("Daemon ready. IPC server listening on ws://{}", self.ipc.address());
        
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
        
        // Wait for auto-save tasks to complete final saves
        let _ = tokio::time::timeout(Duration::from_secs(5), save_handle).await;
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
    async fn init_services(&self) -> Result<(
        Arc<ToolRegistry>,
        Option<Arc<MemoryService>>,
        Arc<AgentService>,
        Arc<LlmRouter>,
        Option<PathBuf>,  // tools_dir
    ), crate::DaemonError> {
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
        info!("LLM router initialized with {} providers: {:?}", available.len(), available);
        
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
                info!("Bootstrapped {} default skills into {:?}", bootstrapped, resolved);
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
            use crate::embedding_router::{EmbeddingRouter, EmbeddingProviderInfo};

            // Build primary embedding client
            let primary_client = match self.embedding.provider.as_str() {
                "openai" => {
                    let api_key = std::env::var("OPENAI_API_KEY").ok();
                    api_key.map(|key| {
                        info!("Primary embeddings: OpenAI {}", self.embedding.model);
                        (
                            EmbeddingProviderInfo { name: "openai".into(), model: self.embedding.model.clone() },
                            Arc::new(nanna_llm::EmbeddingClient::openai(&key).with_model(&self.embedding.model)),
                        )
                    })
                }
                "openrouter" => {
                    let api_key = self.config.llm.openrouter_api_key.clone()
                        .or_else(|| std::env::var("OPENROUTER_API_KEY").ok());
                    api_key.map(|key| {
                        info!("Primary embeddings: OpenRouter {}", self.embedding.model);
                        (
                            EmbeddingProviderInfo { name: "openrouter".into(), model: self.embedding.model.clone() },
                            Arc::new(nanna_llm::EmbeddingClient::openai(&key)
                                .with_model(&self.embedding.model)
                                .with_base_url("https://openrouter.ai/api")),
                        )
                    })
                }
                "ollama" | _ => {
                    info!("Primary embeddings: Ollama {} at {}", self.embedding.model, self.embedding.ollama_host);
                    Some((
                        EmbeddingProviderInfo { name: "ollama".into(), model: self.embedding.model.clone() },
                        Arc::new(nanna_llm::EmbeddingClient::ollama(&self.embedding.ollama_host)
                            .with_model(&self.embedding.model)),
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
                                EmbeddingProviderInfo { name: "openai".into(), model: fallback_model.clone() },
                                Arc::new(nanna_llm::EmbeddingClient::openai(&api_key).with_model(&fallback_model)),
                            );
                        }
                    }
                    if primary_info.name != "openrouter" {
                        if let Some(api_key) = self.config.llm.openrouter_api_key.clone()
                            .or_else(|| std::env::var("OPENROUTER_API_KEY").ok()) {
                            let fallback_model = "openai/text-embedding-3-small".to_string();
                            info!("Adding OpenRouter embedding fallback: {}", fallback_model);
                            embed_router = embed_router.with_fallback(
                                EmbeddingProviderInfo { name: "openrouter".into(), model: fallback_model.clone() },
                                Arc::new(nanna_llm::EmbeddingClient::openai(&api_key)
                                    .with_model(&fallback_model)
                                    .with_base_url("https://openrouter.ai/api")),
                            );
                        }
                    }
                    if primary_info.name != "ollama" {
                        let fallback_model = "nomic-embed-text".to_string();
                        info!("Adding Ollama embedding fallback: {} at {}", fallback_model, self.embedding.ollama_host);
                        embed_router = embed_router.with_fallback(
                            EmbeddingProviderInfo { name: "ollama".into(), model: fallback_model.clone() },
                            Arc::new(nanna_llm::EmbeddingClient::ollama(&self.embedding.ollama_host)
                                .with_model(&fallback_model)),
                        );
                    }

                    info!("Embedding router: {} providers configured", embed_router.provider_count());
                    let embed_router = Arc::new(embed_router);

                    // Create embedding function that routes through the EmbeddingRouter.
                    // Tracks the router generation to detect provider switches.
                    let router_for_fn = embed_router.clone();
                    let last_generation = Arc::new(std::sync::atomic::AtomicU64::new(embed_router.generation()));
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
                                let old_gen = gen_tracker.swap(new_gen, std::sync::atomic::Ordering::Relaxed);
                                if new_gen != old_gen {
                                    // Provider switched — trigger re-embed in background
                                    if let Some(mem) = mem_cell.get() {
                                        let mem = mem.clone();
                                        tokio::spawn(async move {
                                            tracing::info!("Embedding provider changed (gen {} → {}), probing dimension and re-embedding if needed...", old_gen, new_gen);
                                            match mem.probe_and_align_dimension().await {
                                                Ok(dim) => tracing::info!("Dimension probe complete: {} dims", dim),
                                                Err(e) => tracing::warn!("Dimension probe failed: {}", e),
                                            }
                                        });
                                    }
                                }
                            }

                            Ok(embedding)
                        })
                    });

                    // Try to get embedding dimension from model info cache or API
                    let dimension = self.get_embedding_dimension().await;
                    let config = nanna_memory::MemoryServiceConfig {
                        dimension,
                        ..Default::default()
                    };
                    info!("Memory service using dimension {} for model {}", dimension, self.embedding.model);

                    // Wire up SQLite persistence if storage is available.
                    // The persistence adapter is constructed here and attached to the
                    // MemoryService so all writes are automatically mirrored to SQLite.
                    let memory_service = if let Some(ref storage) = self.storage {
                        let repo = storage.memories();
                        let db = Arc::new(SqliteMemoryPersistence::new(repo));
                        nanna_memory::MemoryService::new(config)
                            .with_embed_fn(embed_fn)
                            .with_persistence(db)
                    } else {
                        warn!("No storage backend available — memory will NOT be persisted to SQLite");
                        nanna_memory::MemoryService::new(config)
                            .with_embed_fn(embed_fn)
                    };

                    // One-time migration: if memories.json exists and SQLite is empty,
                    // load from JSON into in-memory cache then save each entry to SQLite.
                    let json_path = self.memory_path.as_ref();
                    let should_migrate = if let (Some(path), Some(storage)) = (json_path, &self.storage) {
                        if path.exists() {
                            match storage.memories().count().await {
                                Ok(0) => true,
                                Ok(n) => {
                                    info!("SQLite already has {} memories — skipping JSON migration", n);
                                    false
                                }
                                Err(e) => {
                                    warn!("Could not check SQLite memory count: {}", e);
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
                        info!("Migrating memories from {:?} to SQLite (one-time migration)", path);
                        match memory_service.load(path).await {
                            Ok(()) => {
                                let count = memory_service.count().await;
                                info!("Loaded {} memories from JSON, flushing to SQLite...", count);
                                // Flush all entries to SQLite
                                match memory_service.flush_to_db().await {
                                    Ok(n) => info!("Flushed {} memories to SQLite", n),
                                    Err(e) => warn!("Failed to flush memories to SQLite: {}", e),
                                }
                                // Rename the JSON file so we don't re-migrate next time
                                let migrated_path = path.with_extension("json.migrated");
                                if let Err(e) = tokio::fs::rename(path, &migrated_path).await {
                                    warn!("Could not rename migrated JSON file: {}", e);
                                } else {
                                    info!("Renamed {:?} → {:?} (migration complete)", path, migrated_path);
                                }
                            }
                            Err(e) => {
                                warn!("JSON migration failed: {}. Will attempt to load from SQLite.", e);
                            }
                        }
                    }

                    // Load from SQLite into the in-memory cache (normal startup path).
                    // Skipped if we just migrated (the entries are already in-memory from the JSON load above).
                    if !should_migrate {
                        match memory_service.load_from_db().await {
                            Ok(count) => {
                                info!("Loaded {} memories from SQLite", count);
                            }
                            Err(nanna_memory::MemoryError::Persistence(ref e))
                                if e.contains("No persistence backend") =>
                            {
                                // No storage configured — silently skip
                            }
                            Err(e) => {
                                warn!("Failed to load memories from SQLite: {}", e);
                            }
                        }
                    }

                    // Probe the actual embedding dimension from the model.
                    // If the model returns a different dimension than stored entries,
                    // re-embeds all mismatched entries with the new model.
                    match memory_service.probe_and_align_dimension().await {
                        Ok(actual_dim) => {
                            if actual_dim != dimension {
                                info!(
                                    "Embedding dimension corrected: {} → {} for model {}",
                                    dimension, actual_dim, self.embedding.model
                                );
                            }
                        }
                        Err(e) => {
                            warn!(
                                "Could not probe embedding dimension (model may be loading): {}. \
                                 Using static dimension {}.",
                                e, dimension
                            );
                        }
                    }
                    
                    info!("Memory service initialized with SQLite persistence and embedding router");
                    let memory_arc = Arc::new(memory_service);

                    // Wire the memory service into the embed_fn's OnceCell
                    // so provider-switch re-embedding can find it
                    let _ = memory_for_reembed.set(memory_arc.clone());

                    Some(memory_arc)
                }
                None => {
                    warn!("Memory service disabled: no embedding provider available");
                    warn!("To enable memory: set OPENAI_API_KEY or use Ollama with embeddings model");
                    None
                }
            }
        } else {
            info!("Memory service disabled in config");
            None
        };

        // Shared session history for the recall_messages tool service
        let session_history: SharedSessionHistory = Arc::new(tokio::sync::RwLock::new(Vec::new()));

        // Build script services and load all tools from disk
        {
            let spawner_arc: Option<Arc<dyn AgentSpawner + Send + Sync>> =
                if !router.available_providers().is_empty() {
                    Some(Arc::new(AgentSpawnerImpl {
                        router: Arc::new(router.clone()),
                        tools: tools.clone(),
                        agent_config: nanna_agent::AgentConfig {
                            model: self.config.agent.model.clone(),
                            max_tokens: self.config.agent.max_tokens,
                            temperature: self.config.agent.temperature,
                            max_iterations: None, // Unlimited — model stops when done
                            thinking_mode: self.config.agent.thinking_mode,
                            summarization_priority: self.config.agent.summarization_priority.clone(),
                            summarization_ollama_url: self.config.agent.summarization_ollama_url.clone(),
                            sub_agent_model: self.config.agent.sub_agent_model.clone(),
                            ..Default::default()
                        },
                        system_prompt: nanna_agent::prompts::DEFAULT_SYSTEM_PROMPT.to_string(),
                        workspace_root: None,
                        workspace_context: None,
                        stats: None, // TODO: wire shared stats tracker from daemon state
                    }))
                } else {
                    None
                };

            let services = build_script_services(&memory, spawner_arc, session_history.clone());

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
                let wrapper = nanna_tools::skills::ScriptedToolWrapper::from_source("discover_tools", &source)
                    .expect("discover_tools skill must parse")
                    .with_registry(Arc::downgrade(&tools));
                tools.register(wrapper).await;
                info!("Registered discover_tools skill from {:?}", dir);
            } else {
                warn!("discover_tools not found in tools directory");
            }
        }

        // Create agent service with multi-provider router
        let event_tx = self.ipc.event_sender();
        let router = Arc::new(router);
        let agent = Arc::new(AgentService::new(
            self.config.agent.clone(),
            router.clone(),
            tools.clone(),
            memory.clone(),
            event_tx,
        ).with_session_history(session_history));

        info!("Agent service initialized");

        Ok((tools, memory, agent, router, tools_dir))
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
                warn!("Failed to load Nanna config: {}, using defaults with env overrides", e);
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

        // Log configured providers
        let mut providers = Vec::new();
        if builder.config.llm.anthropic_api_key.is_some() || builder.config.llm.anthropic_oauth_token.is_some() {
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

        info!("Daemon config loaded: model={}, embedding={}:{}, providers=[{}], brave_key={}",
              builder.config.agent.model,
              builder.embedding.provider,
              builder.embedding.model,
              providers.join(", "),
              if builder.brave_api_key.is_some() { "set" } else { "none" });
        
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
        let mut server = DaemonServer::new(self.config, self.embedding, self.memory_path, self.brave_api_key);
        server.log_buffer = self.log_buffer;

        // Initialize SQLite storage for model stats persistence
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
                warn!("Failed to initialize storage: {}. Model stats will not persist.", e);
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
