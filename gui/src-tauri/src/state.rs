//! Application state and shared GUI data types.
//!
//! The GUI is a pure daemon client: the daemon owns storage, memory, the tool
//! registry, the agent loop, and the scheduler. `AppState` therefore holds only
//! what a thin client needs — a config cache, the backend (daemon) handle, a
//! workspace-registry cache, UI-model state, and its own log buffer.

#[allow(clippy::wildcard_imports)]
use crate::*;

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

/// Application state shared across commands.
///
/// Every persistent subsystem (sessions, memory, tools, cron, agent loop) lives
/// in the daemon and is reached through [`Backend`]. The fields here are a thin
/// client's working set only.
pub struct AppState {
    /// Local config cache. Settings commands mutate this and persist it to
    /// `config.toml`; the daemon reads the same file (and config actions over
    /// IPC keep it live).
    pub(crate) config: Config,
    /// What to do when the window is closed
    pub(crate) close_mode: Arc<RwLock<CloseMode>>,
    /// Currently active model (populated from the daemon's `model_switch`
    /// events — a UI-badge cache, not authoritative state).
    pub(crate) active_model: Arc<RwLock<String>>,
    /// Models the UI believes are on cooldown (client-side badge cache).
    pub(crate) rate_limited_models: Arc<RwLock<HashMap<String, i64>>>,
    /// Workspace-registry cache, hydrated from the daemon at startup. The daemon
    /// owns workspace persistence; this backs local reads and the workspace-file
    /// editing commands.
    pub(crate) workspaces: Arc<RwLock<WorkspaceRegistry>>,
    /// Backend abstraction (daemon client + sidecar lifecycle)
    pub(crate) backend: Arc<Backend>,
    /// Recent log lines emitted by *this* process, captured by the tracing layer
    /// installed in `run()`. The Logs page merges these with the daemon's own.
    pub(crate) log_buffer: LogBuffer,
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
    /// Chronological run journal (thinking/text/tool/fault items). Kept as
    /// raw JSON — the daemon owns the schema; the frontend renders it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeline: Option<serde_json::Value>,
    /// Run benchmark totals {input_tokens, output_tokens, duration_ms, model}.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<serde_json::Value>,
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
