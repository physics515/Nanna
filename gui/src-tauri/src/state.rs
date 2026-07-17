//! Application state and shared GUI data types.

#[allow(clippy::wildcard_imports)]
use crate::*;

// =============================================================================
// Memory Service Adapter for Tool System
// =============================================================================

/// Adapter to make MemoryService work with the MemoryStorage trait used by tools.
///
/// Holds a live handle to the workspace registry so every remember/recall scopes
/// to whatever workspace is active *at call time* (not at tool setup).
pub struct MemoryServiceAdapter {
    service: Arc<MemoryService>,
    workspaces: Arc<RwLock<WorkspaceRegistry>>,
}

impl MemoryServiceAdapter {
    pub(crate) fn new(service: Arc<MemoryService>, workspaces: Arc<RwLock<WorkspaceRegistry>>) -> Self {
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
    pub(crate) async fn active_workspace_id(&self) -> Option<String> {
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
    pub(crate) storage: Option<Arc<Storage>>,
    pub(crate) llm: Arc<LlmClient>,
    pub(crate) tools: Arc<ToolRegistry>,
    pub(crate) config: Config,
    /// What to do when window is closed
    pub(crate) close_mode: Arc<RwLock<CloseMode>>,
    /// FSRS-6 cognitive memory service
    pub(crate) memory: Arc<MemoryService>,
    /// Path to persist memories (JSON file)
    pub(crate) memory_path: std::path::PathBuf,
    /// Background task scheduler (heartbeats, consolidation)
    pub(crate) scheduler: Arc<RwLock<Scheduler>>,
    /// Last consolidation timestamp
    pub(crate) last_consolidation: Arc<RwLock<Option<i64>>>,
    /// Runtime settings for memory & scheduling (on by default)
    pub(crate) dreaming_enabled: Arc<RwLock<bool>>,
    pub(crate) scheduler_enabled: Arc<RwLock<bool>>,
    pub(crate) heartbeat_enabled: Arc<RwLock<bool>>,
    pub(crate) heartbeat_interval_seconds: Arc<RwLock<u64>>,
    /// Embedding configuration (separate from chat provider)
    pub(crate) embedding_provider: Arc<RwLock<String>>,
    pub(crate) embedding_model: Arc<RwLock<String>>,
    pub(crate) embedding_enabled: Arc<RwLock<bool>>,
    /// Ollama server URL (default: http://localhost:11434)
    pub(crate) ollama_host: Arc<RwLock<String>>,
    /// Model for memory extraction (empty = use chat model)
    pub(crate) extraction_model: Arc<RwLock<String>>,
    /// Currently active model (the one that will be used for the next request)
    pub(crate) active_model: Arc<RwLock<String>>,
    /// Models currently on cooldown due to rate limits (model_id -> cooldown_until timestamp)
    pub(crate) rate_limited_models: Arc<RwLock<HashMap<String, i64>>>,
    /// Workspace registry for multi-workspace support
    pub(crate) workspaces: Arc<RwLock<WorkspaceRegistry>>,
    /// User tool authoring manager
    pub(crate) user_tools: Arc<tool_authoring::UserToolManager>,
    /// Backend abstraction (daemon or embedded mode)
    pub(crate) backend: Arc<Backend>,
    /// In-process agent service for embedded mode (the daemon's `AgentService`
    /// running inside the GUI). `None` in daemon mode — the daemon runs its own.
    pub(crate) agent_service: Option<Arc<nanna_daemon::agent_service::AgentService>>,
    /// Recent log lines emitted by *this* process, captured by the tracing layer
    /// installed in `run()`. Populated in both modes: even when attached to a
    /// daemon the GUI still emits its own lines, and the Logs page shows both.
    pub(crate) log_buffer: LogBuffer,
}

impl AppState {
    /// Local storage, or an error suitable for returning from a tauri command.
    /// Only embedded-mode code paths may rely on this succeeding.
    pub(crate) fn storage(&self) -> Result<&Arc<Storage>, String> {
        self.storage.as_ref().ok_or_else(|| {
            "Local storage is not open (daemon mode — the daemon owns nanna.db)".to_string()
        })
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

/// Application config for frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub theme: String,
    pub model: String,
    pub api_key_set: bool,
    pub available_models: Vec<String>,
    pub available_tools: Vec<String>,
}

