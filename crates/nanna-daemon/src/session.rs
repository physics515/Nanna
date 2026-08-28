//! Session management
//!
//! Sessions represent conversations with the agent. Multiple channels
//! can subscribe to the same session.
//!
//! All session and message data is persisted to Turso via nanna-storage.
//! The in-memory HashMap serves as a hot cache for fast access.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Unique session identifier
pub type SessionId = String;

/// Channel identifier (e.g., "gui:abc123", "telegram:456")
pub type ChannelId = String;

/// A message in a session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCallRecord>,
    #[serde(default)]
    pub attachments: Vec<AttachmentRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Chronological journal of the run that produced this message (thinking
    /// segments, tool calls, text segments, healed faults — in order). The
    /// flat `tool_calls`/`reasoning` fields above are kept for older
    /// messages and for model-context reconstruction; when `timeline` is
    /// non-empty the UI renders it instead.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub timeline: Vec<TimelineItem>,
    /// Token + wall-clock totals for the run (model benchmarking).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<RunUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

impl MessageRole {
    /// Convert to the string format used in the database.
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::System => "system",
            Self::Tool => "tool",
        }
    }

    /// Parse from the string format used in the database.
    pub fn from_db_str(s: &str) -> Self {
        match s {
            "user" => Self::User,
            "assistant" => Self::Assistant,
            "system" => Self::System,
            "tool" => Self::Tool,
            _ => Self::User,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
    pub output: Option<String>,
    pub success: Option<bool>,
    pub duration_ms: Option<u64>,
}

/// One entry in a run's chronological journal. A long-horizon run is not
/// "one thinking blob + one flat tool list + one text blob" — it is an
/// interleaved sequence (think → call tools → think → speak → …), and for
/// runs that heal through provider faults it can span many attempts. The
/// timeline records events in the order they happened so the UI can replay
/// the run faithfully, including after navigation away and back.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TimelineItem {
    /// A contiguous burst of thinking/reasoning (closed by the next tool
    /// call or text output).
    Thinking { content: String, at: String },
    /// A contiguous burst of visible assistant text.
    Text { content: String, at: String },
    /// One tool call. `output`/`success`/`duration_ms` are back-filled when
    /// the call completes; a run that dies mid-call leaves them None.
    Tool {
        call_id: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        success: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
        /// Tokens spent on the action: input+output of the LLM request that
        /// issued this tool call (parallel calls from one request share it).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens: Option<u64>,
        /// Run-total tokens spent at the moment this call was issued.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        total_tokens: Option<u64>,
        at: String,
    },
    /// A provider fault the run healed through (stream drop, timeout, …).
    /// Recorded so the journal explains why thinking/text may restart:
    /// the attempt after a fault regenerates rather than resumes.
    Fault { message: String, at: String },
    /// The harness starting work on one plan item.
    ///
    /// This is run MECHANICS, not conversation. It used to be written into
    /// the assistant's message as literal `**[working]** …` markdown, which
    /// had three costs: the transcript read as a wall of banners instead of a
    /// reply, the banners were persisted into conversation history and fed
    /// back to the model as context on later turns, and they crowded out the
    /// narration a watcher actually wants. As a timeline item the GUI can
    /// render it as a status row that updates in place.
    Step {
        /// `working` | `planning` | `verifying`. Named `phase` because the
        /// enum is internally tagged on `kind`.
        phase: String,
        /// Human-readable item title, e.g. "Implement the get command".
        label: String,
        item_id: i64,
        at: String,
    },
}

/// Resource totals for one run, for benchmarking models against each other
/// on identical tasks: total tokens spent and wall-clock time taken.
/// Token totals accumulate across EVERY healing attempt via the per-request
/// usage callback — not just the attempt that finally succeeded. (Streams
/// that die before the provider reports usage still under-count slightly;
/// that loss is inherent to the protocol.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub duration_ms: u64,
    /// The model that finished the run (last model used).
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentRecord {
    pub id: String,
    pub filename: String,
    pub content_type: String,
    pub url: Option<String>,
}

/// Metadata key holding a session's chat-model override.
///
/// Named `chat_model` rather than `model` because it names the ONE thing the
/// override covers: the absence of `sub_agent_model` / `summarization_model` /
/// `embedding_model` keys reads as deliberate rather than forgotten. Those stay
/// global — see `[llm]` and `[embedding]` in the config.
const CHAT_MODEL_KEY: &str = "chat_model";

/// Metadata key holding a session's user-selected extra tools.
///
/// Named `chat_tools` beside `chat_model` because it is the same kind of
/// per-chat override: a selection the user made for THIS conversation. The
/// semantics are strictly additive — these tools are unioned into the active
/// set a turn starts with, and an empty/absent list means "exactly the
/// default behavior", so a session that never picked is byte-identical to
/// before the feature existed.
const CHAT_TOOLS_KEY: &str = "chat_tools";

/// A conversation session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub messages: Vec<SessionMessage>,
    /// Channels subscribed to this session (receive events)
    #[serde(default)]
    pub subscribers: HashSet<ChannelId>,
    /// Channel that "owns" this session (can clear, rename, etc.)
    pub owner: Option<ChannelId>,
    /// Session metadata
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
    /// Workspace this session belongs to (None = global)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
}

