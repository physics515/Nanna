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
    // Tasks (P15 agent-grade task store + P14 long-horizon runs)
    // =========================================================================
    Task(TaskAction),

    // =========================================================================
    // Subscriptions
    // =========================================================================
    Subscribe(SubscribeAction),
    Unsubscribe(UnsubscribeAction),
}

// =============================================================================
// Task Actions (P15 store + P14 long-horizon runs)
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum TaskAction {
    /// List tasks in a scope
    List {
        #[serde(default)]
        scope: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        include_closed: Option<bool>,
    },
    /// Get one task with notes + activity
    Get { id: i64 },
    /// The one actionable item
    Next {
        #[serde(default)]
        scope: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
    },
    /// Create a task (full field surface)
    Create {
        title: String,
        #[serde(default)]
        scope: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        parent_id: Option<i64>,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        priority: Option<i64>,
        #[serde(default)]
        labels: Option<Vec<String>>,
        #[serde(default)]
        tools: Option<Vec<String>>,
        #[serde(default)]
        due_at: Option<String>,
        #[serde(default)]
        recurrence: Option<String>,
        #[serde(default)]
        depends_on: Option<Vec<i64>>,
        #[serde(default)]
        acceptance: Option<serde_json::Value>,
        #[serde(default)]
        project: Option<String>,
        #[serde(default)]
        assignee: Option<String>,
    },
    /// Partial update (status accepts pending|in_progress|cancelled)
    Update {
        id: i64,
        #[serde(default)]
        patch: serde_json::Value,
    },
    /// Complete with acceptance verification
    Done {
        id: i64,
        #[serde(default)]
        workdir: Option<String>,
    },
    /// Delete a task subtree
    Delete { id: i64 },
    /// Append a working note
    Note { id: i64, content: String },
    /// Filter-language query
    Query {
        filter: String,
        #[serde(default)]
        scope: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
    },
    /// Start a long-horizon run over a scope's plan (P14)
    StartRun {
        goal: String,
        #[serde(default)]
        scope: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        workdir: Option<String>,
        #[serde(default)]
        max_wall_clock_secs: Option<u64>,
        #[serde(default)]
        max_total_tokens: Option<u64>,
    },
    /// Status of the scope's run (live or last report)
    RunStatus {
        #[serde(default)]
        scope: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
    },
    /// Cancel the scope's active run
    CancelRun {
        #[serde(default)]
        scope: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
    },
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
    CreateInWorkspace {
        name: Option<String>,
        workspace_id: Option<String>,
    },
    /// Rename a session
    Rename { id: String, name: String },
    /// Delete a session
    Delete { id: String },
    /// Delete all sessions
    DeleteAll,
    /// Clear session history
    Clear { id: String },
    /// Get session history
    History {
        id: String,
        limit: Option<usize>,
        before: Option<String>,
    },
    /// Switch active session (for this client)
    Switch { id: String },
    /// Fork a session (create copy)
    Fork { id: String, name: Option<String> },
    /// Get current execution state (in-flight streaming text, active tools).
    /// `light: true` omits the run journal (timeline) — for cheap periodic
    /// polls that only need counters; a full snapshot of a multi-hour run
    /// clones and ships megabytes.
    GetRunState {
        id: String,
        #[serde(default)]
        light: bool,
    },
    /// Session-scoped liveness: is this session working, wedged, or finished
    /// — answered from the daemon's own ledger (current phase, what the turn
    /// awaits, last step, last tool call, last side-effecting call, stop
    /// state) instead of a client grepping logs or proxying on log bytes.
    /// Constant-size response, safe to poll.
    Liveness { id: String },
    /// Set/change the workspace for a session (None = make global)
    SetWorkspace {
        id: String,
        workspace_id: Option<String>,
    },
    /// Set/change the chat model for a session (None = follow the global default).
    /// Chat only — sub-agent, summarization and embedding models stay global.
    ///
    /// `model` is a router spec in the shape `llm.model_priority` already
    /// holds (`ollama/qwen3:14b`, `openrouter/…`, bare `claude-…`), stored
    /// verbatim. The daemon does NOT weigh it against the live providers
    /// here — the only refusal is an unknown `id`, so `ok` means "the pin is
    /// recorded", never "this model will run". A pin nothing serves is caught
    /// at the start of the next turn, which fails loudly naming the model and
    /// saying the pin came from this chat, rather than quietly falling back
    /// to the global default.
    ///
    /// Leaving the check to the turn is the design, not a gap. Acceptance
    /// here could not be a promise in any case: a provider may go down
    /// between the click and the next turn, so the turn's lookup is the one
    /// that has to exist. Running the same router lookup at this boundary as
    /// well would put one policy in two places, which is exactly how a
    /// boundary starts refusing pins the turn would have served (and the
    /// reverse). A client that wants to warn before the user commits should
    /// ask the router what is live and say so in its own words — it must not
    /// read a guarantee into this answer.
    ///
    /// A null — or absent — `model` is the clear, as it is for `SetWorkspace`.
    SetModel {
        id: String,
        model: Option<String>,
    },
    /// Set or clear the user-selected extra tools for a session.
    ///
    /// Strictly additive: the turn unions these into whatever active set it
    /// would have built anyway (core tools, `discover_tools` paging, per-task
    /// scopes all unchanged). An empty — or absent — `tools` is the clear,
    /// restoring byte-identical default behavior. Names are stored verbatim;
    /// an unknown name costs nothing (the registry simply has nothing to
    /// activate for it), so the only refusal here is an unknown `id`.
    SetTools {
        id: String,
        #[serde(default)]
        tools: Vec<String>,
    },

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
    Search {
        query: String,
        limit: Option<usize>,
        scope: Option<String>,
    },
    /// Get a specific memory
    Get { id: String },
    /// Create a memory
    Create {
        content: String,
        tags: Option<Vec<String>>,
        importance: Option<u8>,
    },
    /// Update a memory
    Update {
        id: String,
        content: Option<String>,
        tags: Option<Vec<String>>,
    },
    /// Delete a memory
    Delete { id: String },
    /// Clear memories. `scope`: None = all, "global" = global-only,
    /// anything else = that workspace's entries only.
    Clear {
        #[serde(default)]
        scope: Option<String>,
    },
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
    Create {
        name: String,
        description: String,
        code: String,
        needs_shell: Option<bool>,
    },
    /// Update a user tool
    Update {
        name: String,
        description: Option<String>,
        code: Option<String>,
        needs_shell: Option<bool>,
    },
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
    Add {
        schedule: String,
        task: String,
        name: Option<String>,
    },
    /// Update a job
    Update {
        id: String,
        schedule: Option<String>,
        task: Option<String>,
        enabled: Option<bool>,
    },
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
    Send {
        channel_id: String,
        target: String,
        content: String,
    },
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
    /// Get workspace context (README.md, AGENTS.md, ROADMAP.md, …)
    GetContext { id: String },
    /// Update workspace context file
    UpdateContext {
        id: String,
        file: String,
        content: String,
    },
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
    Logs {
        lines: Option<usize>,
        level: Option<String>,
    },
    /// Health check
    Health,
    /// Get model performance statistics (routing, latency, cache hits, etc.)
    ModelStats,
    /// Get per-tool performance statistics (call counts, latency, error rates)
    ToolStats,
    /// Get global tool + session dashboard stats
    GlobalStats,
    /// Get hourly tool stats time-series (for graphs)
    ToolStatsHourly {
        tool_name: Option<String>,
        hours: Option<u32>,
    },
    /// Get daily tool stats time-series (for graphs)
    ToolStatsDaily {
        tool_name: Option<String>,
        days: Option<u32>,
    },
    /// Get recent tool call log entries
    ToolCallLog {
        tool_name: Option<String>,
        limit: Option<u32>,
    },
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
    MessageStart {
        session_id: String,
        message_id: String,
    },
    MessageDelta {
        session_id: String,
        message_id: String,
        delta: String,
    },
    MessageEnd {
        session_id: String,
        message_id: String,
        content: String,
    },

    // Thinking/reasoning events
    ThinkingDelta {
        session_id: String,
        delta: String,
    },

    /// The harness moved on to a plan item. Run mechanics for a status row,
    /// never message content — see `TimelineItem::Step`.
    StepStarted {
        session_id: String,
        /// `working` | `planning` | `verifying`
        kind: String,
        label: String,
        item_id: i64,
    },

    // Tool events
    ToolStart {
        session_id: String,
        call_id: String,
        name: String,
        input: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        /// Tokens spent by the LLM request that issued this call.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens: Option<u64>,
        /// Run-total tokens spent when this call was issued.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        total_tokens: Option<u64>,
    },
    ToolEnd {
        session_id: String,
        call_id: String,
        output: String,
        success: bool,
        duration_ms: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<Value>,
    },

    /// Low-frequency liveness beat while a chat turn is in flight. Lets any
    /// watcher — GUI spinner, a script, a bench driver — distinguish "the
    /// model is thinking" from "the process is gone" without reading logs:
    /// beats continue while the daemon is alive and the turn is live; beats
    /// stopping without a `message_end` means wedged or dead. Cadence is
    /// derived from the silence budgets (`liveness::beat_interval_secs`),
    /// ~30s today.
    LivenessBeat {
        session_id: String,
        /// Seconds since this turn started.
        elapsed_s: u64,
        /// Coarse phase: planning | step_pending | streaming | thinking | tool.
        phase: String,
        /// What the turn is waiting on right now, human-readable
        /// (e.g. "model output (ollama/qwen3.5:9b): last token 41s ago").
        awaiting: String,
        /// Seconds since the last observed event in this turn.
        #[serde(skip_serializing_if = "Option::is_none")]
        quiet_s: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        step_index: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        last_tool: Option<String>,
        /// Monotone beat counter within the turn (gap detection).
        beat: u64,
    },

    // Session events
    SessionCreated {
        id: String,
        name: Option<String>,
    },
    SessionDeleted {
        id: String,
    },
    SessionRenamed {
        id: String,
        name: String,
    },

    /// The set of known workspaces changed — one was registered or removed.
    ///
    /// Carries no payload and says nothing about which workspace is active.
    /// Activation is a per-client view concern: several clients may sit in
    /// different workspaces at once, and pushing an active id would yank one
    /// client's view because another opened something. Clients re-fetch the
    /// list and leave their own selection alone.
    ///
    /// Without this a workspace registered after a client connected stayed
    /// invisible to it until restart.
    WorkspacesChanged,

    /// The daemon's configuration was mutated and committed (set / reset /
    /// reload / import).
    ///
    /// Payload-free for the same reason as [`Event::WorkspacesChanged`]: the
    /// config is shared state, and pushing a diff would make every client
    /// re-implement the merge. Clients re-fetch whatever slice they render.
    ///
    /// One event per committed mutation — the model priority a client shows
    /// otherwise stays whatever it read at mount, so a change made in another
    /// window (or by the agent itself) was invisible until restart.
    ConfigChanged,

    // Memory events
    MemoryCreated {
        id: String,
        content: String,
    },
    /// The durable memory store had page-level corruption at startup: the
    /// damaged database file was quarantined and a fresh store rebuilt from
    /// whatever rows were still reachable. `expected` is `None` when the
    /// corrupt store's row count could not even be read.
    MemoryStoreRebuilt {
        recovered: usize,
        expected: Option<usize>,
        quarantine_path: String,
    },
    MemoryUpdated {
        id: String,
    },
    MemoryDeleted {
        id: String,
    },

    // Channel events
    ChannelConnected {
        id: String,
    },
    ChannelDisconnected {
        id: String,
        reason: Option<String>,
    },
    ChannelError {
        id: String,
        error: String,
    },
    ChannelMessage {
        channel_id: String,
        sender: String,
        content: String,
    },

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
    ModelSwitch {
        model: String,
        reason: Option<String>,
    },

    // System events
    StatusChange {
        status: String,
    },
    /// Live context-window usage for a session's active run. Emitted after
    /// every LLM request: `used` = provider-reported prompt tokens of that
    /// request, `window` = the enforced context bound.
    ContextUsage {
        session_id: String,
        used: u64,
        window: u64,
    },
    Error {
        code: String,
        message: String,
        /// The session the error belongs to, when known. Clients must not
        /// attribute session-less errors to whatever session is open.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },

    // Client connection events
    Connected {
        client_id: String,
    },
    Disconnected {
        client_id: String,
    },

    // Task events (P15 store mutations + P14 long-horizon run lifecycle)
    TaskRunStarted {
        scope: String,
        scope_id: Option<String>,
        goal: String,
    },
    TaskRunProgress {
        scope: String,
        scope_id: Option<String>,
        task_id: Option<i64>,
        /// started | completed | abandoned | acceptance_checked | replanned | ...
        kind: String,
        detail: serde_json::Value,
    },
    TaskRunCompleted {
        scope: String,
        scope_id: Option<String>,
        /// Serialized `LongHorizonReport`
        report: serde_json::Value,
    },
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
            ControlAction::CreateSession { name } => {
                Action::Session(SessionAction::Create { name })
            }
            ControlAction::SwitchSession { id } => Action::Session(SessionAction::Switch { id }),
            ControlAction::MemorySearch { query, limit } => Action::Memory(MemoryAction::Search {
                query,
                limit,
                scope: None,
            }),
            ControlAction::GetConfig => Action::Config(ConfigAction::Get { path: None }),
            ControlAction::SetConfig { path, value } => {
                Action::Config(ConfigAction::Set { path, value })
            }
            ControlAction::ListTools => Action::Tool(ToolAction::List),
            ControlAction::RunTool { name, input } => {
                Action::Tool(ToolAction::Execute { name, input })
            }
            ControlAction::Status => Action::System(SystemAction::Status),
            ControlAction::Restart => Action::System(SystemAction::Restart),
            ControlAction::Shutdown => Action::System(SystemAction::Shutdown),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every client hand-writes this envelope as JSON (the GUI's
    /// `daemon_client.rs`, the CLI, anything on the socket), so the tag
    /// strings ARE the contract: renaming the variant is a silent
    /// "unknown action" for all of them and not a compile error anywhere.
    #[test]
    fn a_chat_model_pin_arrives_as_the_documented_envelope() {
        let raw = serde_json::json!({
            "type": "session",
            "action": "set_model",
            "id": "sess-1",
            "model": "ollama/qwen3:14b",
        });

        match serde_json::from_value::<Action>(raw).expect("set_model must parse") {
            Action::Session(SessionAction::SetModel { id, model }) => {
                assert_eq!(id, "sess-1");
                // Stored and routed verbatim — the provider prefix is part of
                // the spec, not decoration to be stripped on the wire.
                assert_eq!(model.as_deref(), Some("ollama/qwen3:14b"));
            }
            other => panic!("expected a session set_model action, got {other:?}"),
        }
    }

    /// Un-pinning is `"model": null`, exactly as `set_workspace` un-scopes a
    /// session with a null `workspace_id` — and an ABSENT key means the same
    /// thing, because serde's internally-tagged path defaults a missing
    /// `Option` to `None`. Pinned because that is the entire difference
    /// between "clear this chat's model" and "I forgot a field": a client
    /// that drops the key un-pins the chat and is told nothing.
    #[test]
    fn a_null_or_absent_model_both_clear_the_pin() {
        for raw in [
            serde_json::json!({
                "type": "session", "action": "set_model", "id": "sess-1", "model": null,
            }),
            serde_json::json!({ "type": "session", "action": "set_model", "id": "sess-1" }),
        ] {
            let action = serde_json::from_value::<Action>(raw).expect("a clear must parse");
            assert!(
                matches!(
                    action,
                    Action::Session(SessionAction::SetModel { model: None, .. })
                ),
                "no model on the wire is the clear, not a parse error"
            );
        }
    }
}
