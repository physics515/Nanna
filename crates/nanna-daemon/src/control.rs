//! Control Plane - Handles all control actions from channels
//!
//! Every channel (GUI, Telegram, CLI, etc.) has full access to:
//! - Session management
//! - Memory browsing/editing
//! - Configuration
//! - Tool management  
//! - Scheduler/cron
//! - System operations

use crate::protocol::*;
use crate::session::{SessionManager, MessageRole};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// The control plane provides unified access to all daemon functionality
pub struct ControlPlane {
    sessions: Arc<SessionManager>,
    // TODO: Add other managers
    // memory: Arc<MemoryManager>,
    // config: Arc<ConfigManager>,
    // tools: Arc<ToolManager>,
    // scheduler: Arc<SchedulerManager>,
    // channels: Arc<ChannelManager>,
}

impl ControlPlane {
    /// Create a new control plane
    pub fn new(sessions: Arc<SessionManager>) -> Self {
        Self { sessions }
    }
    
    /// Handle an action and return a response
    pub async fn handle(&self, client_id: &str, action: Action) -> Value {
        match action {
            Action::Chat(chat) => self.handle_chat(client_id, chat).await,
            Action::Session(session) => self.handle_session(client_id, session).await,
            Action::Memory(memory) => self.handle_memory(client_id, memory).await,
            Action::Config(config) => self.handle_config(client_id, config).await,
            Action::Tool(tool) => self.handle_tool(client_id, tool).await,
            Action::Scheduler(scheduler) => self.handle_scheduler(client_id, scheduler).await,
            Action::Channel(channel) => self.handle_channel(client_id, channel).await,
            Action::System(system) => self.handle_system(client_id, system).await,
            Action::Subscribe(sub) => self.handle_subscribe(client_id, sub).await,
            Action::Unsubscribe(unsub) => self.handle_unsubscribe(client_id, unsub).await,
        }
    }
    
    // =========================================================================
    // Chat Handlers
    // =========================================================================
    
    async fn handle_chat(&self, client_id: &str, action: ChatAction) -> Value {
        match action {
            ChatAction::Send { session_id, content, attachments: _ } => {
                debug!("Chat send from {} to session {}", client_id, session_id);
                
                // Add user message to session
                if let Some(msg_id) = self.sessions.add_message(&session_id, MessageRole::User, &content).await {
                    // TODO: Actually call the agent and stream response
                    // For now, just acknowledge receipt
                    json!({
                        "status": "accepted",
                        "message_id": msg_id,
                        "session_id": session_id
                    })
                } else {
                    json!({
                        "error": "session_not_found",
                        "message": format!("Session {} not found", session_id)
                    })
                }
            }
            ChatAction::Cancel { session_id } => {
                info!("Chat cancel for session {}", session_id);
                json!({ "status": "cancelled", "session_id": session_id })
            }
            ChatAction::Regenerate { session_id } => {
                info!("Chat regenerate for session {}", session_id);
                json!({ "status": "regenerating", "session_id": session_id })
            }
        }
    }
    
    // =========================================================================
    // Session Handlers
    // =========================================================================
    
    async fn handle_session(&self, client_id: &str, action: SessionAction) -> Value {
        match action {
            SessionAction::List => {
                let sessions = self.sessions.list().await;
                json!({ "sessions": sessions })
            }
            SessionAction::Get { id } => {
                if let Some(session) = self.sessions.get(&id).await {
                    json!({ "session": session })
                } else {
                    json!({ "error": "not_found", "message": format!("Session {} not found", id) })
                }
            }
            SessionAction::Create { name } => {
                let session = self.sessions.create(name).await;
                // Auto-subscribe the creating client
                self.sessions.subscribe(&session.id, client_id.to_string()).await;
                json!({ "session": session })
            }
            SessionAction::Rename { id, name } => {
                if self.sessions.rename(&id, name.clone()).await {
                    json!({ "status": "renamed", "id": id, "name": name })
                } else {
                    json!({ "error": "not_found", "message": format!("Session {} not found", id) })
                }
            }
            SessionAction::Delete { id } => {
                if self.sessions.delete(&id).await {
                    json!({ "status": "deleted", "id": id })
                } else {
                    json!({ "error": "not_found", "message": format!("Session {} not found", id) })
                }
            }
            SessionAction::Clear { id } => {
                if self.sessions.clear(&id).await {
                    json!({ "status": "cleared", "id": id })
                } else {
                    json!({ "error": "not_found", "message": format!("Session {} not found", id) })
                }
            }
            SessionAction::History { id, limit, before: _ } => {
                if let Some(session) = self.sessions.get(&id).await {
                    let messages: Vec<_> = session.messages.iter()
                        .rev()
                        .take(limit.unwrap_or(50))
                        .cloned()
                        .collect();
                    json!({ "messages": messages })
                } else {
                    json!({ "error": "not_found", "message": format!("Session {} not found", id) })
                }
            }
            SessionAction::Switch { id } => {
                if self.sessions.get(&id).await.is_some() {
                    // Subscribe client to this session
                    self.sessions.subscribe(&id, client_id.to_string()).await;
                    json!({ "status": "switched", "session_id": id })
                } else {
                    json!({ "error": "not_found", "message": format!("Session {} not found", id) })
                }
            }
            SessionAction::Fork { id, name } => {
                if let Some(original) = self.sessions.get(&id).await {
                    let mut forked = self.sessions.create(
                        name.or_else(|| original.name.as_ref().map(|n| format!("{} (copy)", n)))
                    ).await;
                    // Copy messages
                    forked.messages = original.messages.clone();
                    self.sessions.update(forked.clone()).await;
                    json!({ "session": forked })
                } else {
                    json!({ "error": "not_found", "message": format!("Session {} not found", id) })
                }
            }
        }
    }
    