impl Session {
    /// Create a new session
    pub fn new(name: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            subscribers: HashSet::new(),
            owner: None,
            metadata: HashMap::new(),
            workspace_id: None,
        }
    }
    
    /// Create a session with a specific ID
    pub fn with_id(id: impl Into<String>, name: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            id: id.into(),
            name,
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            subscribers: HashSet::new(),
            owner: None,
            metadata: HashMap::new(),
            workspace_id: None,
        }
    }

    /// Set the workspace ID for this session
    pub fn with_workspace(mut self, workspace_id: impl Into<String>) -> Self {
        self.workspace_id = Some(workspace_id.into());
        self
    }
    
    /// This session's chat-model override, if the user picked one.
    ///
    /// `None` means "use the global default" — the absence of the key is the
    /// unset state, so a session that never picked behaves exactly as it did
    /// before overrides existed. The spec is stored verbatim in router form
    /// (`ollama/qwen3:14b`, `openrouter/…`, bare `claude-…`), the same shape
    /// `llm.model_priority` holds, so a caller can hand it straight to
    /// `LlmRouter::client_for_model`.
    pub fn chat_model(&self) -> Option<&str> {
        self.metadata.get(CHAT_MODEL_KEY).and_then(serde_json::Value::as_str)
    }

    /// Tools the user manually added to this chat's context, if any.
    ///
    /// Empty means "no manual selection" — the unset state — so a turn built
    /// from it behaves exactly as it did before per-chat tool selection
    /// existed. Selections only ever ADD to the default active set; they
    /// never remove or replace defaults.
    pub fn chat_tools(&self) -> Vec<String> {
        self.metadata
            .get(CHAT_TOOLS_KEY)
            .and_then(serde_json::Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Add a message to the session (in-memory only — use SessionManager for persistence)
    pub fn add_message(&mut self, role: MessageRole, content: impl Into<String>) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.messages.push(SessionMessage {
            id: id.clone(),
            role,
            content: content.into(),
            timestamp: Utc::now(),
            tool_calls: Vec::new(),
            attachments: Vec::new(),
            reasoning: None,
            timeline: Vec::new(),
            usage: None,
        });
        self.updated_at = Utc::now();
        id
    }

    /// Add a message with tool calls, reasoning, run timeline, and usage
    /// totals to the session (in-memory only)
    pub fn add_full_message(
        &mut self,
        role: MessageRole,
        content: impl Into<String>,
        tool_calls: Vec<ToolCallRecord>,
        reasoning: Option<String>,
        timeline: Vec<TimelineItem>,
        usage: Option<RunUsage>,
    ) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.messages.push(SessionMessage {
            id: id.clone(),
            role,
            content: content.into(),
            timestamp: Utc::now(),
            tool_calls,
            attachments: Vec::new(),
            reasoning,
            timeline,
            usage,
        });
        self.updated_at = Utc::now();
        id
    }
    
    /// Subscribe a channel to this session
    pub fn subscribe(&mut self, channel_id: ChannelId) {
        self.subscribers.insert(channel_id);
    }
    
    /// Unsubscribe a channel from this session
    pub fn unsubscribe(&mut self, channel_id: &str) {
        self.subscribers.remove(channel_id);
    }
    
    /// Check if a channel is subscribed
    pub fn is_subscribed(&self, channel_id: &str) -> bool {
        self.subscribers.contains(channel_id)
    }
    
    /// Set the session owner
    pub fn set_owner(&mut self, channel_id: Option<ChannelId>) {
        self.owner = channel_id;
    }
    
    /// Clear all messages
    pub fn clear(&mut self) {
        self.messages.clear();
        self.updated_at = Utc::now();
    }

    /// Prepare the session to regenerate the last assistant response.
    ///
    /// Drops the most recent user message **and everything after it** (the
    /// stale assistant reply plus any trailing tool turns), returning that user
    /// message's content so the caller can replay the turn through the normal
    /// send path (which re-adds the user message and runs the agent afresh).
    ///
    /// Returns `None` and leaves the session unchanged when there is no user
    /// message to regenerate from.
    pub fn take_last_user_turn(&mut self) -> Option<String> {
        let idx = self
            .messages
            .iter()
            .rposition(|m| matches!(m.role, MessageRole::User))?;
        let content = self.messages[idx].content.clone();
        // Remove the user message and any messages that followed it.
        self.messages.truncate(idx);
        self.updated_at = Utc::now();
        Some(content)
    }

    /// Get display name (name or truncated ID)
    pub fn display_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            format!("Session {}", &self.id[..floor_boundary(&self.id, 8)])
        })
    }
    
    /// Get message count
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }
}

/// Session summary for listing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: SessionId,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: usize,
    pub subscriber_count: usize,
    pub owner: Option<ChannelId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// Chat-model override, mirrored out of `metadata` so a client that lists
    /// sessions can render the pin without a `session.get` per row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_model: Option<String>,
    /// User-selected extra tools, mirrored out of `metadata` for the same
    /// reason as `chat_model`. Empty = no manual selection (default behavior).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chat_tools: Vec<String>,
}

impl From<&Session> for SessionSummary {
    fn from(session: &Session) -> Self {
        Self {
            id: session.id.clone(),
            name: session.name.clone(),
            created_at: session.created_at,
            updated_at: session.updated_at,
            message_count: session.messages.len(),
            subscriber_count: session.subscribers.len(),
            owner: session.owner.clone(),
            workspace_id: session.workspace_id.clone(),
            chat_model: session.chat_model().map(str::to_string),
            chat_tools: session.chat_tools(),
        }
    }
}

/// State of a sub-agent session
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubSessionState {
    Spawning,
    Running,
    Waiting,
    Completed,
    Failed,
    Killed,
}

/// Metadata for a sub-agent session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubSessionInfo {
    /// Session ID
    pub session_id: SessionId,
    /// Parent session ID (None for top-level sessions)
    pub parent_id: Option<SessionId>,
    /// Human-readable label
    pub label: Option<String>,
    /// Task description / initial prompt
    pub task: String,
    /// Current state
    pub state: SubSessionState,
    /// When it was spawned
    pub spawned_at: DateTime<Utc>,
    /// When it completed/failed/was killed
    pub finished_at: Option<DateTime<Utc>>,
    /// Model used
    pub model: Option<String>,
    /// Result summary (on completion)
    pub result: Option<String>,
    /// Error message (on failure)
    pub error: Option<String>,
    /// Cancellation flag for cooperative shutdown
    #[serde(skip)]
    pub cancellation_flag: Option<Arc<std::sync::atomic::AtomicBool>>,
}

/// A message in the sub-session mailbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailboxMessage {
    pub from: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
}

/// Serialize a SessionMessage's extra fields (tool_calls, attachments, reasoning, timeline, usage) to JSON metadata.
fn message_to_metadata(msg: &SessionMessage) -> Option<String> {
    let has_tool_calls = !msg.tool_calls.is_empty();
    let has_attachments = !msg.attachments.is_empty();
    let has_reasoning = msg.reasoning.is_some();
    let has_timeline = !msg.timeline.is_empty();
    let has_usage = msg.usage.is_some();

    if !has_tool_calls && !has_attachments && !has_reasoning && !has_timeline && !has_usage {
        return None;
    }

    let mut meta = serde_json::Map::new();
    if has_tool_calls {
        meta.insert("tool_calls".to_string(), serde_json::to_value(&msg.tool_calls).unwrap_or_default());
    }
    if has_attachments {
        meta.insert("attachments".to_string(), serde_json::to_value(&msg.attachments).unwrap_or_default());
    }
    if let Some(ref reasoning) = msg.reasoning {
        meta.insert("reasoning".to_string(), serde_json::Value::String(reasoning.clone()));
    }
    if has_timeline {
        meta.insert("timeline".to_string(), serde_json::to_value(&msg.timeline).unwrap_or_default());
    }
    if let Some(ref usage) = msg.usage {
        meta.insert("usage".to_string(), serde_json::to_value(usage).unwrap_or_default());
    }
    Some(serde_json::Value::Object(meta).to_string())
}

