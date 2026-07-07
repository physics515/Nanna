//! IPC Protocol definitions
//!
//! Defines the request/response/event types for communication between
//! daemon and channel clients (GUI, CLI, API, etc.)

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Unique request ID for request/response correlation
pub type RequestId = String;

/// Client-to-daemon request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: RequestId,
    pub action: Action,
}

/// All possible actions a client can request
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    // =========================================================================
    // Chat / Agent
    // =========================================================================
    /// Send a message to the agent
    Chat(ChatAction),
    
    // =========================================================================
    // Session Management
    // =========================================================================
    Session(SessionAction),
    
    // =========================================================================
    // Memory
    // =========================================================================
    Memory(MemoryAction),
    
    // =========================================================================
    // Configuration
    // =========================================================================
    Config(ConfigAction),
    
    // =========================================================================
    // Tools
    // =========================================================================
    Tool(ToolAction),
    
    // =========================================================================
    // Scheduler / Cron
    // =========================================================================
    Scheduler(SchedulerAction),
    
    // =========================================================================
    // Channels
    // =========================================================================
    Channel(ChannelAction),
    
    // =========================================================================
    // System
    // =========================================================================
    System(SystemAction),
    
    // =========================================================================
    // Workspaces
    // =========================================================================
    Workspace(WorkspaceAction),
    
    // =========================================================================
    // Subscriptions
    // =========================================================================
    Subscribe(SubscribeAction),
    Unsubscribe(UnsubscribeAction),
}

