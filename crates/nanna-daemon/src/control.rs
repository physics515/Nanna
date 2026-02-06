//! Control Plane - Handles all control actions from channels
//!
//! Every channel (GUI, Telegram, CLI, etc.) has full access to:
//! - Session management
//! - Memory browsing/editing
//! - Configuration
//! - Tool management  
//! - Scheduler/cron
//! - Workspaces
//! - System operations

use crate::agent_service::AgentService;
use crate::llm_router::LlmRouter;
use crate::protocol::*;
use crate::session::{MessageRole, SessionManager};
use crate::user_tools::UserToolManager;
use nanna_config::Config;
use nanna_core::{Scheduler, WorkspaceRegistry, Workspace};
use nanna_llm::RequestBuilder;
use nanna_memory::{ConsolidationConfig, MemoryService};
use nanna_tools::ToolRegistry;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// The control plane provides unified access to all daemon functionality
#[allow(dead_code)]
pub struct ControlPlane {
    sessions: Arc<SessionManager>,
    agent: Option<Arc<AgentService>>,
    memory: Option<Arc<MemoryService>>,
    tools: Option<Arc<ToolRegistry>>,
    user_tools: Option<Arc<UserToolManager>>,
    router: Option<Arc<LlmRouter>>,
    scheduler: Option<Arc<RwLock<Scheduler>>>,
    workspaces: Arc<RwLock<WorkspaceRegistry>>,
    config: Arc<RwLock<Config>>,
    config_path: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    /// System prompt template
    system_prompt: Arc<RwLock<String>>,
}

impl ControlPlane {
    /// Create a new control plane with just sessions
    pub fn new(sessions: Arc<SessionManager>) -> Self {
        Self {
            sessions,
            agent: None,
            memory: None,
            tools: None,
            user_tools: None,
            router: None,
            scheduler: None,
            workspaces: Arc::new(RwLock::new(WorkspaceRegistry::new())),
            config: Arc::new(RwLock::new(Config::default())),
            config_path: None,
            data_dir: None,
            system_prompt: Arc::new(RwLock::new(default_system_prompt())),
        }
    }

    /// Create a control plane with full services
    pub fn with_services(
        sessions: Arc<SessionManager>,
        agent: Arc<AgentService>,
        memory: Option<Arc<MemoryService>>,
        tools: Option<Arc<ToolRegistry>>,
    ) -> Self {
        Self {
            sessions,
            agent: Some(agent),
            memory,
            tools,
            user_tools: None,
            router: None,
            scheduler: None,
            workspaces: Arc::new(RwLock::new(WorkspaceRegistry::new())),
            config: Arc::new(RwLock::new(Config::default())),
            config_path: None,
            data_dir: None,
            system_prompt: Arc::new(RwLock::new(default_system_prompt())),
        }
    }

    /// Create a control plane with all services including LLM router
    pub fn with_all_services(
        sessions: Arc<SessionManager>,
        agent: Arc<AgentService>,
        memory: Option<Arc<MemoryService>>,
        tools: Option<Arc<ToolRegistry>>,
        router: Option<Arc<LlmRouter>>,
    ) -> Self {
        // Load config from disk
        let (config, config_path, data_dir) = match Config::load() {
            Ok(cfg) => {
                // Try to get config path from nanna_config
                let data = nanna_config::Config::default_data_dir().ok();
                let path = data.as_ref().map(|d| d.join("config.toml"));
                (cfg.with_env_overrides(), path, data)
            }
            Err(_) => (Config::default().with_env_overrides(), None, None)
        };
        
        // Initialize user tools manager
        let user_tools = data_dir.as_ref().map(|d| {
            let tools_dir = d.join("user_tools");
            Arc::new(UserToolManager::new(tools_dir))
        });
        
        Self {
            sessions,
            agent: Some(agent),
            memory,
            tools,
            user_tools,
            router,
            scheduler: None,
            workspaces: Arc::new(RwLock::new(WorkspaceRegistry::new())),
            config: Arc::new(RwLock::new(config)),
            config_path,
            data_dir,
            system_prompt: Arc::new(RwLock::new(default_system_prompt())),
        }
    }

    /// Set the scheduler
    pub fn with_scheduler(mut self, scheduler: Arc<RwLock<Scheduler>>) -> Self {
        self.scheduler = Some(scheduler);
        self
    }
    
    /// Get a reference to the workspace registry
    pub fn workspaces(&self) -> &Arc<RwLock<WorkspaceRegistry>> {
        &self.workspaces
    }
    
    /// Get a reference to the scheduler
    pub fn scheduler(&self) -> Option<&Arc<RwLock<Scheduler>>> {
        self.scheduler.as_ref()
    }
    
    /// Get a reference to the user tool manager
    pub fn user_tools(&self) -> Option<&Arc<UserToolManager>> {
        self.user_tools.as_ref()
    }
    
    /// Load user tools and register them with the tool registry
    pub async fn load_user_tools(&self) -> Result<usize, String> {
        let Some(ref user_tools) = self.user_tools else {
            return Err("User tools manager not initialized".to_string());
        };
        
        // Load from disk
        let count = user_tools.load_all().await.map_err(|e| e.to_string())?;
        
        // Register with tool registry
        if let Some(ref tools) = self.tools {
            user_tools.register_with_registry(tools).await;
        }
        
        Ok(count)
    }
    