/// Deserialize a DB message row back into a SessionMessage.
fn db_message_to_session_message(
    message_id: &str,
    role: &str,
    content: &str,
    created_at: &str,
    metadata: Option<&serde_json::Value>,
) -> SessionMessage {
    let tool_calls = metadata
        .and_then(|m| m.get("tool_calls"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let attachments = metadata
        .and_then(|m| m.get("attachments"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let reasoning = metadata
        .and_then(|m| m.get("reasoning"))
        .and_then(|v| v.as_str())
        .map(String::from);
    let timeline = metadata
        .and_then(|m| m.get("timeline"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let usage = metadata
        .and_then(|m| m.get("usage"))
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    // Parse timestamp, fall back to now
    let timestamp = chrono::DateTime::parse_from_rfc3339(created_at)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| {
            // Try Turso datetime format: "2026-03-31 14:09:49"
            chrono::NaiveDateTime::parse_from_str(created_at, "%Y-%m-%d %H:%M:%S")
                .map(|ndt| ndt.and_utc())
        })
        .unwrap_or_else(|_| Utc::now());

    SessionMessage {
        id: message_id.to_string(),
        role: MessageRole::from_db_str(role),
        content: content.to_string(),
        timestamp,
        tool_calls,
        attachments,
        reasoning,
        timeline,
        usage,
    }
}

/// The `sessions` row a persist writes, lifted out from under the map's guard.
///
/// A write-through has to survive the guard being dropped, because the database
/// round trip must not happen while the map is locked. Carrying the whole
/// [`Session`] across that await would copy its entire message history — the
/// largest thing a session owns — for what is often a one-key metadata change,
/// so only the columns the upsert actually binds come along.
struct SessionRow {
    id: SessionId,
    name: Option<String>,
    workspace_id: Option<String>,
    created_at: String,
    updated_at: String,
    metadata: HashMap<String, serde_json::Value>,
}

impl From<&Session> for SessionRow {
    fn from(session: &Session) -> Self {
        Self {
            id: session.id.clone(),
            name: session.name.clone(),
            workspace_id: session.workspace_id.clone(),
            created_at: session.created_at.to_rfc3339(),
            updated_at: session.updated_at.to_rfc3339(),
            metadata: session.metadata.clone(),
        }
    }
}

/// Manages all sessions with write-through to Turso.
pub struct SessionManager {
    sessions: Arc<RwLock<HashMap<SessionId, Session>>>,
    /// Default session ID (for new clients)
    default_session: Arc<RwLock<Option<SessionId>>>,
    /// Sub-session registry (session_id -> info)
    sub_sessions: Arc<RwLock<HashMap<SessionId, SubSessionInfo>>>,
    /// Per-session mailbox for inter-session messaging
    mailboxes: Arc<RwLock<HashMap<SessionId, Vec<MailboxMessage>>>>,
    /// Database storage for persistence (None = in-memory only, e.g. tests)
    storage: Option<Arc<nanna_storage::Storage>>,
}

impl SessionManager {
    /// Create a new session manager (no persistence)
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            default_session: Arc::new(RwLock::new(None)),
            sub_sessions: Arc::new(RwLock::new(HashMap::new())),
            mailboxes: Arc::new(RwLock::new(HashMap::new())),
            storage: None,
        }
    }

    /// Create a new session manager backed by Turso storage
    pub fn with_storage(storage: Arc<nanna_storage::Storage>) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            default_session: Arc::new(RwLock::new(None)),
            sub_sessions: Arc::new(RwLock::new(HashMap::new())),
            mailboxes: Arc::new(RwLock::new(HashMap::new())),
            storage: Some(storage),
        }
    }

    /// Load all daemon sessions and their messages from Turso into the in-memory cache.
    /// Call this once at startup.
    pub async fn load_from_db(&self) -> usize {
        let Some(ref storage) = self.storage else {
            return 0;
        };

        let db_sessions = match storage.list_daemon_sessions().await {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to load sessions from database: {}", e);
                return 0;
            }
        };

        let count = db_sessions.len();
        let mut sessions = self.sessions.write().await;
        let mut default = self.default_session.write().await;

        for db_session in db_sessions {
            let session_id = db_session.session_id.clone();

            // Parse timestamps
            let created_at = chrono::DateTime::parse_from_rfc3339(&db_session.created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .or_else(|_| {
                    chrono::NaiveDateTime::parse_from_str(&db_session.created_at, "%Y-%m-%d %H:%M:%S")
                        .map(|ndt| ndt.and_utc())
                })
                .unwrap_or_else(|_| Utc::now());
            let updated_at = chrono::DateTime::parse_from_rfc3339(&db_session.updated_at)
                .map(|dt| dt.with_timezone(&Utc))
                .or_else(|_| {
                    chrono::NaiveDateTime::parse_from_str(&db_session.updated_at, "%Y-%m-%d %H:%M:%S")
                        .map(|ndt| ndt.and_utc())
                })
                .unwrap_or_else(|_| Utc::now());

            // Load messages from DB
            let db_messages = match storage.load_daemon_messages(&session_id).await {
                Ok(msgs) => msgs,
                Err(e) => {
                    warn!("Failed to load messages for session {}: {}", session_id, e);
                    Vec::new()
                }
            };

            let messages: Vec<SessionMessage> = db_messages.iter().map(|m| {
                db_message_to_session_message(
                    m.tool_use_id.as_deref().unwrap_or(&m.id.to_string()),
                    &m.role,
                    &m.content,
                    &m.created_at,
                    m.metadata.as_ref(),
                )
            }).collect();

            let session = Session {
                id: session_id.clone(),
                name: db_session.name,
                created_at,
                updated_at,
                messages,
                subscribers: HashSet::new(),
                owner: None,
                metadata: db_session.metadata
                    .and_then(|v| {
                        if let serde_json::Value::Object(map) = v {
                            Some(map.into_iter().collect())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default(),
                workspace_id: db_session.workspace_id,
            };

            sessions.insert(session_id.clone(), session);

            // First session becomes default
            if default.is_none() {
                *default = Some(session_id);
            }
        }

        info!("Loaded {} sessions from database", count);
        count
    }

    /// Persist session metadata to DB (fire-and-forget on errors)
    async fn persist_session(&self, session: &Session) {
        self.persist_row(&SessionRow::from(session)).await;
    }

    /// Write one `sessions` row, metadata included (fire-and-forget on errors).
    ///
    /// The metadata blob always travels, even when the map is empty. The SQL is
    /// `metadata = COALESCE(?6, metadata)`, so a NULL means "keep whatever is on
    /// disk" — and sending NULL for an empty map made a key the user just
    /// REMOVED survive the write, with `load_from_db` resurrecting it on the
    /// next daemon start. Un-pinning a chat's model would have looked like it
    /// worked right up until a restart put the pin back. The literal `{}` is
    /// what makes a removal land.
    async fn persist_row(&self, row: &SessionRow) {
        let Some(ref storage) = self.storage else {
            warn!("persist_session called but no storage backend — session {} will not be persisted", row.id);
            return;
        };
        let metadata = match serde_json::to_string(&row.metadata) {
            Ok(json) => json,
            Err(e) => {
                warn!("Failed to serialize metadata for session {}: {}", row.id, e);
                return;
            }
        };
        match storage.upsert_daemon_session(
            &row.id,
            row.name.as_deref(),
            row.workspace_id.as_deref(),
            &row.created_at,
            &row.updated_at,
            Some(&metadata),
        ).await {
            Ok(()) => info!("Persisted session {} to database", row.id),
            Err(e) => warn!("Failed to persist session {} to database: {}", row.id, e),
        }
    }

    /// Persist a single message to DB
    async fn persist_message(&self, session_id: &str, msg: &SessionMessage) {
        let Some(ref storage) = self.storage else {
            warn!("persist_message called but no storage backend — message {} will not be persisted", msg.id);
            return;
        };
        let created = msg.timestamp.to_rfc3339();
        let metadata = message_to_metadata(msg);
        match storage.add_daemon_message(
            session_id,
            &msg.id,
            msg.role.as_db_str(),
            &msg.content,
            &created,
            metadata.as_deref(),
        ).await {
            Ok(()) => info!("Persisted {} message {} in session {}", msg.role.as_db_str(), msg.id, session_id),
            Err(e) => warn!("Failed to persist message {} in session {}: {}", msg.id, session_id, e),
        }
    }
    
    /// Create a session and return it
    pub async fn create(&self, name: Option<String>) -> Session {
        self.create_in_workspace(name, None).await
    }

    /// Create a new session in a specific workspace
    pub async fn create_in_workspace(&self, name: Option<String>, workspace_id: Option<String>) -> Session {
        let mut session = Session::new(name);
        session.workspace_id = workspace_id;
        let id = session.id.clone();
        
        // Persist to DB first
        self.persist_session(&session).await;

        let mut sessions = self.sessions.write().await;
        sessions.insert(id.clone(), session.clone());
        
        // Set as default if it's the first session
        let mut default = self.default_session.write().await;
        if default.is_none() {
            *default = Some(id.clone());
        }
        
        info!("Created session: {} (workspace: {:?})", id, session.workspace_id);
        session
    }
    
    /// Set or clear the workspace for an existing session
    pub async fn set_workspace(&self, session_id: &str, workspace_id: Option<String>) -> bool {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            session.workspace_id = workspace_id.clone();
            // Persist to DB
            if let Some(ref storage) = self.storage {
                if let Err(e) = storage.set_daemon_session_workspace(session_id, workspace_id.as_deref()).await {
                    warn!("Failed to persist workspace change for session {}: {}", session_id, e);
                }
            }
            true
        } else {
            false
        }
    }

    /// Set or clear this session's chat model (`None` = follow the global
    /// `[llm]` default).
    ///
    /// Chat only. Sub-agent, summarization and embedding models stay global on
    /// purpose — this writes one key and touches nothing else.
    pub async fn set_chat_model(&self, session_id: &str, model: Option<String>) -> bool {
        let Some(row) = self.apply_chat_model(session_id, model).await else {
            return false;
        };
        // The database round trip runs with the map unlocked — see
        // `apply_chat_model` for why that split exists.
        self.persist_row(&row).await;
        true
    }

    /// Apply a chat-model pick to the cached session and hand back the row that
    /// still has to be written; `None` when there is no such session.
    ///
    /// The two halves are separate because the `sessions` WRITE guard must not
    /// be held across the storage await. It once was, and every concurrent
    /// `get`/`list` then queued behind a full database round trip — including
    /// the read an in-flight turn does in `prepare_chat_turn` to resolve this
    /// very pin. The guard dies with this function; only the row crosses into
    /// the write.
    async fn apply_chat_model(&self, session_id: &str, model: Option<String>) -> Option<SessionRow> {
        // Normalize at the boundary so every reader sees a single shape for
        // "no override". A blank pick from a client is a CLEAR, not a pin on
        // the empty string — which no provider serves, so it would only ever
        // reach the turn's fail-fast.
        let model = model.filter(|spec| !spec.trim().is_empty());

        let mut sessions = self.sessions.write().await;
        let session = sessions.get_mut(session_id)?;
        match model {
            Some(spec) => {
                session
                    .metadata
                    .insert(CHAT_MODEL_KEY.to_string(), serde_json::Value::String(spec));
            }
            None => {
                session.metadata.remove(CHAT_MODEL_KEY);
            }
        }
        session.updated_at = Utc::now();
        info!("Session {} chat model set to {:?}", session_id, session.chat_model());
        Some(SessionRow::from(&*session))
    }

    /// Set or clear this session's user-selected extra tools (empty = no
    /// manual selection, i.e. default tool behavior).
    ///
    /// Additive by contract: the turn unions these into its default active
    /// set — they never remove or replace defaults. This writes one metadata
    /// key and touches nothing else.
    pub async fn set_chat_tools(&self, session_id: &str, tools: Vec<String>) -> bool {
        let Some(row) = self.apply_chat_tools(session_id, tools).await else {
            return false;
        };
        // The database round trip runs with the map unlocked — see
        // `apply_chat_model` for why that split exists.
        self.persist_row(&row).await;
        true
    }

    /// Apply a tool selection to the cached session and hand back the row that
    /// still has to be written; `None` when there is no such session.
    ///
    /// Same lock discipline as `apply_chat_model`: the write guard dies with
    /// this function; only the row crosses into the persist.
    async fn apply_chat_tools(&self, session_id: &str, tools: Vec<String>) -> Option<SessionRow> {
        // Normalize at the boundary: blank names are noise, and an empty
        // selection is stored as ABSENCE of the key so "never picked" and
        // "cleared the picks" are the same unset state.
        let tools: Vec<String> = tools
            .into_iter()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();

        let mut sessions = self.sessions.write().await;
        let session = sessions.get_mut(session_id)?;
        if tools.is_empty() {
            session.metadata.remove(CHAT_TOOLS_KEY);
        } else {
            session.metadata.insert(
                CHAT_TOOLS_KEY.to_string(),
                serde_json::Value::Array(
                    tools.into_iter().map(serde_json::Value::String).collect(),
                ),
            );
        }
        session.updated_at = Utc::now();
        info!("Session {} chat tools set to {:?}", session_id, session.chat_tools());
        Some(SessionRow::from(&*session))
    }

    /// Get a session by ID
    pub async fn get(&self, id: &str) -> Option<Session> {
        let sessions = self.sessions.read().await;
        sessions.get(id).cloned()
    }
    
    /// Get or create the default session
    pub async fn get_or_create_default(&self) -> Session {
        // Check if default exists
        let default_id = {
            let default = self.default_session.read().await;
            default.clone()
        };
        
        if let Some(id) = default_id {
            if let Some(session) = self.get(&id).await {
                return session;
            }
        }
        
        // Create new default
        self.create(Some("Main".to_string())).await
    }
    
    /// List all sessions
    pub async fn list(&self) -> Vec<SessionSummary> {
        let sessions = self.sessions.read().await;
        sessions.values().map(SessionSummary::from).collect()
    }
    
    /// Update a session from a caller-held snapshot.
    ///
    /// The snapshot is not authoritative about the chat-model pin. Callers read
    /// a session, mutate their copy, and write it back — `ChatAction::Regenerate`
    /// does exactly that — so a `session.setModel` landing inside that window
    /// would be undone by the write: a stale snapshot resurrects a pin the user
    /// just cleared, or drops one they just set, and the chat then runs on a
    /// model the picker says it is not using. [`Self::set_chat_model`] is the
    /// single writer of that key and the live map is where its write landed, so
    /// the live value wins and `update` stays pin-neutral — it can neither set
    /// nor clear a pin. A session the map has never seen has no live value to
    /// reconcile against, so its snapshot carries through as-is.
    pub async fn update(&self, mut session: Session) {
        let row = {
            let mut sessions = self.sessions.write().await;
            if let Some(live) = sessions.get(&session.id) {
                match live.metadata.get(CHAT_MODEL_KEY) {
                    Some(pin) => {
                        session
                            .metadata
                            .insert(CHAT_MODEL_KEY.to_string(), pin.clone());
                    }
                    None => {
                        session.metadata.remove(CHAT_MODEL_KEY);
                    }
                }
            }
            let row = SessionRow::from(&session);
            sessions.insert(session.id.clone(), session);
            row
        };
        self.persist_row(&row).await;
    }
    
    /// Delete a session
    pub async fn delete(&self, id: &str) -> bool {
        let mut sessions = self.sessions.write().await;
        let removed = sessions.remove(id).is_some();

        if removed {
            // Delete from DB
            if let Some(ref storage) = self.storage {
                if let Err(e) = storage.delete_daemon_session(id).await {
                    warn!("Failed to delete session {} from DB: {}", id, e);
                }
            }
            // Clear default if it was this session
            let mut default = self.default_session.write().await;
            if default.as_deref() == Some(id) {
                *default = sessions.keys().next().cloned();
            }
            info!("Deleted session: {}", id);
        }

        removed
    }

    /// Delete all sessions
    pub async fn delete_all(&self) -> usize {
        let mut sessions = self.sessions.write().await;
        let count = sessions.len();

        // Delete all from DB
        if let Some(ref storage) = self.storage {
            for id in sessions.keys() {
                if let Err(e) = storage.delete_daemon_session(id).await {
                    warn!("Failed to delete session {} from DB: {}", id, e);
                }
            }
        }

        sessions.clear();

        // Clear default session
        let mut default = self.default_session.write().await;
        *default = None;

        info!("Deleted all {} sessions", count);
        count
    }

    /// Rename a session
    pub async fn rename(&self, id: &str, name: String) -> bool {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(id) {
            session.name = Some(name.clone());
            session.updated_at = Utc::now();
            // Persist to DB
            if let Some(ref storage) = self.storage {
                if let Err(e) = storage.rename_daemon_session(id, &name).await {
                    warn!("Failed to persist rename for session {}: {}", id, e);
                }
            }
            true
        } else {
            false
        }
    }
    
    /// Clear a session's messages
    pub async fn clear(&self, id: &str) -> bool {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(id) {
            session.clear();
            // Clear from DB
            if let Some(ref storage) = self.storage {
                if let Err(e) = storage.clear_daemon_session_messages(id).await {
                    warn!("Failed to clear messages for session {} in DB: {}", id, e);
                }
            }
            true
        } else {
            false
        }
    }
    
    /// Add a message to a session (with write-through to DB)
    pub async fn add_message(&self, session_id: &str, role: MessageRole, content: impl Into<String>) -> Option<String> {
        let content = content.into();
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            let msg_id = session.add_message(role, content);
            // Persist the new message synchronously
            if let Some(msg) = session.messages.last() {
                self.persist_message(session_id, msg).await;
            }
            Some(msg_id)
        } else {
            None
        }
    }

    /// Add a message with tool calls, reasoning, run timeline, and usage totals to a session (with write-through to DB)
    pub async fn add_full_message(
        &self,
        session_id: &str,
        role: MessageRole,
        content: impl Into<String>,
        tool_calls: Vec<ToolCallRecord>,
        reasoning: Option<String>,
        timeline: Vec<TimelineItem>,
        usage: Option<RunUsage>,
    ) -> Option<String> {
        let content = content.into();
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            let msg_id = session.add_full_message(role, content, tool_calls, reasoning, timeline, usage);
            // Persist the new message synchronously
            if let Some(msg) = session.messages.last() {
                self.persist_message(session_id, msg).await;
            }
            Some(msg_id)
        } else {
            None
        }
    }
    
    /// Subscribe a channel to a session
    pub async fn subscribe(&self, session_id: &str, channel_id: ChannelId) -> bool {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            session.subscribe(channel_id);
            true
        } else {
            false
        }
    }
    
    /// Unsubscribe a channel from a session
    pub async fn unsubscribe(&self, session_id: &str, channel_id: &str) -> bool {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            session.unsubscribe(channel_id);
            true
        } else {
            false
        }
    }
    
    /// Get all sessions a channel is subscribed to
    pub async fn get_subscriptions(&self, channel_id: &str) -> Vec<SessionId> {
        let sessions = self.sessions.read().await;
        sessions.values()
            .filter(|s| s.is_subscribed(channel_id))
            .map(|s| s.id.clone())
            .collect()
    }
    
    /// Get subscribers for a session
    pub async fn get_subscribers(&self, session_id: &str) -> Vec<ChannelId> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id)
            .map(|s| s.subscribers.iter().cloned().collect())
            .unwrap_or_default()
    }
    
    /// Get session count
    pub async fn count(&self) -> usize {
        self.sessions.read().await.len()
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    /// Restore a session from persistence (used during startup / migration)
    pub async fn restore(&self, session: Session) {
        let id = session.id.clone();
        // Persist to DB if storage is available (for migration from JSON)
        self.persist_session(&session).await;
        // Also persist all messages
        for msg in &session.messages {
            self.persist_message(&id, msg).await;
        }

        let mut sessions = self.sessions.write().await;
        sessions.insert(id.clone(), session);
        
        // Set as default if it's the first session
        let mut default = self.default_session.write().await;
        if default.is_none() {
            *default = Some(id);
        }
    }
    
    /// Set the default session ID
    pub async fn set_default(&self, id: &str) {
        let sessions = self.sessions.read().await;
        if sessions.contains_key(id) {
            let mut default = self.default_session.write().await;
            *default = Some(id.to_string());
        }
    }
    
    /// Get the internal sessions map (for legacy code that needs it)
    pub fn sessions_map(&self) -> Arc<RwLock<HashMap<SessionId, Session>>> {
        self.sessions.clone()
    }
    
    /// Get the default session ID holder
    pub fn default_session_id(&self) -> Arc<RwLock<Option<SessionId>>> {
        self.default_session.clone()
    }
}