    // =========================================================================
    // Memory Handlers
    // =========================================================================
    
    async fn handle_memory(&self, _client_id: &str, action: MemoryAction) -> Value {
        match action {
            MemoryAction::Search { query, limit } => {
                // TODO: Implement actual memory search
                warn!("Memory search not yet implemented");
                json!({ 
                    "memories": [],
                    "query": query,
                    "limit": limit.unwrap_or(10)
                })
            }
            MemoryAction::Get { id } => {
                json!({ "error": "not_implemented", "message": format!("Get memory {} not implemented", id) })
            }
            MemoryAction::Create { content, tags, importance } => {
                json!({ 
                    "error": "not_implemented", 
                    "message": "Create memory not implemented",
                    "content": content,
                    "tags": tags,
                    "importance": importance
                })
            }
            MemoryAction::Update { id, content, tags } => {
                json!({ 
                    "error": "not_implemented",
                    "message": format!("Update memory {} not implemented", id),
                    "content": content,
                    "tags": tags
                })
            }
            MemoryAction::Delete { id } => {
                json!({ "error": "not_implemented", "message": format!("Delete memory {} not implemented", id) })
            }
            MemoryAction::Stats => {
                json!({ 
                    "total": 0,
                    "by_source": {},
                    "by_importance": {}
                })
            }
            MemoryAction::Consolidate => {
                json!({ "status": "not_implemented", "message": "Consolidation not implemented" })
            }
        }
    }
    
    // =========================================================================
    // Config Handlers
    // =========================================================================
    
    async fn handle_config(&self, _client_id: &str, action: ConfigAction) -> Value {
        match action {
            ConfigAction::Get { path } => {
                // TODO: Implement config get
                json!({ 
                    "config": {},
                    "path": path
                })
            }
            ConfigAction::Set { path, value } => {
                json!({ 
                    "error": "not_implemented",
                    "path": path,
                    "value": value
                })
            }
            ConfigAction::Reset { path } => {
                json!({ "error": "not_implemented", "path": path })
            }
            ConfigAction::Reload => {
                json!({ "status": "not_implemented" })
            }
            ConfigAction::Export => {
                json!({ "config": {} })
            }
            ConfigAction::Import { config: _ } => {
                json!({ "status": "not_implemented" })
            }
        }
    }
    
    // =========================================================================
    // Tool Handlers
    // =========================================================================
    
    async fn handle_tool(&self, _client_id: &str, action: ToolAction) -> Value {
        match action {
            ToolAction::List => {
                // TODO: Get actual tool list
                json!({ "tools": [] })
            }
            ToolAction::Get { name } => {
                json!({ "error": "not_found", "name": name })
            }
            ToolAction::Enable { name } => {
                json!({ "status": "not_implemented", "name": name })
            }
            ToolAction::Disable { name } => {
                json!({ "status": "not_implemented", "name": name })
            }
            ToolAction::Execute { name, input } => {
                json!({ 
                    "error": "not_implemented",
                    "name": name,
                    "input": input
                })
            }
            ToolAction::Create { name, description, code, needs_shell } => {
                json!({ 
                    "error": "not_implemented",
                    "name": name,
                    "description": description,
                    "code": code,
                    "needs_shell": needs_shell
                })
            }
            ToolAction::Delete { name } => {
                json!({ "error": "not_implemented", "name": name })
            }
        }
    }
    
    // =========================================================================
    // Scheduler Handlers
    // =========================================================================
    