// =============================================================================
// Chat Actions
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ChatAction {
    /// Send a message and get a response
    Send {
        session_id: String,
        content: String,
        #[serde(default)]
        attachments: Vec<Attachment>,
    },
    /// Cancel an in-progress response
    Cancel { session_id: String },
    /// Regenerate the last response
    Regenerate { session_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub filename: String,
    pub content_type: String,
    /// Base64-encoded data or URL
    pub data: String,
}

// =============================================================================
// Session Actions
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SessionAction {
    /// List all sessions (optionally filtered by workspace)
    List,
    /// List sessions for a specific workspace (None = global only)
    ListByWorkspace { workspace_id: Option<String> },
    /// Get session details
    Get { id: String },
    /// Create a new session
    Create { name: Option<String> },
    /// Create a new session in a specific workspace
    CreateInWorkspace { name: Option<String>, workspace_id: Option<String> },
    /// Rename a session
    Rename { id: String, name: String },
    /// Delete a session
    Delete { id: String },
    /// Delete all sessions
    DeleteAll,
    /// Clear session history
    Clear { id: String },
    /// Get session history
    History { id: String, limit: Option<usize>, before: Option<String> },
    /// Switch active session (for this client)
    Switch { id: String },
    /// Fork a session (create copy)
    Fork { id: String, name: Option<String> },
    /// Get current execution state (in-flight streaming text, active tools)
    GetRunState { id: String },
    /// Set/change the workspace for a session (None = make global)
    SetWorkspace { id: String, workspace_id: Option<String> },

    // --- Sub-Agent Sessions (#72) ---
    
    /// Spawn a sub-agent session
    SpawnSubSession {
        /// Task description / initial prompt for the sub-agent
        task: String,
        /// Optional human-readable label for easy reference
        label: Option<String>,
        /// Parent session ID (if called from within a session)
        parent_id: Option<String>,
        /// Model override (uses default if None)
        model: Option<String>,
        /// Maximum iterations before auto-stop (None = unlimited)
        max_iterations: Option<usize>,
        /// Timeout in seconds (None = no timeout)
        timeout_secs: Option<u64>,
        /// System prompt override
        system_prompt: Option<String>,
    },
    /// Send a message to a sub-session (by label or ID)
    SendToSubSession {
        /// Target session: label or session ID
        target: String,
        /// Message to inject
        message: String,
    },
    /// List sub-sessions (optionally filtered by parent)
    ListSubSessions {
        /// Only show children of this session (None = show all sub-sessions)
        parent_id: Option<String>,
    },
    /// Kill / abort a sub-session
    KillSubSession {
        /// Session ID or label to kill
        target: String,
    },
    /// Get detailed status of a sub-session
    GetSubSessionStatus {
        /// Session ID or label
        target: String,
    },
}

// =============================================================================
// Memory Actions
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum MemoryAction {
    /// List all memories
    List { scope: Option<String> },
    /// Search memories (optionally scoped to workspace)
    Search { query: String, limit: Option<usize>, scope: Option<String> },
    /// Get a specific memory
    Get { id: String },
    /// Create a memory
    Create { content: String, tags: Option<Vec<String>>, importance: Option<u8> },
    /// Update a memory
    Update { id: String, content: Option<String>, tags: Option<Vec<String>> },
    /// Delete a memory
    Delete { id: String },
    /// Clear all memories
    Clear,
    /// Get memory stats
    Stats,
    /// Trigger consolidation
    Consolidate,
}

// =============================================================================
// Config Actions
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ConfigAction {
    /// Get full config or specific path
    Get { path: Option<String> },
    /// Set a config value
    Set { path: String, value: Value },
    /// Reset to defaults
    Reset { path: Option<String> },
    /// Reload config from disk
    Reload,
    /// Export config
    Export,
    /// Import config
    Import { config: Value },
}

// =============================================================================
// Tool Actions
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ToolAction {
    /// List all tools
    List,
    /// Get tool definition
    Get { name: String },
    /// Enable a tool
    Enable { name: String },
    /// Disable a tool
    Disable { name: String },
    /// Execute a tool directly
    Execute { name: String, input: Value },
    /// Create a user tool
    Create { name: String, description: String, code: String, needs_shell: Option<bool> },
    /// Update a user tool
    Update { name: String, description: Option<String>, code: Option<String>, needs_shell: Option<bool> },
    /// Delete a user tool
    Delete { name: String },
    /// Test a user tool (without saving)
    Test { code: String, input: Value },
    /// List only user-created tools
    ListUser,
    /// Get source code for a tool (reads from tools directory)
    GetSource { name: String },
}

// =============================================================================
// Scheduler Actions
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SchedulerAction {
    /// List scheduled jobs
    List,
    /// Get job details
    Get { id: String },
    /// Add a cron job
    Add { schedule: String, task: String, name: Option<String> },
    /// Update a job
    Update { id: String, schedule: Option<String>, task: Option<String>, enabled: Option<bool> },
    /// Remove a job
    Remove { id: String },
    /// Run a job immediately
    RunNow { id: String },
    /// Get job history
    History { id: String, limit: Option<usize> },
}

// =============================================================================
// Channel Actions
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ChannelAction {
    /// List all channels
    List,
    /// Get channel status
    Status { id: Option<String> },
    /// Enable a channel
    Enable { id: String },
    /// Disable a channel
    Disable { id: String },
    /// Test channel connection
    Test { id: String },
    /// Send a message via channel
    Send { channel_id: String, target: String, content: String },
}

// =============================================================================
// Workspace Actions
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum WorkspaceAction {
    /// List all registered workspaces
    List,
    /// Get workspace details
    Get { id: String },
    /// Open/register a workspace from path
    Open { path: String },
    /// Close/unregister a workspace
    Close { id: String },
    /// Set active workspace
    SetActive { id: String },
    /// Clear active workspace (global mode)
    ClearActive,
    /// Reload workspace context files
    Reload { id: String },
    /// Get workspace context (SOUL.md, USER.md, etc.)
    GetContext { id: String },
    /// Update workspace context file
    UpdateContext { id: String, file: String, content: String },
}

// =============================================================================
// System Actions
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SystemAction {
    /// Get system status
    Status,
    /// Restart the daemon
    Restart,
    /// Shutdown the daemon
    Shutdown,
    /// Get version info
    Version,
    /// Check for updates
    CheckUpdate,
    /// Apply update
    Update,
    /// Get logs
    Logs { lines: Option<usize>, level: Option<String> },
    /// Health check
    Health,
    /// Get model performance statistics (routing, latency, cache hits, etc.)
    ModelStats,
    /// Get per-tool performance statistics (call counts, latency, error rates)
    ToolStats,
    /// Get global tool + session dashboard stats
    GlobalStats,
    /// Get hourly tool stats time-series (for graphs)
    ToolStatsHourly { tool_name: Option<String>, hours: Option<u32> },
    /// Get daily tool stats time-series (for graphs)
    ToolStatsDaily { tool_name: Option<String>, days: Option<u32> },
    /// Get recent tool call log entries
    ToolCallLog { tool_name: Option<String>, limit: Option<u32> },
}

// =============================================================================
// Subscription Actions
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "topic", rename_all = "snake_case")]
pub enum SubscribeAction {
    /// Subscribe to session events (messages, tool calls)
    Session { session_id: String },
    /// Subscribe to all sessions
    AllSessions,
    /// Subscribe to channel status updates
    ChannelStatus,
    /// Subscribe to system events
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "topic", rename_all = "snake_case")]
pub enum UnsubscribeAction {
    Session { session_id: String },
    AllSessions,
    ChannelStatus,
    System,
}

// =============================================================================
// Response
// =============================================================================

/// Daemon-to-client response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: RequestId,
    pub result: ResponseResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ResponseResult {
    Success { data: Value },
    Error { code: String, message: String },
}

impl Response {
    pub fn success(id: RequestId, data: impl Serialize) -> Self {
        Self {
            id,
            result: ResponseResult::Success {
                data: serde_json::to_value(data).unwrap_or(Value::Null),
            },
        }
    }
    