impl SessionManager {
    // =========================================================================
    // Sub-Session Management (#72)
    // =========================================================================

    /// Register a new sub-session (called after creating the Session)
    pub async fn register_sub_session(&self, info: SubSessionInfo) {
        let id = info.session_id.clone();
        info!("Registered sub-session: {} (parent: {:?}, label: {:?})", id, info.parent_id, info.label);
        self.sub_sessions.write().await.insert(id.clone(), info);
        // Initialize empty mailbox
        self.mailboxes.write().await.entry(id).or_default();
    }

    /// Update sub-session state
    pub async fn set_sub_session_state(&self, session_id: &str, state: SubSessionState) {
        let mut subs = self.sub_sessions.write().await;
        if let Some(info) = subs.get_mut(session_id) {
            info.state = state;
            if matches!(state, SubSessionState::Completed | SubSessionState::Failed | SubSessionState::Killed) {
                info.finished_at = Some(Utc::now());
            }
        }
    }

    /// Set sub-session result (on completion)
    pub async fn set_sub_session_result(&self, session_id: &str, result: String) {
        let mut subs = self.sub_sessions.write().await;
        if let Some(info) = subs.get_mut(session_id) {
            info.result = Some(result);
            info.state = SubSessionState::Completed;
            info.finished_at = Some(Utc::now());
        }
    }

