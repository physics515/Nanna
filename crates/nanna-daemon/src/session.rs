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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentRecord {
    pub id: String,
    pub filename: String,
    pub content_type: String,
    pub url: Option<String>,
}

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
        });
        self.updated_at = Utc::now();
        id
    }

    /// Add a message with tool calls and reasoning to the session (in-memory only)
    pub fn add_full_message(
        &mut self,
        role: MessageRole,
        content: impl Into<String>,
        tool_calls: Vec<ToolCallRecord>,
        reasoning: Option<String>,
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
    
    /// Get display name (name or truncated ID)
    pub fn display_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            format!("Session {}", &self.id[..8])
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

/// Serialize a SessionMessage's extra fields (tool_calls, attachments, reasoning) to JSON metadata.
fn message_to_metadata(msg: &SessionMessage) -> Option<String> {
    let has_tool_calls = !msg.tool_calls.is_empty();
    let has_attachments = !msg.attachments.is_empty();
    let has_reasoning = msg.reasoning.is_some();

    if !has_tool_calls && !has_attachments && !has_reasoning {
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
        let Some(ref storage) = self.storage else {
            warn!("persist_session called but no storage backend — session {} will not be persisted", session.id);
            return;
        };
        let created = session.created_at.to_rfc3339();
        let updated = session.updated_at.to_rfc3339();
        let metadata = if session.metadata.is_empty() {
            None
        } else {
            serde_json::to_string(&session.metadata).ok()
        };
        match storage.upsert_daemon_session(
            &session.id,
            session.name.as_deref(),
            session.workspace_id.as_deref(),
            &created,
            &updated,
            metadata.as_deref(),
        ).await {
            Ok(()) => info!("Persisted session {} to database", session.id),
            Err(e) => warn!("Failed to persist session {} to database: {}", session.id, e),
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
    
    /// Update a session
    pub async fn update(&self, session: Session) {
        self.persist_session(&session).await;
        let mut sessions = self.sessions.write().await;
        sessions.insert(session.id.clone(), session);
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

    /// Add a message with tool calls and reasoning to a session (with write-through to DB)
    pub async fn add_full_message(
        &self,
        session_id: &str,
        role: MessageRole,
        content: impl Into<String>,
        tool_calls: Vec<ToolCallRecord>,
        reasoning: Option<String>,
    ) -> Option<String> {
        let content = content.into();
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            let msg_id = session.add_full_message(role, content, tool_calls, reasoning);
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
            .map(|mb| std::mem::take(mb))
            .unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;
    
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
