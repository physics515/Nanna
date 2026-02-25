//! Session management
//!
//! Sessions represent conversations with the agent. Multiple channels
//! can subscribe to the same session.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

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
        }
    }
    
    /// Add a message to the session
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

    /// Add a message with tool calls and reasoning to the session
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
        }
    }
}

/// Manages all sessions
pub struct SessionManager {
    sessions: Arc<RwLock<HashMap<SessionId, Session>>>,
    /// Default session ID (for new clients)
    default_session: Arc<RwLock<Option<SessionId>>>,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            default_session: Arc::new(RwLock::new(None)),
        }
    }
    
    /// Create a session and return it
    pub async fn create(&self, name: Option<String>) -> Session {
        let session = Session::new(name);
        let id = session.id.clone();
        
        let mut sessions = self.sessions.write().await;
        sessions.insert(id.clone(), session.clone());
        
        // Set as default if it's the first session
        let mut default = self.default_session.write().await;
        if default.is_none() {
            *default = Some(id.clone());
        }
        
        info!("Created session: {}", id);
        session
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
        let mut sessions = self.sessions.write().await;
        sessions.insert(session.id.clone(), session);
    }
    
    /// Delete a session
    pub async fn delete(&self, id: &str) -> bool {
        let mut sessions = self.sessions.write().await;
        let removed = sessions.remove(id).is_some();

        if removed {
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
            session.name = Some(name);
            session.updated_at = Utc::now();
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
            true
        } else {
            false
        }
    }
    
    /// Add a message to a session
    pub async fn add_message(&self, session_id: &str, role: MessageRole, content: impl Into<String>) -> Option<String> {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            Some(session.add_message(role, content))
        } else {
            None
        }
    }

    /// Add a message with tool calls and reasoning to a session
    pub async fn add_full_message(
        &self,
        session_id: &str,
        role: MessageRole,
        content: impl Into<String>,
        tool_calls: Vec<ToolCallRecord>,
        reasoning: Option<String>,
    ) -> Option<String> {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            Some(session.add_full_message(role, content, tool_calls, reasoning))
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
    /// Restore a session from persistence (used during startup)
    pub async fn restore(&self, session: Session) {
        let id = session.id.clone();
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
    
    /// Get the internal sessions map (for persistence)
    pub fn sessions_map(&self) -> Arc<RwLock<HashMap<SessionId, Session>>> {
        self.sessions.clone()
    }
    
    /// Get the default session ID holder (for persistence)
    pub fn default_session_id(&self) -> Arc<RwLock<Option<SessionId>>> {
        self.default_session.clone()
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
