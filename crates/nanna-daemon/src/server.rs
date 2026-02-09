//! Daemon Server - Main daemon orchestrator
//!
//! Combines IPC server, control plane, sessions, persistence, and all subsystems.

use crate::agent_service::{AgentService, AgentServiceConfig};
use crate::control::ControlPlane;
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
    ToolRegistry, EchoTool, ExecTool, ReadFileTool, WriteFileTool, ListDirTool,
    WebSearchTool, WebFetchTool, RememberTool, RecallTool, ReflectTool,
    MemoryServiceStorage, MemoryServiceAdapter, MemoryResult, StorageHandle,
    TaskTool, AgentSpawner, SpawnResult,
    CodeOutlineTool, CodeSearchTool, ProjectStructureTool,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;

/// Adapter to make MemoryService implement MemoryServiceAdapter trait
struct MemoryServiceWrapper(Arc<MemoryService>);

#[async_trait]
impl MemoryServiceAdapter for MemoryServiceWrapper {
    async fn remember(&self, content: &str, metadata: HashMap<String, String>, importance: f32) -> Result<String, String> {
        self.0.remember_with_importance(content, metadata, importance)
            .await
            .map(|(id, _)| id)
            .map_err(|e| e.to_string())
    }

    async fn recall(&self, query: &str, limit: usize) -> Result<Vec<MemoryResult>, String> {
        let results = self.0.recall(query).await.map_err(|e| e.to_string())?;
        Ok(results.into_iter()
            .take(limit)
            .map(|r| MemoryResult {
                id: r.id,
                content: r.content,
                score: Some(r.score),
            })
            .collect())
    }

    async fn forget(&self, id: &str) -> Result<(), String> {
        self.0.forget(id).await.map_err(|e| e.to_string())
    }

    async fn list(&self, limit: usize) -> Result<Vec<MemoryResult>, String> {
        let all = self.0.list_all().await;
        Ok(all.into_iter()
            .take(limit)
            .map(|e| MemoryResult {
                id: e.id,
                content: e.content,
                score: Some(e.weight),
            })
            .collect())
    }
}
/// Concrete implementation of AgentSpawner that lives in the daemon
/// where it can create Agent instances with isolated context.
struct AgentSpawnerImpl {
    llm: Arc<LlmClient>,
    tools: Arc<ToolRegistry>,
    agent_config: nanna_agent::AgentConfig,
    system_prompt: String,
    workspace_root: Option<PathBuf>,
    workspace_context: Option<String>,
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

        let agent = Agent::new(config, self.llm.clone(), self.tools.clone())
            .with_context(context);

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
        }
    }
}