    /// Set sub-session error (on failure)
    pub async fn set_sub_session_error(&self, session_id: &str, error: String) {
        let mut subs = self.sub_sessions.write().await;
        if let Some(info) = subs.get_mut(session_id) {
            info.error = Some(error);
            info.state = SubSessionState::Failed;
            info.finished_at = Some(Utc::now());
        }
    }

    /// Get sub-session info
    pub async fn get_sub_session(&self, session_id: &str) -> Option<SubSessionInfo> {
        self.sub_sessions.read().await.get(session_id).cloned()
    }

    /// Find a sub-session by label
    pub async fn find_sub_session_by_label(&self, label: &str) -> Option<SubSessionInfo> {
        self.sub_sessions.read().await.values()
            .find(|s| s.label.as_deref() == Some(label))
            .cloned()
    }

    /// Resolve a sub-session target (label or ID) to a SubSessionInfo
    pub async fn resolve_sub_session(&self, target: &str) -> Option<SubSessionInfo> {
        let subs = self.sub_sessions.read().await;
        if let Some(info) = subs.get(target) {
            return Some(info.clone());
        }
        subs.values()
            .find(|s| s.label.as_deref() == Some(target))
            .cloned()
    }

    /// List sub-sessions, optionally filtered by parent
    pub async fn list_sub_sessions(&self, parent_id: Option<&str>) -> Vec<SubSessionInfo> {
        let subs = self.sub_sessions.read().await;
        subs.values()
            .filter(|s| match parent_id {
                Some(pid) => s.parent_id.as_deref() == Some(pid),
                None => true,
            })
            .cloned()
            .collect()
    }