    async fn handle_scheduler(&self, _client_id: &str, action: SchedulerAction) -> Value {
        match action {
            SchedulerAction::List => {
                json!({ "jobs": [] })
            }
            SchedulerAction::Get { id } => {
                json!({ "error": "not_found", "id": id })
            }
            SchedulerAction::Add { schedule, task, name } => {
                json!({ 
                    "error": "not_implemented",
                    "schedule": schedule,
                    "task": task,
                    "name": name
                })
            }
            SchedulerAction::Update { id, schedule, task, enabled } => {
                json!({ 
                    "error": "not_implemented",
                    "id": id,
                    "schedule": schedule,
                    "task": task,
                    "enabled": enabled
                })
            }
            SchedulerAction::Remove { id } => {
                json!({ "error": "not_implemented", "id": id })
            }
            SchedulerAction::RunNow { id } => {
                json!({ "error": "not_implemented", "id": id })
            }
            SchedulerAction::History { id, limit } => {
                json!({ "error": "not_implemented", "id": id, "limit": limit })
            }
        }
    }
    
    // =========================================================================
    // Channel Handlers
    // =========================================================================
    
    async fn handle_channel(&self, _client_id: &str, action: ChannelAction) -> Value {
        match action {
            ChannelAction::List => {
                json!({ "channels": [] })
            }
            ChannelAction::Status { id } => {
                json!({ "error": "not_implemented", "id": id })
            }
            ChannelAction::Enable { id } => {
                json!({ "error": "not_implemented", "id": id })
            }
            ChannelAction::Disable { id } => {
                json!({ "error": "not_implemented", "id": id })
            }
            ChannelAction::Test { id } => {
                json!({ "error": "not_implemented", "id": id })
            }
            ChannelAction::Send { channel_id, target, content } => {
                json!({ 
                    "error": "not_implemented",
                    "channel_id": channel_id,
                    "target": target,
                    "content": content
                })
            }
        }
    }
    
    // =========================================================================
    // System Handlers
    // =========================================================================
    
    async fn handle_system(&self, _client_id: &str, action: SystemAction) -> Value {
        match action {
            SystemAction::Status => {
                json!({
                    "status": "running",
                    "version": env!("CARGO_PKG_VERSION"),
                    "uptime_secs": 0, // TODO: Track uptime
                    "sessions": self.sessions.count().await,
                    "memory_mb": 0,
                })
            }
            SystemAction::Restart => {
                info!("Restart requested");
                json!({ "status": "restarting" })
            }
            SystemAction::Shutdown => {
                info!("Shutdown requested");
                json!({ "status": "shutting_down" })
            }
            SystemAction::Version => {
                json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "name": "nanna-daemon",
                })
            }
            SystemAction::CheckUpdate => {
                json!({ "update_available": false })
            }
            SystemAction::Update => {
                json!({ "error": "not_implemented" })
            }
            SystemAction::Logs { lines, level } => {
                json!({ 
                    "logs": [],
                    "lines": lines.unwrap_or(100),
                    "level": level.unwrap_or_else(|| "info".to_string())
                })
            }
            SystemAction::Health => {
                json!({
                    "healthy": true,
                    "checks": {
                        "database": "ok",
                        "memory": "ok",
                        "llm": "ok"
                    }
                })
            }
        }
    }
    
    // =========================================================================
    // Subscription Handlers
    // =========================================================================
    
    async fn handle_subscribe(&self, client_id: &str, action: SubscribeAction) -> Value {
        match action {
            SubscribeAction::Session { session_id } => {
                if self.sessions.subscribe(&session_id, client_id.to_string()).await {
                    json!({ "status": "subscribed", "session_id": session_id })
                } else {
                    json!({ "error": "not_found", "session_id": session_id })
                }
            }
            SubscribeAction::AllSessions => {
                // TODO: Track global subscriptions
                json!({ "status": "subscribed", "topic": "all_sessions" })
            }
            SubscribeAction::ChannelStatus => {
                json!({ "status": "subscribed", "topic": "channel_status" })
            }
            SubscribeAction::System => {
                json!({ "status": "subscribed", "topic": "system" })
            }
        }
    }
    
    async fn handle_unsubscribe(&self, client_id: &str, action: UnsubscribeAction) -> Value {
        match action {
            UnsubscribeAction::Session { session_id } => {
                self.sessions.unsubscribe(&session_id, client_id).await;
                json!({ "status": "unsubscribed", "session_id": session_id })
            }
            UnsubscribeAction::AllSessions => {
                json!({ "status": "unsubscribed", "topic": "all_sessions" })
            }
            UnsubscribeAction::ChannelStatus => {
                json!({ "status": "unsubscribed", "topic": "channel_status" })
            }
            UnsubscribeAction::System => {
                json!({ "status": "unsubscribed", "topic": "system" })
            }
        }
    }
}