/// The main daemon server
#[allow(dead_code)]
pub struct DaemonServer {
    config: DaemonConfig,
    embedding: EmbeddingConfig,
    memory_path: Option<PathBuf>,
    brave_api_key: Option<String>,
    sessions: Arc<SessionManager>,
    control: Arc<ControlPlane>,
    ipc: Arc<IpcServer>,
    persistence: Arc<PersistenceManager>,
    shutdown_tx: broadcast::Sender<()>,
    /// PID file (prevents multiple instances)
    pid_file: Option<PidFile>,
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
            brave_api_key,
            sessions,
            control,
            ipc,
            persistence,
            shutdown_tx,
            pid_file,
        }
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
        let (tools, memory, agent, router) = self.init_services().await?;

        // Create control plane with all services (including router for consolidation)
        let control = Arc::new(ControlPlane::with_all_services(
            self.sessions.clone(),
            agent,
            memory.clone(),
            Some(tools),
            Some(router), // Pass router for consolidation
        ));
        
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

        // Spawn memory auto-save task (parallels session auto-save)
        let mem_save_handle = if let (Some(mem_service), Some(mem_path)) = (&memory, &self.memory_path) {
            let mem = mem_service.clone();
            let path = mem_path.clone();
            let save_interval = Duration::from_secs(self.config.auto_save_interval_secs);
            let mut mem_shutdown = self.shutdown_tx.subscribe();

            Some(tokio::spawn(async move {
                let mut interval = tokio::time::interval(save_interval);
                loop {
                    tokio::select! {
                        _ = interval.tick() => {
                            if let Err(e) = mem.save(&path).await {
                                error!("Memory auto-save failed: {}", e);
                            }
                        }
                        _ = mem_shutdown.recv() => {
                            // Final save on shutdown
                            if let Err(e) = mem.save(&path).await {
                                error!("Memory final save failed: {}", e);
                            } else {
                                info!("Memory final save completed");
                            }
                            break;
                        }
                    }
                }
            }))
        } else {
            None
        };
        
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
        if let Some(handle) = mem_save_handle {
            let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
        }
        
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

        // Always add Ollama (local, no auth needed)
        info!("Adding Ollama provider at {}", self.config.llm.ollama_host);
        router = router.with_ollama(&self.config.llm.ollama_host);

        let available = router.available_providers();
        if available.is_empty() {
            return Err(crate::DaemonError::Config(
                "No LLM providers configured. Please set up at least one provider (Anthropic, OpenAI, OpenRouter, or Ollama).".to_string()
            ));
        }
        info!("LLM router initialized with {} providers: {:?}", available.len(), available);
        
        // Create tool registry with built-in tools
        let tools = Arc::new(ToolRegistry::new());
        
        // Register built-in tools
        tools.register(EchoTool).await;
        tools.register(ExecTool::new()).await;
        tools.register(ReadFileTool::new()).await;
        tools.register(WriteFileTool::new()).await;
        tools.register(ListDirTool::new()).await;
        tools.register(WebFetchTool::new()).await;
        tools.register(CodeOutlineTool::new()).await;
        tools.register(CodeSearchTool::new()).await;
        tools.register(ProjectStructureTool::new()).await;
        
        // Register web search if API key available (from config or env)
        let brave_key = self.brave_api_key.clone()
            .or_else(|| std::env::var("BRAVE_API_KEY").ok());
        if let Some(api_key) = brave_key {
            info!("Registering web_search tool with Brave API");
            tools.register(WebSearchTool::new().with_api_key(&api_key)).await;
        } else {
            info!("Web search disabled: no Brave API key configured");
        }
        
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
        
        let tool_count = tools.definitions().await.len();
        info!("Tool registry initialized with {} tools (including aliases)", tool_count);
        
        // Initialize memory service with embeddings if enabled
        let memory: Option<Arc<MemoryService>> = if self.config.enable_memory {
            // Get embedding client based on embedding config (not LLM provider)
            let embed_client = match self.embedding.provider.as_str() {
                "openai" => {
                    let api_key = std::env::var("OPENAI_API_KEY").ok();
                    api_key.map(|key| {
                        info!("Using OpenAI embeddings: {}", self.embedding.model);
                        Arc::new(nanna_llm::EmbeddingClient::openai(&key))
                    })
                }
                "ollama" => {
                    info!("Using Ollama embeddings: {} at {}", self.embedding.model, self.embedding.ollama_host);
                    Some(Arc::new(nanna_llm::EmbeddingClient::ollama(&self.embedding.ollama_host)
                        .with_model(&self.embedding.model)))
                }
                provider => {
                    warn!("Unknown embedding provider: {}, trying Ollama fallback", provider);
                    Some(Arc::new(nanna_llm::EmbeddingClient::ollama(&self.embedding.ollama_host)
                        .with_model(&self.embedding.model)))
                }
            };
            
            match embed_client {
                Some(embed) => {
                    // Create embedding function that wraps the client
                    let embed_fn: nanna_memory::EmbedFn = Arc::new(move |text: &str| {
                        let client = embed.clone();
                        let text = text.to_string();
                        Box::pin(async move {
                            client.embed_one(&text).await.map_err(|e| e.to_string())
                        })
                    });

                    // Try to get embedding dimension from model info cache or API
                    let dimension = self.get_embedding_dimension().await;
                    let config = nanna_memory::MemoryServiceConfig {
                        dimension,
                        ..Default::default()
                    };
                    info!("Memory service using dimension {} for model {}", dimension, self.embedding.model);
                    let memory_service = nanna_memory::MemoryService::new(config)
                        .with_embed_fn(embed_fn);
                    
                    // Load existing memories from file if available
                    if let Some(ref path) = self.memory_path {
                        if path.exists() {
                            info!("Loading memories from {:?}", path);
                            if let Err(e) = memory_service.load(path).await {
                                warn!("Failed to load memories: {}", e);
                            } else {
                                let count = memory_service.count().await;
                                info!("Loaded {} memories from disk", count);
                            }
                        } else {
                            info!("No existing memory file at {:?}", path);
                        }
                    }
                    
                    info!("Memory service initialized with embeddings");
                    Some(Arc::new(memory_service))
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

        // Register memory tools if memory service is available
        if let Some(ref mem_service) = memory {
            let wrapper: Arc<dyn MemoryServiceAdapter + Send + Sync> = Arc::new(MemoryServiceWrapper(mem_service.clone()));
            let storage: StorageHandle = Arc::new(MemoryServiceStorage::new(wrapper));

            tools.register(RememberTool::new(storage.clone())).await;
            tools.register(RecallTool::new(storage.clone())).await;
            tools.register(ReflectTool::new(storage)).await;

            info!("Registered memory tools (remember, recall, reflect)");
        } else {
            info!("Memory tools not registered: memory service not available");
        }

        // Register task delegation tool (sub-agent spawner)
        // Get the primary LLM client for the sub-agent
        if let Some(primary_client) = router.primary_client() {
            let spawner = AgentSpawnerImpl {
                llm: primary_client,
                tools: tools.clone(),
                agent_config: nanna_agent::AgentConfig {
                    model: self.config.agent.model.clone(),
                    max_tokens: self.config.agent.max_tokens,
                    temperature: self.config.agent.temperature,
                    max_iterations: Some(10), // default for sub-agents, overridden per-spawn
                    thinking_mode: self.config.agent.thinking_mode,
                    summarization_priority: self.config.agent.summarization_priority.clone(),
                    summarization_ollama_url: self.config.agent.summarization_ollama_url.clone(),
                    ..Default::default()
                },
                system_prompt: nanna_agent::prompts::DEFAULT_SYSTEM_PROMPT.to_string(),
                workspace_root: None, // Will be set per-session in the future
                workspace_context: None,
            };
            tools.register(TaskTool::new(Arc::new(spawner))).await;
            info!("Registered task delegation tool");
        } else {
            warn!("No primary LLM client available, task tool not registered");
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
        ));

        info!("Agent service initialized");

        Ok((tools, memory, agent, router))
    }
}

/// Embedding configuration for the daemon
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    /// Provider (ollama, openai)
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
}

impl DaemonBuilder {
    pub fn new() -> Self {
        Self {
            config: DaemonConfig::default(),
            embedding: EmbeddingConfig::default(),
            memory_path: None,
            brave_api_key: None,
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

        // Set Brave API key for web search
        builder.brave_api_key = config.tools.brave_api_key.clone();
        
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
    
    pub fn build(self) -> DaemonServer {
        DaemonServer::new(self.config, self.embedding, self.memory_path, self.brave_api_key)
    }
}

impl Default for DaemonBuilder {
    fn default() -> Self {
        Self::new()
    }
}