    /// Kill a sub-session (set cancellation flag + state)
    pub async fn kill_sub_session(&self, session_id: &str) -> bool {
        let mut subs = self.sub_sessions.write().await;
        if let Some(info) = subs.get_mut(session_id) {
            // Signal cancellation
            if let Some(ref flag) = info.cancellation_flag {
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            info.state = SubSessionState::Killed;
            info.finished_at = Some(Utc::now());
            info!("Killed sub-session: {}", session_id);
            true
        } else {
            false
        }
    }

    /// Send a message to a session's mailbox
    pub async fn send_to_mailbox(&self, session_id: &str, from: &str, content: String) -> bool {
        let mut mailboxes = self.mailboxes.write().await;
        if let Some(mailbox) = mailboxes.get_mut(session_id) {
            mailbox.push(MailboxMessage {
                from: from.to_string(),
                content,
                timestamp: Utc::now(),
            });
            true
        } else {
            false
        }
    }

    /// Drain all messages from a session's mailbox
    pub async fn drain_mailbox(&self, session_id: &str) -> Vec<MailboxMessage> {
        let mut mailboxes = self.mailboxes.write().await;
        mailboxes.get_mut(session_id)
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// Peek at a session's mailbox **without** consuming it (returns a clone).
    /// Used for status/inspection so checking a mailbox never eats pending
    /// inter-session messages the way `drain_mailbox` does.
    pub async fn peek_mailbox(&self, session_id: &str) -> Vec<MailboxMessage> {
        let mailboxes = self.mailboxes.read().await;
        mailboxes.get(session_id).cloned().unwrap_or_default()
    }

    /// Clean up completed/failed/killed sub-sessions older than the given duration
    pub async fn cleanup_sub_sessions(&self, max_age: std::time::Duration) {
        let cutoff = Utc::now() - chrono::Duration::from_std(max_age).unwrap_or(chrono::Duration::hours(24));
        let mut subs = self.sub_sessions.write().await;
        let mut mailboxes = self.mailboxes.write().await;

        let to_remove: Vec<String> = subs.iter()
            .filter(|(_, info)| {
                matches!(info.state, SubSessionState::Completed | SubSessionState::Failed | SubSessionState::Killed)
                    && info.finished_at.map(|t| t < cutoff).unwrap_or(false)
            })
            .map(|(id, _)| id.clone())
            .collect();

        for id in &to_remove {
            subs.remove(id);
            mailboxes.remove(id);
        }

        if !to_remove.is_empty() {
            info!("Cleaned up {} completed sub-sessions", to_remove.len());
        }
    }
}

/// Largest byte index at or below `max` that `s` may be sliced at.
///
/// A `SessionId` is a plain `String`, not a validated UUID: `with_id`,
/// restored legacy sessions and rows read back from the database all carry
/// whatever text their source held. Slicing one at a fixed byte index panics
/// when the id is shorter than the limit or when the index lands inside a
/// multi-byte character, and a panic here would take the whole daemon down.
fn floor_boundary(s: &str, max: usize) -> usize {
    if s.len() <= max {
        return s.len();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_last_user_turn_drops_reply_and_returns_content() {
        let mut s = Session::new(None);
        s.add_message(MessageRole::User, "hi");
        s.add_message(MessageRole::Assistant, "hello");

        assert_eq!(s.take_last_user_turn().as_deref(), Some("hi"));
        assert!(s.messages.is_empty(), "user msg + reply both removed");
    }

    #[test]
    fn take_last_user_turn_preserves_prior_history() {
        let mut s = Session::new(None);
        s.add_message(MessageRole::User, "q1");
        s.add_message(MessageRole::Assistant, "a1");
        s.add_message(MessageRole::User, "q2");
        s.add_message(MessageRole::Assistant, "a2");

        assert_eq!(s.take_last_user_turn().as_deref(), Some("q2"));
        // The earlier completed turn stays; only the latest turn is peeled off.
        assert_eq!(s.messages.len(), 2);
        assert!(matches!(s.messages[0].role, MessageRole::User));
        assert_eq!(s.messages[0].content, "q1");
        assert!(matches!(s.messages[1].role, MessageRole::Assistant));
    }

    #[test]
    fn take_last_user_turn_drops_trailing_tool_turns() {
        let mut s = Session::new(None);
        s.add_message(MessageRole::System, "sys");
        s.add_message(MessageRole::User, "do it");
        s.add_message(MessageRole::Assistant, "working");
        s.add_message(MessageRole::Tool, "tool output");

        assert_eq!(s.take_last_user_turn().as_deref(), Some("do it"));
        // Everything from the user message onward is gone; the system msg stays.
        assert_eq!(s.messages.len(), 1);
        assert!(matches!(s.messages[0].role, MessageRole::System));
    }

    #[test]
    fn take_last_user_turn_none_when_no_user_message() {
        let mut s = Session::new(None);
        s.add_message(MessageRole::System, "sys");
        s.add_message(MessageRole::Assistant, "greeting");
        let before = s.messages.len();

        assert!(s.take_last_user_turn().is_none());
        assert_eq!(s.messages.len(), before, "session left unchanged");
    }

    /// REGRESSION: the display name previews the first 8 bytes of the id, and
    /// an id is any string a caller hands us. When byte 8 falls inside a
    /// multi-byte character the raw slice panics — the same defect that killed
    /// the daemon mid-run from a preview elsewhere.
    #[test]
    fn display_name_clamps_multibyte_id_to_char_boundary() {
        // "session" is 7 bytes and the em dash occupies bytes 7..10, so the
        // 8-byte limit lands inside it.
        let s = Session::with_id("session\u{2014}id", None);

        assert_eq!(s.display_name(), "Session session");
    }

    /// REGRESSION: legacy sessions restored from disk carry short ids like
    /// "main", which the fixed-index slice ran off the end of.
    #[test]
    fn display_name_survives_id_shorter_than_the_preview() {
        let s = Session::with_id("main", None);

        assert_eq!(s.display_name(), "Session main");
    }

    /// REGRESSION (P19): a run's tool calls are first-class citizens of the
    /// chat — they must survive not just navigation (see
    /// `tasks::tests::tool_calls_survive_navigation_via_run_buffers`) but a
    /// full daemon restart. The timeline journal persisted with the message
    /// must round-trip through Turso intact: a fresh SessionManager over the
    /// same database restores the message with its tool call, input, output
    /// and verdict in place.
    #[tokio::test]
    async fn a_runs_tool_calls_survive_daemon_restart() {
        let storage = Arc::new(nanna_storage::Storage::in_memory().await.expect("storage"));
        let manager = SessionManager::with_storage(storage.clone());
        let session = manager.create(Some("restart-proof".to_string())).await;

        let at = Utc::now().to_rfc3339();
        let timeline = vec![
            TimelineItem::Thinking {
                content: "which files exist?".to_string(),
                at: at.clone(),
            },
            TimelineItem::Tool {
                call_id: "c1".to_string(),
                name: "exec".to_string(),
                input: Some(serde_json::json!({"cmd": "ls"})),
                output: Some("file.txt".to_string()),
                success: Some(true),
                duration_ms: Some(12),
                tokens: None,
                total_tokens: None,
                at: at.clone(),
            },
            TimelineItem::Text {
                content: "there is one file: file.txt".to_string(),
                at,
            },
        ];
        manager
            .add_full_message(
                &session.id,
                MessageRole::Assistant,
                "there is one file: file.txt",
                Vec::new(),
                None,
                timeline,
                None,
            )
            .await;

        // "Restart": a brand-new manager over the same database, exactly what
        // daemon startup does via load_from_db.
        let reborn = SessionManager::with_storage(storage);
        assert!(reborn.load_from_db().await >= 1, "session loads from Turso");

        let restored = reborn.get(&session.id).await.expect("session survives restart");
        let msg = restored
            .messages
            .last()
            .expect("assistant message survives restart");
        assert_eq!(msg.content, "there is one file: file.txt");
        assert_eq!(msg.timeline.len(), 3, "the full journal round-trips");
        assert!(matches!(
            &msg.timeline[1],
            TimelineItem::Tool {
                call_id,
                input: Some(input),
                output: Some(output),
                success: Some(true),
                duration_ms: Some(12),
                ..
            } if call_id == "c1" && input["cmd"] == "ls" && output == "file.txt"
        ));
    }

    /// A session that never picked a model must be indistinguishable from one
    /// that existed before per-chat models did: no key, no override, and a
    /// listing that says nothing about a model.
    #[tokio::test]
    async fn an_unpicked_session_reports_no_chat_model() {
        let manager = SessionManager::new();
        let session = manager.create(None).await;

        assert_eq!(session.chat_model(), None);
        assert!(session.metadata.is_empty(), "no key is the unset state");
        assert_eq!(SessionSummary::from(&session).chat_model, None);
    }

    #[tokio::test]
    async fn a_chat_model_pick_shows_up_on_the_session_and_in_the_listing() {
        let manager = SessionManager::new();
        let session = manager.create(None).await;

        assert!(
            manager
                .set_chat_model(&session.id, Some("ollama/qwen3:14b".to_string()))
                .await
        );

        // Stored verbatim in router form, so the turn can hand it straight to
        // the router without re-deriving a provider prefix.
        let picked = manager.get(&session.id).await.expect("session");
        assert_eq!(picked.chat_model(), Some("ollama/qwen3:14b"));

        // `session.list` is what a client hydrates from; without this it would
        // need a `session.get` per row just to render the pin.
        let listed = manager.list().await;
        let summary = listed
            .iter()
            .find(|s| s.id == session.id)
            .expect("session is listed");
        assert_eq!(summary.chat_model.as_deref(), Some("ollama/qwen3:14b"));
    }

    /// A blank pick is a CLEAR, normalized here so every reader downstream sees
    /// one shape for "no override". Pinning the empty string would instead
    /// produce a chat no provider serves.
    #[tokio::test]
    async fn a_blank_chat_model_pick_clears_the_pin() {
        let manager = SessionManager::new();
        let session = manager.create(None).await;

        manager
            .set_chat_model(&session.id, Some("ollama/qwen3:14b".to_string()))
            .await;
        manager.set_chat_model(&session.id, Some("   ".to_string())).await;

        let cleared = manager.get(&session.id).await.expect("session");
        assert_eq!(cleared.chat_model(), None);
        assert!(!cleared.metadata.contains_key(CHAT_MODEL_KEY), "the key is removed, not blanked");
    }

    #[tokio::test]
    async fn setting_a_chat_model_on_an_unknown_session_reports_failure() {
        let manager = SessionManager::new();

        assert!(
            !manager
                .set_chat_model("no-such-session", Some("ollama/qwen3:14b".to_string()))
                .await
        );
    }

    #[tokio::test]
    async fn a_chat_model_pick_survives_a_daemon_restart() {
        let storage = Arc::new(nanna_storage::Storage::in_memory().await.expect("storage"));
        let manager = SessionManager::with_storage(storage.clone());
        let session = manager.create(Some("pinned".to_string())).await;

        manager
            .set_chat_model(&session.id, Some("ollama/qwen3:14b".to_string()))
            .await;

        let reborn = SessionManager::with_storage(storage);
        assert!(reborn.load_from_db().await >= 1, "session loads from Turso");
        let restored = reborn.get(&session.id).await.expect("session survives restart");

        assert_eq!(restored.chat_model(), Some("ollama/qwen3:14b"));
    }

    /// REGRESSION (the COALESCE trap): `upsert_daemon_session` writes
    /// `metadata = COALESCE(?6, metadata)`, so persisting an empty map as NULL
    /// leaves the old blob on disk and `load_from_db` resurrects it. Un-pinning
    /// a chat would then appear to work right up until the next daemon start
    /// put the pin back — a silent lie about which model the chat runs on.
    #[tokio::test]
    async fn clearing_a_chat_model_pick_survives_a_daemon_restart() {
        let storage = Arc::new(nanna_storage::Storage::in_memory().await.expect("storage"));
        let manager = SessionManager::with_storage(storage.clone());
        let session = manager.create(Some("un-pinned".to_string())).await;

        manager
            .set_chat_model(&session.id, Some("ollama/qwen3:14b".to_string()))
            .await;
        manager.set_chat_model(&session.id, None).await;

        let reborn = SessionManager::with_storage(storage);
        assert!(reborn.load_from_db().await >= 1, "session loads from Turso");
        let restored = reborn.get(&session.id).await.expect("session survives restart");

        assert_eq!(
            restored.chat_model(),
            None,
            "the clear must land on disk, not just in the hot cache"
        );
    }

    /// The metadata blob is shared with whatever else a session records, so the
    /// replacing write has to carry the siblings through — both when the key is
    /// added and when it is removed.
    #[tokio::test]
    async fn a_chat_model_pick_leaves_the_rest_of_the_metadata_alone() {
        let storage = Arc::new(nanna_storage::Storage::in_memory().await.expect("storage"));
        let manager = SessionManager::with_storage(storage.clone());
        let mut session = manager.create(None).await;
        session
            .metadata
            .insert("source".to_string(), serde_json::Value::String("telegram".to_string()));
        manager.update(session.clone()).await;

        manager
            .set_chat_model(&session.id, Some("ollama/qwen3:14b".to_string()))
            .await;
        manager.set_chat_model(&session.id, None).await;

        let reborn = SessionManager::with_storage(storage);
        reborn.load_from_db().await;
        let restored = reborn.get(&session.id).await.expect("session survives restart");

        assert_eq!(restored.chat_model(), None);
        assert_eq!(
            restored.metadata.get("source").and_then(serde_json::Value::as_str),
            Some("telegram"),
            "removing the pin must not take the neighbours with it"
        );
    }

    /// The chat-model pin as the DATABASE holds it, read past the hot cache —
    /// the only way to tell "written" from "merely remembered".
    async fn pinned_model_on_disk(
        storage: &nanna_storage::Storage,
        session_id: &str,
    ) -> Option<String> {
        storage
            .list_daemon_sessions()
            .await
            .expect("sessions load")
            .into_iter()
            .find(|row| row.session_id == session_id)
            .and_then(|row| row.metadata)
            .and_then(|meta| {
                meta.get(CHAT_MODEL_KEY)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
    }

    /// REGRESSION: a model pick used to hold the `sessions` WRITE guard across
    /// its database round trip, parking every concurrent `get`/`list` behind a
    /// disk write — the turn resolving this very pin among them. The pick and
    /// the write are now separate steps: `apply_chat_model` takes the guard,
    /// mutates, and gives it back, so by the time the row exists the map is
    /// readable, writable, and already telling the truth — with the database
    /// still untouched.
    #[tokio::test]
    async fn a_chat_model_pick_lands_in_memory_before_the_database_write() {
        let storage = Arc::new(nanna_storage::Storage::in_memory().await.expect("storage"));
        let manager = SessionManager::with_storage(storage.clone());
        let session = manager.create(None).await;

        let row = manager
            .apply_chat_model(&session.id, Some("ollama/qwen3:14b".to_string()))
            .await
            .expect("session exists");

        assert!(
            manager.sessions_map().try_write().is_ok(),
            "the guard is released before the write, not after it"
        );
        assert_eq!(
            manager.get(&session.id).await.expect("session").chat_model(),
            Some("ollama/qwen3:14b"),
            "readers see the pick without waiting on storage"
        );
        assert_eq!(
            pinned_model_on_disk(&storage, &session.id).await,
            None,
            "and all of that happened before the database was touched"
        );

        manager.persist_row(&row).await;
        assert_eq!(
            pinned_model_on_disk(&storage, &session.id).await.as_deref(),
            Some("ollama/qwen3:14b"),
            "the row the mutation handed back is what makes the pick durable"
        );
    }

    /// REGRESSION: `ChatAction::Regenerate` reads a session, peels the last turn
    /// off its copy, and writes that copy back. A `session.setModel` landing
    /// inside that window used to be undone — the snapshot still carried the old
    /// pin and put it straight back, so the chat kept running on a model the
    /// picker said it had stopped using.
    #[tokio::test]
    async fn update_cannot_resurrect_a_pin_cleared_after_its_snapshot() {
        let storage = Arc::new(nanna_storage::Storage::in_memory().await.expect("storage"));
        let manager = SessionManager::with_storage(storage.clone());
        let session = manager.create(Some("regenerating".to_string())).await;
        manager
            .set_chat_model(&session.id, Some("ollama/qwen3:14b".to_string()))
            .await;

        // The snapshot a regenerate is holding, taken while the pin was set.
        let mut snapshot = manager.get(&session.id).await.expect("session");
        assert_eq!(snapshot.chat_model(), Some("ollama/qwen3:14b"));

        // The user un-pins while the regenerate is still assembling.
        manager.set_chat_model(&session.id, None).await;

        // ...and the regenerate writes its snapshot back.
        snapshot.add_message(MessageRole::User, "again");
        manager.update(snapshot).await;

        let live = manager.get(&session.id).await.expect("session");
        assert_eq!(live.chat_model(), None, "the clear survives the write-back");
        assert_eq!(live.messages.len(), 1, "the caller's own edit still lands");

        let reborn = SessionManager::with_storage(storage);
        reborn.load_from_db().await;
        let restored = reborn.get(&session.id).await.expect("session survives restart");
        assert_eq!(
            restored.chat_model(),
            None,
            "and a restart does not bring the resurrected pin back"
        );
    }

    /// The mirror of the resurrection case: a pin set after the snapshot was
    /// taken must not be dropped by the write-back either. `update` is
    /// pin-neutral — [`SessionManager::set_chat_model`] is the only writer of
    /// that key.
    #[tokio::test]
    async fn update_cannot_drop_a_pin_set_after_its_snapshot() {
        let storage = Arc::new(nanna_storage::Storage::in_memory().await.expect("storage"));
        let manager = SessionManager::with_storage(storage.clone());
        let session = manager.create(Some("regenerating".to_string())).await;

        let mut snapshot = manager.get(&session.id).await.expect("session");
        assert_eq!(snapshot.chat_model(), None);

        manager
            .set_chat_model(&session.id, Some("ollama/qwen3:14b".to_string()))
            .await;

        snapshot.add_message(MessageRole::User, "again");
        manager.update(snapshot).await;

        let live = manager.get(&session.id).await.expect("session");
        assert_eq!(live.chat_model(), Some("ollama/qwen3:14b"));

        let reborn = SessionManager::with_storage(storage);
        reborn.load_from_db().await;
        let restored = reborn.get(&session.id).await.expect("session survives restart");
        assert_eq!(restored.chat_model(), Some("ollama/qwen3:14b"));
    }

    #[tokio::test]
    async fn test_peek_mailbox_is_non_destructive() {
        let manager = SessionManager::new();
        let info = SubSessionInfo {
            session_id: "sub-1".to_string(),
            parent_id: None,
            label: None,
            task: "t".to_string(),
            state: SubSessionState::Running,
            spawned_at: Utc::now(),
            finished_at: None,
            model: None,
            result: None,
            error: None,
            cancellation_flag: None,
        };
        manager.register_sub_session(info).await;

        assert!(
            manager
                .send_to_mailbox("sub-1", "parent", "hi".to_string())
                .await
        );
        assert!(
            manager
                .send_to_mailbox("sub-1", "parent", "again".to_string())
                .await
        );

        // Peeking twice returns the same messages without consuming them.
        assert_eq!(manager.peek_mailbox("sub-1").await.len(), 2);
        assert_eq!(manager.peek_mailbox("sub-1").await.len(), 2);

        // Draining consumes; a subsequent peek is empty.
        assert_eq!(manager.drain_mailbox("sub-1").await.len(), 2);
        assert_eq!(manager.peek_mailbox("sub-1").await.len(), 0);

        // Peeking an unknown session yields an empty vec (no panic).
        assert!(manager.peek_mailbox("nope").await.is_empty());
    }

    #[tokio::test]
    async fn test_session_manager() {
        let manager = SessionManager::new();
        
        // Create session
        let session = manager.create(Some("Test".to_string())).await;
        assert_eq!(session.name, Some("Test".to_string()));
        
        // Get session
        let retrieved = manager.get(&session.id).await.unwrap();
        assert_eq!(retrieved.id, session.id);
        
        // List sessions
        let list = manager.list().await;
        assert_eq!(list.len(), 1);
        
        // Subscribe channel
        manager.subscribe(&session.id, "gui:123".to_string()).await;
        let subs = manager.get_subscribers(&session.id).await;
        assert!(subs.contains(&"gui:123".to_string()));
        
        // Delete session
        manager.delete(&session.id).await;
        assert!(manager.get(&session.id).await.is_none());
    }
}