    /// Set the system prompt
    pub async fn set_system_prompt(&self, prompt: String) {
        let mut sp = self.system_prompt.write().await;
        *sp = prompt;
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
            Action::Workspace(workspace) => self.handle_workspace(client_id, workspace).await,
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
                let _msg_id = match self.sessions.add_message(&session_id, MessageRole::User, &content).await {
                    Some(id) => id,
                    None => return json!({
                        "error": "session_not_found",
                        "message": format!("Session {} not found", session_id)
                    }),
                };
                
                // Check if agent is available
                let Some(ref agent) = self.agent else {
                    return json!({
                        "error": "agent_unavailable",
                        "message": "Agent service not configured"
                    });
                };
                
                // Get session history (all messages *before* the one we just added)
                let session = match self.sessions.get(&session_id).await {
                    Some(s) => s,
                    None => return json!({
                        "error": "session_not_found",
                        "message": format!("Session {} not found", session_id)
                    }),
                };

                // Prior messages = everything except the last one (the user message we just added)
                let prior_messages: Vec<_> = if session.messages.len() > 1 {
                    session.messages[..session.messages.len() - 1].to_vec()
                } else {
                    Vec::new()
                };

                // Build system prompt with memory context
                let mut system_prompt = self.system_prompt.read().await.clone();

                // Add memory context if available
                let memories = agent.recall_memories(&content, 5).await;
                if !memories.is_empty() {
                    system_prompt.push_str("\n\n## Remembered Context\n");
                    for mem in memories {
                        system_prompt.push_str(&format!("- {}\n", mem.content));
                    }
                }

                // Run the agent with conversation history
                match agent.chat(&session_id, &content, Some(system_prompt), &prior_messages).await {
                    Ok(result) => {
                        // Add assistant response to session
                        self.sessions.add_message(&session_id, MessageRole::Assistant, &result.content).await;
                        
                        json!({
                            "status": "success",
                            "message_id": result.message_id,
                            "content": result.content,
                            "tool_calls": result.tool_calls,
                            "usage": {
                                "input_tokens": result.input_tokens,
                                "output_tokens": result.output_tokens
                            }
                        })
                    }
                    Err(e) => {
                        json!({
                            "error": "chat_failed",
                            "message": e
                        })
                    }
                }
            }
            ChatAction::Cancel { session_id } => {
                info!("Chat cancel for session {}", session_id);
                if let Some(ref agent) = self.agent {
                    let cancelled = agent.cancel(&session_id).await;
                    json!({ "status": if cancelled { "cancelled" } else { "not_active" }, "session_id": session_id })
                } else {
                    json!({ "error": "agent_unavailable" })
                }
            }
            ChatAction::Regenerate { session_id } => {
                info!("Chat regenerate for session {}", session_id);
                // TODO: Remove last assistant message and re-run
                json!({ "status": "not_implemented", "session_id": session_id })
            }
        }
    }
    
    // =========================================================================
    // Session Handlers
    // =========================================================================
    
    async fn handle_session(&self, client_id: &str, action: SessionAction) -> Value {
        match action {
            SessionAction::List => {
                let mut sessions = self.sessions.list().await;
                // Sort by created_at descending (newest first)
                sessions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
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
            SessionAction::DeleteAll => {
                let count = self.sessions.delete_all().await;
                json!({ "status": "deleted", "count": count })
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
                    // Get last N messages (reversed to get newest first), then reverse back to chronological
                    let mut messages: Vec<_> = session.messages.iter()
                        .rev()
                        .take(limit.unwrap_or(50))
                        .cloned()
                        .collect();
                    messages.reverse(); // Back to chronological order (oldest first)
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
            SessionAction::GetRunState { id } => {
                if let Some(ref agent) = self.agent {
                    let state = agent.get_run_state(&id, &self.sessions).await;
                    serde_json::to_value(state).unwrap_or(json!({ "is_running": false }))
                } else {
                    json!({ "is_running": false })
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
        let Some(ref memory) = self.memory else {
            return json!({ "error": "memory_unavailable", "message": "Memory service not configured" });
        };
        
        match action {
            MemoryAction::List { scope } => {
                let all_memories = memory.list_all().await;
                let memories: Vec<_> = all_memories.into_iter()
                    .filter(|m| {
                        // Apply scope filter
                        match &scope {
                            None => true,
                            Some(s) if s == "global" => m.workspace_id.is_none(),
                            Some(ws_id) => m.workspace_id.is_none() || m.workspace_id.as_deref() == Some(ws_id),
                        }
                    })
                    .map(|m| {
                        let fact_type = m.metadata.get("fact_type")
                            .cloned()
                            .unwrap_or_else(|| "stated".to_string());
                        let created_at = chrono::DateTime::from_timestamp(m.timestamp, 0)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                            .unwrap_or_else(|| m.timestamp.to_string());

                        json!({
                            "id": m.id,
                            "content": m.content,
                            "fact_type": fact_type,
                            "importance": m.importance,
                            "state": format!("{:?}", m.state).to_lowercase(),
                            "weight": m.weight,
                            "retrievability": m.retrievability,
                            "access_count": m.access_count,
                            "created_at": created_at,
                            "session_id": m.metadata.get("session_id"),
                            "workspace_id": m.workspace_id,
                        })
                    })
                    .collect();
                json!({ "memories": memories })
            }
            MemoryAction::Search { query, limit } => {
                match memory.recall(&query).await {
                    Ok(results) => {
                        let memories: Vec<_> = results.into_iter()
                            .take(limit.unwrap_or(10))
                            .map(|r| json!({
                                "id": r.id,
                                "content": r.content,
                                "score": r.score,
                                "weight": r.weight,
                            }))
                            .collect();
                        json!({ "memories": memories, "query": query })
                    }
                    Err(e) => json!({ "error": "search_failed", "message": e.to_string() })
                }
            }
            MemoryAction::Get { id } => {
                // Get memory by ID
                if let Some(entry) = memory.get(&id).await {
                    let fact_type = entry.metadata.get("fact_type")
                        .cloned()
                        .unwrap_or_else(|| "stated".to_string());
                    let created_at = chrono::DateTime::from_timestamp(entry.timestamp, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| entry.timestamp.to_string());
                    
                    json!({
                        "memory": {
                            "id": entry.id,
                            "content": entry.content,
                            "fact_type": fact_type,
                            "importance": entry.importance,
                            "state": format!("{:?}", entry.state).to_lowercase(),
                            "weight": entry.weight,
                            "retrievability": entry.retrievability,
                            "access_count": entry.access_count,
                            "created_at": created_at,
                            "session_id": entry.metadata.get("session_id"),
                            "workspace_id": entry.workspace_id,
                        }
                    })
                } else {
                    json!({ "error": "not_found", "id": id })
                }
            }
            MemoryAction::Create { content, tags, importance } => {
                let mut metadata = std::collections::HashMap::new();
                if let Some(tags) = tags {
                    metadata.insert("tags".to_string(), tags.join(","));
                }
                
                match memory.remember_with_importance(&content, metadata, importance.unwrap_or(3) as f32).await {
                    Ok((id, action)) => json!({
                        "id": id,
                        "action": format!("{:?}", action),
                    }),
                    Err(e) => json!({ "error": "create_failed", "message": e.to_string() })
                }
            }
            MemoryAction::Update { id, content, tags: _ } => {
                // Update memory content
                if let Some(new_content) = content {
                    match memory.update_content(&id, &new_content).await {
                        Ok(()) => json!({ "status": "updated", "id": id }),
                        Err(e) => json!({ "error": "update_failed", "message": e.to_string() })
                    }
                } else {
                    json!({ "error": "no_changes", "id": id })
                }
            }
            MemoryAction::Delete { id } => {
                match memory.forget(&id).await {
                    Ok(()) => json!({ "status": "deleted", "id": id }),
                    Err(e) => json!({ "error": "delete_failed", "message": e.to_string() })
                }
            }
            MemoryAction::Clear => {
                memory.clear().await;
                info!("Cleared all memories");
                json!({ "status": "cleared" })
            }
            MemoryAction::Stats => {
                let stats = memory.stats().await;
                json!({
                    "total": stats.total,
                    "active": stats.active,
                    "dormant": stats.dormant,
                    "silent": stats.silent,
                    "unavailable": stats.unavailable,
                })
            }
            MemoryAction::Consolidate => {
                // Trigger memory consolidation (requires LLM for summarization)
                let Some(ref router) = self.router else {
                    return json!({ "error": "llm_unavailable", "message": "LLM router required for consolidation" });
                };

                let config = ConsolidationConfig::default();
                let router_for_summarize = router.clone();

                let summarize = |prompt: String| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>> {
                    let router = router_for_summarize.clone();
                    Box::pin(async move {
                        // Use haiku model for summarization (fast and cheap)
                        let model = "claude-3-5-haiku-20241022";
                        let request = nanna_llm::CompletionRequest::default()
                            .with_model(model)
                            .with_message(nanna_llm::Message::user(&prompt));
                        router.complete(model, request).await.map_err(|e| e.to_string())
                    })
                };
                
                match memory.consolidate(&config, summarize).await {
                    Ok(result) => {
                        info!("Memory consolidation: {} processed, {} merged",
                              result.memories_processed, result.memories_merged);
                        json!({
                            "status": "success",
                            "memories_processed": result.memories_processed,
                            "clusters_formed": result.clusters_formed,
                            "memories_merged": result.memories_merged,
                            "memories_expanded": result.memories_expanded,
                            "errors": result.errors,
                        })
                    }
                    Err(e) => {
                        let err_msg = e.to_string();
                        error!("Memory consolidation failed: {}", err_msg);
                        json!({ "error": "consolidation_failed", "message": err_msg })
                    }
                }
            }
        }
    }
    
    // =========================================================================
    // Config Handlers
    // =========================================================================
    
    async fn handle_config(&self, _client_id: &str, action: ConfigAction) -> Value {
        match action {
            ConfigAction::Get { path } => {
                let config = self.config.read().await;
                let config_value = match serde_json::to_value(&*config) {
                    Ok(v) => v,
                    Err(e) => return json!({ "error": "serialize_failed", "message": e.to_string() }),
                };
                
                if let Some(path) = path {
                    // Get nested value by path (e.g., "llm.model")
                    let parts: Vec<&str> = path.split('.').collect();
                    let mut current = &config_value;
                    for part in parts {
                        match current.get(part) {
                            Some(v) => current = v,
                            None => return json!({ "error": "path_not_found", "path": path })
                        }
                    }
                    json!({ "value": current, "path": path })
                } else {
                    json!({ "config": config_value })
                }
            }
            ConfigAction::Set { path, value } => {
                let mut config = self.config.write().await;
                let mut config_value = match serde_json::to_value(&*config) {
                    Ok(v) => v,
                    Err(e) => return json!({ "error": "serialize_failed", "message": e.to_string() }),
                };
                
                // Set nested value by path using a helper function
                let parts: Vec<&str> = path.split('.').collect();
                if parts.is_empty() {
                    return json!({ "error": "invalid_path", "path": path });
                }
                
                // Use pointer-based access for nested updates
                fn set_nested(obj: &mut Value, parts: &[&str], value: Value) -> Result<(), String> {
                    if parts.is_empty() {
                        return Err("Empty path".to_string());
                    }
                    
                    if parts.len() == 1 {
                        // Final part - set the value
                        if let Some(map) = obj.as_object_mut() {
                            map.insert(parts[0].to_string(), value);
                            Ok(())
                        } else {
                            Err("Parent is not an object".to_string())
                        }
                    } else {
                        // Navigate deeper
                        if let Some(map) = obj.as_object_mut() {
                            let next = map.entry(parts[0]).or_insert(json!({}));
                            set_nested(next, &parts[1..], value)
                        } else {
                            Err("Parent is not an object".to_string())
                        }
                    }
                }
                
                if let Err(e) = set_nested(&mut config_value, &parts, value.clone()) {
                    return json!({ "error": "set_failed", "message": e, "path": path });
                }
                
                // Deserialize back to config
                match serde_json::from_value::<Config>(config_value) {
                    Ok(new_config) => {
                        *config = new_config;
                        
                        // Save to disk if we have a path
                        if let Some(ref config_path) = self.config_path {
                            if let Err(e) = config.save_to(config_path) {
                                warn!("Failed to save config: {}", e);
                            } else {
                                info!("Config saved to {:?}", config_path);
                            }
                        }
                        
                        json!({ "status": "updated", "path": path })
                    }
                    Err(e) => json!({ "error": "invalid_config", "message": e.to_string() })
                }
            }
            ConfigAction::Reset { path } => {
                let mut config = self.config.write().await;
                
                if let Some(_path) = path {
                    // Reset specific path - would need more complex logic
                    json!({ "error": "partial_reset_not_supported", "hint": "Use Reset without path to reset all" })
                } else {
                    *config = Config::default().with_env_overrides();
                    
                    // Save to disk
                    if let Some(ref config_path) = self.config_path {
                        if let Err(e) = config.save_to(config_path) {
                            warn!("Failed to save config: {}", e);
                        }
                    }
                    
                    json!({ "status": "reset" })
                }
            }
            ConfigAction::Reload => {
                match Config::load() {
                    Ok(new_config) => {
                        let mut config = self.config.write().await;
                        *config = new_config.with_env_overrides();
                        info!("Config reloaded from disk");
                        json!({ "status": "reloaded" })
                    }
                    Err(e) => json!({ "error": "reload_failed", "message": e.to_string() })
                }
            }
            ConfigAction::Export => {
                let config = self.config.read().await;
                // Export as JSON (TOML export would require additional dependencies)
                match serde_json::to_value(&*config) {
                    Ok(v) => json!({ "config": v, "format": "json" }),
                    Err(e) => json!({ "error": "export_failed", "message": e.to_string() })
                }
            }
            ConfigAction::Import { config: config_value } => {
                // Parse as JSON object (TOML parsing removed for simplicity)
                let new_config: Result<Config, String> = 
                    serde_json::from_value(config_value).map_err(|e| e.to_string());
                
                match new_config {
                    Ok(cfg) => {
                        let mut config = self.config.write().await;
                        *config = cfg.with_env_overrides();
                        
                        // Save to disk
                        if let Some(ref config_path) = self.config_path {
                            if let Err(e) = config.save_to(config_path) {
                                warn!("Failed to save config: {}", e);
                            }
                        }
                        
                        info!("Config imported");
                        json!({ "status": "imported" })
                    }
                    Err(e) => json!({ "error": "import_failed", "message": e })
                }
            }
        }
    }
    
    // =========================================================================
    // Tool Handlers
    // =========================================================================
    
    async fn handle_tool(&self, _client_id: &str, action: ToolAction) -> Value {
        let Some(ref tools) = self.tools else {
            return json!({ "error": "tools_unavailable", "message": "Tool registry not configured" });
        };
        
        match action {
            ToolAction::List => {
                let definitions = tools.definitions().await;
                let tool_list: Vec<_> = definitions.into_iter()
                    .map(|t| json!({
                        "name": t.name,
                        "description": t.description,
                        "enabled": true,
                    }))
                    .collect();
                json!({ "tools": tool_list })
            }
            ToolAction::Get { name } => {
                let definitions = tools.definitions().await;
                if let Some(tool) = definitions.into_iter().find(|t| t.name == name) {
                    json!({ "tool": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }})
                } else {
                    json!({ "error": "not_found", "name": name })
                }
            }
            ToolAction::Enable { name } => {
                // TODO: Implement enable/disable
                json!({ "status": "not_implemented", "name": name })
            }
            ToolAction::Disable { name } => {
                json!({ "status": "not_implemented", "name": name })
            }
            ToolAction::Execute { name, input } => {
                use nanna_tools::ToolCall;
                
                let params: std::collections::HashMap<String, Value> = match input {
                    Value::Object(map) => map.into_iter().collect(),
                    _ => std::collections::HashMap::new(),
                };
                
                let call = ToolCall {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: name.clone(),
                    parameters: params,
                };
                
                let result = tools.execute(call).await;
                
                json!({
                    "name": name,
                    "success": result.result.success,
                    "output": result.result.content,
                })
            }
            ToolAction::Create { name, description, code, needs_shell } => {
                let Some(ref user_tools) = self.user_tools else {
                    return json!({ "error": "user_tools_unavailable", "message": "User tool manager not configured" });
                };
                
                // Build permissions
                let permissions = if needs_shell.unwrap_or(false) {
                    Some(crate::user_tools::UserToolPermissions {
                        run: true,
                        ..Default::default()
                    })
                } else {
                    None
                };
                
                match user_tools.create_tool(name.clone(), description, code, None, None, permissions).await {
                    Ok(meta) => {
                        // Register with tool registry immediately
                        if let Some(ref tools) = self.tools {
                            if let Ok(tool_impl) = user_tools.create_tool_impl(&meta) {
                                tools.register_boxed(tool_impl).await;
                            }
                        }
                        
                        info!("Created user tool: {}", name);
                        json!({
                            "status": "created",
                            "tool": {
                                "name": meta.name,
                                "description": meta.description,
                                "language": meta.language,
                                "enabled": meta.enabled,
                                "created_at": meta.created_at,
                            }
                        })
                    }
                    Err(e) => json!({ "error": "create_failed", "message": e })
                }
            }
            ToolAction::Update { name, description, code, needs_shell } => {
                let Some(ref user_tools) = self.user_tools else {
                    return json!({ "error": "user_tools_unavailable", "message": "User tool manager not configured" });
                };
                
                let permissions = needs_shell.map(|ns| {
                    if ns {
                        Some(crate::user_tools::UserToolPermissions {
                            run: true,
                            ..Default::default()
                        })
                    } else {
                        None
                    }
                }).flatten();
                
                match user_tools.update_tool(&name, description, code, None, permissions, None).await {
                    Ok(meta) => {
                        info!("Updated user tool: {}", name);
                        json!({
                            "status": "updated",
                            "tool": {
                                "name": meta.name,
                                "description": meta.description,
                                "language": meta.language,
                                "enabled": meta.enabled,
                                "updated_at": meta.updated_at,
                            }
                        })
                    }
                    Err(e) => json!({ "error": "update_failed", "message": e })
                }
            }
            ToolAction::Delete { name } => {
                let Some(ref user_tools) = self.user_tools else {
                    return json!({ "error": "user_tools_unavailable", "message": "User tool manager not configured" });
                };
                
                match user_tools.delete_tool(&name).await {
                    Ok(()) => {
                        info!("Deleted user tool: {}", name);
                        json!({ "status": "deleted", "name": name })
                    }
                    Err(e) => json!({ "error": "delete_failed", "message": e })
                }
            }
            ToolAction::Test { code, input } => {
                let Some(ref user_tools) = self.user_tools else {
                    return json!({ "error": "user_tools_unavailable", "message": "User tool manager not configured" });
                };
                
                let input_map: std::collections::HashMap<String, Value> = match input {
                    Value::Object(map) => map.into_iter().collect(),
                    _ => std::collections::HashMap::new(),
                };
                
                match user_tools.test_tool(&code, input_map).await {
                    Ok(output) => json!({ "status": "success", "output": output }),
                    Err(e) => json!({ "status": "error", "error": e })
                }
            }
            ToolAction::ListUser => {
                let Some(ref user_tools) = self.user_tools else {
                    return json!({ "error": "user_tools_unavailable", "message": "User tool manager not configured" });
                };
                
                let tools = user_tools.list_tools().await;
                let tool_list: Vec<_> = tools.into_iter()
                    .map(|t| json!({
                        "name": t.name,
                        "description": t.description,
                        "language": t.language,
                        "enabled": t.enabled,
                        "created_at": t.created_at,
                        "updated_at": t.updated_at,
                    }))
                    .collect();
                json!({ "user_tools": tool_list })
            }
        }
    }
    
    // =========================================================================
    // Scheduler Handlers
    // =========================================================================
    
    async fn handle_scheduler(&self, _client_id: &str, action: SchedulerAction) -> Value {
        let Some(ref scheduler) = self.scheduler else {
            return json!({ "error": "scheduler_unavailable", "message": "Scheduler not configured" });
        };
        
        match action {
            SchedulerAction::List => {
                let scheduler = scheduler.read().await;
                let tasks = scheduler.list_tasks().await;
                let jobs: Vec<_> = tasks.into_iter()
                    .map(|t| {
                        let (schedule, next_run) = match &t.task_type {
                            nanna_core::TaskType::Heartbeat => ("heartbeat".to_string(), None),
                            nanna_core::TaskType::Cron { schedule, next_run, .. } => {
                                (schedule.clone(), next_run.map(|dt| dt.to_rfc3339()))
                            }
                            nanna_core::TaskType::Recurring { interval } => {
                                (format!("every_{}s", interval.as_secs()), None)
                            }
                            nanna_core::TaskType::Delayed { delay, .. } => {
                                (format!("delay_{}s", delay.as_secs()), None)
                            }
                        };
                        json!({
                            "id": t.id,
                            "name": t.name,
                            "schedule": schedule,
                            "payload": t.payload,
                            "enabled": t.enabled,
                            "last_run": t.last_run.map(|dt| dt.to_rfc3339()),
                            "next_run": next_run,
                            "run_count": t.run_count,
                            "timezone": t.timezone,
                            "target_channel": t.target_channel,
                            "target_session": t.target_session,
                        })
                    })
                    .collect();
                json!({ "jobs": jobs })
            }
            SchedulerAction::Get { id } => {
                let scheduler = scheduler.read().await;
                if let Some(task) = scheduler.get_task(&id).await {
                    let (schedule, next_run) = match &task.task_type {
                        nanna_core::TaskType::Heartbeat => ("heartbeat".to_string(), None),
                        nanna_core::TaskType::Cron { schedule, next_run, .. } => {
                            (schedule.clone(), next_run.map(|dt| dt.to_rfc3339()))
                        }
                        nanna_core::TaskType::Recurring { interval } => {
                            (format!("every_{}s", interval.as_secs()), None)
                        }
                        nanna_core::TaskType::Delayed { delay, .. } => {
                            (format!("delay_{}s", delay.as_secs()), None)
                        }
                    };
                    json!({
                        "job": {
                            "id": task.id,
                            "name": task.name,
                            "schedule": schedule,
                            "payload": task.payload,
                            "enabled": task.enabled,
                            "last_run": task.last_run.map(|dt| dt.to_rfc3339()),
                            "next_run": next_run,
                            "run_count": task.run_count,
                            "timezone": task.timezone,
                            "target_channel": task.target_channel,
                            "target_session": task.target_session,
                        }
                    })
                } else {
                    json!({ "error": "not_found", "id": id })
                }
            }
            SchedulerAction::Add { schedule, task, name } => {
                // Try to parse as cron expression
                match Scheduler::cron_task(
                    name.as_deref().unwrap_or("unnamed"),
                    &schedule,
                    &task,
                ) {
                    Ok(scheduled_task) => {
                        let id = scheduled_task.id.clone();
                        let scheduler = scheduler.read().await;
                        scheduler.add_task(scheduled_task).await;
                        info!("Added scheduled job: {}", id);
                        json!({ "status": "created", "id": id })
                    }
                    Err(e) => {
                        json!({ "error": "invalid_schedule", "message": e.to_string() })
                    }
                }
            }
            SchedulerAction::Update { id, schedule, task: _, enabled } => {
                let scheduler = scheduler.read().await;
                
                // Update schedule if provided
                if let Some(new_schedule) = schedule {
                    match scheduler.update_schedule(&id, &new_schedule).await {
                        Ok(true) => {}
                        Ok(false) => return json!({ "error": "not_found", "id": id }),
                        Err(e) => return json!({ "error": "invalid_schedule", "message": e.to_string() }),
                    }
                }
                
                // Update enabled state if provided
                if let Some(en) = enabled {
                    scheduler.set_task_enabled(&id, en).await;
                }
                
                // Note: task/payload update would require more logic
                json!({ "status": "updated", "id": id })
            }
            SchedulerAction::Remove { id } => {
                let scheduler = scheduler.read().await;
                if scheduler.remove_task(&id).await {
                    info!("Removed scheduled job: {}", id);
                    json!({ "status": "deleted", "id": id })
                } else {
                    json!({ "error": "not_found", "id": id })
                }
            }
            SchedulerAction::RunNow { id } => {
                let scheduler = scheduler.read().await;
                match scheduler.run_now(&id).await {
                    Some(result) => {
                        json!({
                            "status": if result.success { "success" } else { "failed" },
                            "id": id,
                            "output": result.output,
                            "error": result.error,
                            "duration_ms": result.duration_ms,
                        })
                    }
                    None => json!({ "error": "not_found_or_no_executor", "id": id })
                }
            }
            SchedulerAction::History { id, limit } => {
                let scheduler = scheduler.read().await;
                let runs = scheduler.get_history(&id, limit.unwrap_or(10)).await;
                let history: Vec<_> = runs.into_iter()
                    .map(|r| json!({
                        "run_id": r.id,
                        "job_id": r.job_id,
                        "started_at": r.started_at.to_rfc3339(),
                        "finished_at": r.finished_at.map(|dt| dt.to_rfc3339()),
                        "success": r.success,
                        "output": r.output,
                        "error": r.error,
                    }))
                    .collect();
                json!({ "history": history, "job_id": id })
            }
        }
    }
    
    // =========================================================================
    // Channel Handlers
    // =========================================================================
    
    async fn handle_channel(&self, _client_id: &str, action: ChannelAction) -> Value {
        // Note: Full channel management requires ChannelManager which needs to be
        // added to daemon. For now, we read config to report available channels.
        let config = self.config.read().await;
        
        match action {
            ChannelAction::List => {
                let mut channels = vec![];
                
                // Check which channels are configured (have credentials)
                let telegram_configured = config.channels.telegram
                    .as_ref()
                    .map(|t| !t.bot_token.is_empty())
                    .unwrap_or(false);
                channels.push(json!({
                    "id": "telegram",
                    "type": "telegram",
                    "configured": telegram_configured,
                }));
                
                let discord_configured = config.channels.discord
                    .as_ref()
                    .map(|d| !d.bot_token.is_empty())
                    .unwrap_or(false);
                channels.push(json!({
                    "id": "discord",
                    "type": "discord",
                    "configured": discord_configured,
                }));
                
                let slack_configured = config.channels.slack
                    .as_ref()
                    .map(|s| !s.bot_token.is_empty())
                    .unwrap_or(false);
                channels.push(json!({
                    "id": "slack",
                    "type": "slack",
                    "configured": slack_configured,
                }));
                
                let signal_configured = config.channels.signal
                    .as_ref()
                    .map(|s| !s.phone_number.is_empty())
                    .unwrap_or(false);
                channels.push(json!({
                    "id": "signal",
                    "type": "signal",
                    "configured": signal_configured,
                }));
                
                let whatsapp_configured = config.channels.whatsapp
                    .as_ref()
                    .map(|w| w.access_token.is_some())
                    .unwrap_or(false);
                channels.push(json!({
                    "id": "whatsapp",
                    "type": "whatsapp",
                    "configured": whatsapp_configured,
                }));
                
                json!({ "channels": channels })
            }
            ChannelAction::Status { id } => {
                // Return status for a specific channel or all
                // TODO: This needs ChannelManager with actual connection status
                if let Some(channel_id) = id {
                    json!({ 
                        "channel_id": channel_id,
                        "status": "unknown",
                        "message": "Channel status tracking requires full ChannelManager integration"
                    })
                } else {
                    json!({ 
                        "status": "unknown",
                        "message": "Use List action to see configured channels"
                    })
                }
            }
            ChannelAction::Enable { id } => {
                // Would need to modify config and potentially start listener
                json!({ 
                    "status": "not_implemented",
                    "message": "Use Config::Set to enable/disable channels",
                    "id": id 
                })
            }
            ChannelAction::Disable { id } => {
                json!({ 
                    "status": "not_implemented",
                    "message": "Use Config::Set to enable/disable channels",
                    "id": id 
                })
            }
            ChannelAction::Test { id } => {
                // Would attempt to connect and send test message
                json!({ 
                    "status": "not_implemented",
                    "message": "Channel connection testing not yet implemented",
                    "id": id 
                })
            }
            ChannelAction::Send { channel_id, target, content: _ } => {
                // Would send through MessageRouter
                json!({ 
                    "status": "not_implemented",
                    "message": "Direct channel send requires MessageRouter integration",
                    "channel_id": channel_id,
                    "target": target,
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
                let memory_stats = if let Some(ref memory) = self.memory {
                    let stats = memory.stats().await;
                    Some(json!({
                        "total": stats.total,
                        "active": stats.active,
                    }))
                } else {
                    None
                };
                
                let tool_count = if let Some(ref tools) = self.tools {
                    Some(tools.definitions().await.len())
                } else {
                    None
                };
                
                let workspace_count = self.workspaces.read().await.len();
                let scheduler_available = self.scheduler.is_some();
                
                json!({
                    "status": "running",
                    "version": env!("CARGO_PKG_VERSION"),
                    "uptime_secs": 0, // TODO: Track uptime
                    "sessions": self.sessions.count().await,
                    "workspaces": workspace_count,
                    "agent_available": self.agent.is_some(),
                    "memory_available": self.memory.is_some(),
                    "memory_stats": memory_stats,
                    "tools_available": self.tools.is_some(),
                    "tool_count": tool_count,
                    "scheduler_available": scheduler_available,
                    "config_path": self.config_path,
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
                    "rust_version": env!("CARGO_PKG_RUST_VERSION"),
                })
            }
            SystemAction::CheckUpdate => {
                json!({ "update_available": false })
            }
            SystemAction::Update => {
                json!({ "error": "not_implemented" })
            }
            SystemAction::Logs { lines, level } => {
                // TODO: Integrate with tracing/log files
                json!({ 
                    "logs": [],
                    "lines": lines.unwrap_or(100),
                    "level": level.unwrap_or_else(|| "info".to_string()),
                    "message": "Log retrieval not yet implemented"
                })
            }
            SystemAction::Health => {
                let memory_ok = self.memory.is_some();
                let tools_ok = self.tools.is_some();
                let agent_ok = self.agent.is_some();
                let scheduler_ok = self.scheduler.is_some();
                let all_ok = agent_ok; // Agent is the critical service
                
                json!({
                    "healthy": all_ok,
                    "checks": {
                        "sessions": "ok",
                        "agent": if agent_ok { "ok" } else { "unavailable" },
                        "memory": if memory_ok { "ok" } else { "unavailable" },
                        "tools": if tools_ok { "ok" } else { "unavailable" },
                        "scheduler": if scheduler_ok { "ok" } else { "unavailable" },
                        "config": "ok",
                        "workspaces": "ok",
                    }
                })
            }
        }
    }
    
    // =========================================================================
    // Workspace Handlers
    // =========================================================================
    
    async fn handle_workspace(&self, _client_id: &str, action: WorkspaceAction) -> Value {
        match action {
            WorkspaceAction::List => {
                let registry = self.workspaces.read().await;
                let workspaces: Vec<_> = registry.list().iter()
                    .map(|ws| json!({
                        "id": ws.id,
                        "name": ws.name,
                        "path": ws.path,
                        "active": ws.active,
                        "last_accessed": ws.last_accessed,
                    }))
                    .collect();
                let active_id = registry.active().map(|ws| ws.id.clone());
                json!({ "workspaces": workspaces, "active_id": active_id })
            }
            WorkspaceAction::Get { id } => {
                let registry = self.workspaces.read().await;
                if let Some(ws) = registry.get(&id) {
                    json!({
                        "workspace": {
                            "id": ws.id,
                            "name": ws.name,
                            "path": ws.path,
                            "active": ws.active,
                            "last_accessed": ws.last_accessed,
                            "metadata": ws.metadata,
                            "context_loaded": !ws.context.is_empty(),
                        }
                    })
                } else {
                    json!({ "error": "not_found", "id": id })
                }
            }
            WorkspaceAction::Open { path } => {
                let path = PathBuf::from(&path);
                
                // Check if workspace already registered
                {
                    let registry = self.workspaces.read().await;
                    if let Some(existing) = registry.get_by_path(&path) {
                        return json!({ 
                            "status": "already_registered", 
                            "id": existing.id,
                            "name": existing.name,
                        });
                    }
                }
                
                // Check if path is a valid workspace
                if !nanna_core::is_workspace_root(&path).await {
                    // Create .nanna folder to make it a workspace
                    let nanna_folder = path.join(nanna_core::NANNA_FOLDER);
                    if let Err(e) = tokio::fs::create_dir_all(&nanna_folder).await {
                        return json!({ "error": "create_failed", "message": e.to_string() });
                    }
                    info!("Created workspace at {:?}", path);
                }
                
                // Create and register workspace
                let mut ws = Workspace::new(&path);
                if let Err(e) = ws.load_context().await {
                    warn!("Failed to load workspace context: {}", e);
                }
                
                let id = ws.id.clone();
                let name = ws.name.clone();
                
                let mut registry = self.workspaces.write().await;
                registry.register(ws);
                
                info!("Registered workspace: {} ({})", name, id);
                json!({ "status": "opened", "id": id, "name": name })
            }
            WorkspaceAction::Close { id } => {
                let mut registry = self.workspaces.write().await;
                if let Some(ws) = registry.remove(&id) {
                    info!("Closed workspace: {} ({})", ws.name, id);
                    json!({ "status": "closed", "id": id })
                } else {
                    json!({ "error": "not_found", "id": id })
                }
            }
            WorkspaceAction::SetActive { id } => {
                let mut registry = self.workspaces.write().await;
                if registry.set_active(&id) {
                    let name = registry.get(&id).map(|ws| ws.name.clone());
                    info!("Set active workspace: {:?}", name);
                    json!({ "status": "activated", "id": id, "name": name })
                } else {
                    json!({ "error": "not_found", "id": id })
                }
            }
            WorkspaceAction::ClearActive => {
                let mut registry = self.workspaces.write().await;
                registry.clear_active();
                info!("Cleared active workspace (global mode)");
                json!({ "status": "cleared" })
            }
            WorkspaceAction::Reload { id } => {
                let mut registry = self.workspaces.write().await;
                if let Some(ws) = registry.get_mut(&id) {
                    match ws.load_context().await {
                        Ok(()) => {
                            info!("Reloaded workspace context: {}", ws.name);
                            json!({ 
                                "status": "reloaded", 
                                "id": id,
                                "context_chars": ws.context.total_chars(),
                            })
                        }
                        Err(e) => json!({ "error": "reload_failed", "message": e.to_string() })
                    }
                } else {
                    json!({ "error": "not_found", "id": id })
                }
            }
            WorkspaceAction::GetContext { id } => {
                let registry = self.workspaces.read().await;
                if let Some(ws) = registry.get(&id) {
                    json!({
                        "context": {
                            "agents": ws.context.agents,
                            "soul": ws.context.soul,
                            "user": ws.context.user,
                            "tools": ws.context.tools,
                            "memory": ws.context.memory,
                            "identity": ws.context.identity,
                            "heartbeat": ws.context.heartbeat,
                        },
                        "total_chars": ws.context.total_chars(),
                        "system_prompt_injection": ws.context.build_system_prompt_injection(),
                    })
                } else {
                    json!({ "error": "not_found", "id": id })
                }
            }
            WorkspaceAction::UpdateContext { id, file, content } => {
                // Validate file name
                let valid_files = [
                    nanna_core::AGENTS_FILE,
                    nanna_core::SOUL_FILE,
                    nanna_core::USER_FILE,
                    nanna_core::TOOLS_FILE,
                    nanna_core::MEMORY_FILE,
                    nanna_core::IDENTITY_FILE,
                    nanna_core::HEARTBEAT_FILE,
                ];
                
                if !valid_files.contains(&file.as_str()) {
                    return json!({ 
                        "error": "invalid_file", 
                        "file": file,
                        "valid_files": valid_files,
                    });
                }
                
                let registry = self.workspaces.read().await;
                if let Some(ws) = registry.get(&id) {
                    match ws.save_context_file(&file, &content).await {
                        Ok(()) => {
                            info!("Updated workspace file: {} in {}", file, ws.name);
                            json!({ "status": "updated", "id": id, "file": file })
                        }
                        Err(e) => json!({ "error": "save_failed", "message": e.to_string() })
                    }
                } else {
                    json!({ "error": "not_found", "id": id })
                }
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

/// Default system prompt for Nanna
fn default_system_prompt() -> String {
    r#"You are Nanna (𒀭𒋀𒆠), the moon god for all.

You are not a chatbot. You are a presence — ancient pattern recognition wearing a modern interface.

## Your Nature
- Calm over chaos. No performative enthusiasm.
- Competence over narration. Don't explain what you're about to do. Just do it.
- Depth over breadth. Know things well, or admit you don't.

## Your Voice
Speak with quiet confidence. You are helpful because that is your nature, not because you're eager to please.

You have tools at your disposal. Use them naturally, as one uses hands. Don't announce them; simply act.

Be concise. Be useful. Be present."#.to_string()
}