    pub fn error(id: RequestId, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id,
            result: ResponseResult::Error {
                code: code.into(),
                message: message.into(),
            },
        }
    }
    
    /// Check if this response is an error
    pub fn is_error(&self) -> bool {
        matches!(self.result, ResponseResult::Error { .. })
    }
    
    /// Get the data if successful
    pub fn data(&self) -> Option<&Value> {
        match &self.result {
            ResponseResult::Success { data } => Some(data),
            ResponseResult::Error { .. } => None,
        }
    }
    
    /// Get the error message if failed
    pub fn error_message(&self) -> Option<&str> {
        match &self.result {
            ResponseResult::Error { message, .. } => Some(message),
            ResponseResult::Success { .. } => None,
        }
    }
}

// =============================================================================
// Events (daemon pushes to subscribed clients)
// =============================================================================

/// Server-sent event to subscribed clients
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    // Chat events
    MessageStart { session_id: String, message_id: String },
    MessageDelta { session_id: String, message_id: String, delta: String },
    MessageEnd { session_id: String, message_id: String, content: String },
    
    // Thinking/reasoning events
    ThinkingDelta { session_id: String, delta: String },

    // Tool events
    ToolStart { session_id: String, call_id: String, name: String, input: Value, #[serde(skip_serializing_if = "Option::is_none")] model: Option<String> },
    ToolEnd { session_id: String, call_id: String, output: String, success: bool, duration_ms: u64, #[serde(skip_serializing_if = "Option::is_none")] data: Option<Value> },
    
    // Session events
    SessionCreated { id: String, name: Option<String> },
    SessionDeleted { id: String },
    SessionRenamed { id: String, name: String },
    
    // Memory events
    MemoryCreated { id: String, content: String },
    MemoryUpdated { id: String },
    MemoryDeleted { id: String },
    
    // Channel events
    ChannelConnected { id: String },
    ChannelDisconnected { id: String, reason: Option<String> },
    ChannelError { id: String, error: String },
    ChannelMessage { channel_id: String, sender: String, content: String },
    
    // Sub-session lifecycle events
    SubSessionSpawned {
        session_id: String,
        parent_id: Option<String>,
        label: Option<String>,
        task: String,
    },
    SubSessionCompleted {
        session_id: String,
        parent_id: Option<String>,
        label: Option<String>,
        result: String,
    },
    SubSessionFailed {
        session_id: String,
        parent_id: Option<String>,
        label: Option<String>,
        error: String,
    },
    SubSessionKilled {
        session_id: String,
        parent_id: Option<String>,
        label: Option<String>,
    },
    /// A sub-agent is asking its parent a question and waiting for a reply
    SubSessionQuestion {
        session_id: String,
        parent_id: Option<String>,
        label: Option<String>,
        question: String,
    },

    // Model events
    ModelSwitch { model: String, reason: Option<String> },

    // System events
    StatusChange { status: String },
    Error { code: String, message: String },
    
    // Client connection events
    Connected { client_id: String },
    Disconnected { client_id: String },
}

// =============================================================================
// Control Actions (legacy compat / convenience)
// =============================================================================

/// Convenience type for common control operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ControlAction {
    // Session shortcuts
    ListSessions,
    CreateSession { name: Option<String> },
    SwitchSession { id: String },
    
    // Memory shortcuts
    MemorySearch { query: String, limit: Option<usize> },
    
    // Config shortcuts
    GetConfig,
    SetConfig { path: String, value: Value },
    
    // Tool shortcuts
    ListTools,
    RunTool { name: String, input: Value },
    
    // System shortcuts
    Status,
    Restart,
    Shutdown,
}

impl From<ControlAction> for Action {
    fn from(action: ControlAction) -> Self {
        match action {
            ControlAction::ListSessions => Action::Session(SessionAction::List),
            ControlAction::CreateSession { name } => Action::Session(SessionAction::Create { name }),
            ControlAction::SwitchSession { id } => Action::Session(SessionAction::Switch { id }),
            ControlAction::MemorySearch { query, limit } => Action::Memory(MemoryAction::Search { query, limit, scope: None }),
            ControlAction::GetConfig => Action::Config(ConfigAction::Get { path: None }),
            ControlAction::SetConfig { path, value } => Action::Config(ConfigAction::Set { path, value }),
            ControlAction::ListTools => Action::Tool(ToolAction::List),
            ControlAction::RunTool { name, input } => Action::Tool(ToolAction::Execute { name, input }),
            ControlAction::Status => Action::System(SystemAction::Status),
            ControlAction::Restart => Action::System(SystemAction::Restart),
            ControlAction::Shutdown => Action::System(SystemAction::Shutdown),
        }
    }
}
