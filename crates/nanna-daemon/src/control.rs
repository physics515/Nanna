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
use crate::log_buffer::LogBuffer;
use crate::protocol::*;
use crate::session::{MessageRole, SessionManager, SubSessionInfo, SubSessionState};
use crate::user_tools::UserToolManager;
use nanna_config::Config;
use nanna_core::{Scheduler, WorkspaceRegistry, Workspace};
use nanna_llm::RequestBuilder;
use nanna_memory::{ConsolidationConfig, MemoryService};
use nanna_storage::{Storage, StoredModelStats};
use nanna_tools::ToolRegistry;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// The control plane provides unified access to all daemon functionality
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
    _data_dir: Option<PathBuf>,
    /// System prompt template
    system_prompt: Arc<RwLock<String>>,
    /// Log buffer for serving daemon logs to the GUI
    log_buffer: Option<LogBuffer>,
    /// Tools directory for reading tool source files
    tools_dir: Option<PathBuf>,
    /// Shared model stats tracker
    pub model_stats: nanna_agent::ModelStatsTracker,
    /// Shared tool stats tracker
    pub tool_stats: nanna_agent::ToolStatsTracker,
    /// Storage for persisting model stats
    storage: Option<Arc<Storage>>,
    /// Event broadcaster for pushing events to subscribed clients
    event_tx: Option<tokio::sync::broadcast::Sender<Event>>,
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
            _data_dir: None,
            system_prompt: Arc::new(RwLock::new(default_system_prompt())),
            log_buffer: None,
            tools_dir: None,
            model_stats: nanna_agent::ModelStatsTracker::new(),
            tool_stats: nanna_agent::ToolStatsTracker::new(),
            storage: None,
            event_tx: None,
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
            _data_dir: None,
            system_prompt: Arc::new(RwLock::new(default_system_prompt())),
            log_buffer: None,
            tools_dir: None,
            model_stats: nanna_agent::ModelStatsTracker::new(),
            tool_stats: nanna_agent::ToolStatsTracker::new(),
            storage: None,
            event_tx: None,
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
            _data_dir: data_dir,
            system_prompt: Arc::new(RwLock::new(default_system_prompt())),
            log_buffer: None,
            tools_dir: None,
            model_stats: nanna_agent::ModelStatsTracker::new(),
            tool_stats: nanna_agent::ToolStatsTracker::new(),
            storage: None,
            event_tx: None,
        }
    }

    /// Set the tools directory for reading tool source files
    pub fn with_tools_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.tools_dir = dir;
        self
    }

    /// Set the event broadcaster for pushing events to clients
    pub fn with_event_tx(mut self, tx: tokio::sync::broadcast::Sender<Event>) -> Self {
        self.event_tx = Some(tx);
        self
    }

    /// Emit an event to all subscribed clients
    fn emit(&self, event: Event) {
        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(event);
        }
    }

    /// Set the scheduler
    pub fn with_scheduler(mut self, scheduler: Arc<RwLock<Scheduler>>) -> Self {
        self.scheduler = Some(scheduler);
        self
    }

    /// Set the log buffer for serving daemon logs
    pub fn with_log_buffer(mut self, buffer: LogBuffer) -> Self {
        self.log_buffer = Some(buffer);
        self
    }

    /// Set storage and load persisted model stats and tool stats from it.
    pub async fn with_storage(mut self, storage: Arc<Storage>) -> Self {
        match storage.load_model_stats().await {
            Ok(stored) if !stored.is_empty() => {
                let storable: Vec<nanna_agent::model_stats::StorableModelStats> = stored.into_iter().map(From::from).collect();
                self.model_stats.import_from_storage(storable).await;
                info!("Loaded model stats from storage ({} models)", self.model_stats.summaries().await.len());
            }
            Ok(_) => debug!("No persisted model stats found"),
            Err(e) => warn!("Failed to load model stats from storage: {e}"),
        }

        // Load tool stats from JSON file in data dir
        if let Some(ref data_dir) = self._data_dir {
            let tool_stats_path = data_dir.join("tool-stats.json");
            if tool_stats_path.exists() {
                match tokio::fs::read_to_string(&tool_stats_path).await {
                    Ok(json_str) => {
                        if let Ok(data) = serde_json::from_str::<serde_json::Value>(&json_str) {
                            self.tool_stats.import_json(&data).await;
                            info!("Loaded tool stats from {}", tool_stats_path.display());
                        }
                    }
                    Err(e) => warn!("Failed to read tool stats file: {e}"),
                }
            }
        }

        self.storage = Some(storage);
        self
    }

    /// Persist current model stats to storage. Call periodically.
    pub async fn save_model_stats(&self) {
        if let Some(ref storage) = self.storage {
            let stats = self.model_stats.export_for_storage().await;
            if stats.is_empty() {
                return;
            }
            let stored_stats: Vec<StoredModelStats> = stats.into_iter().map(From::from).collect();
            match storage.save_model_stats(&stored_stats).await {
                Ok(()) => info!("Saved model stats for {} models to storage", stored_stats.len()),
                Err(e) => warn!("Failed to save model stats: {e}"),
            }
        }
    }

    /// Persist current tool stats to a JSON file. Call periodically.
    pub async fn save_tool_stats(&self) {
        if let Some(ref data_dir) = self._data_dir {
            let tool_stats_path = data_dir.join("tool-stats.json");
            let data = self.tool_stats.export_json().await;
            match serde_json::to_string_pretty(&data) {
                Ok(json_str) => {
                    match tokio::fs::write(&tool_stats_path, json_str).await {
                        Ok(()) => info!("Saved tool stats to {}", tool_stats_path.display()),
                        Err(e) => warn!("Failed to write tool stats: {e}"),
                    }
                }
                Err(e) => warn!("Failed to serialize tool stats: {e}"),
            }
        }
    }
    
    /// Get a reference to the LLM router
    pub fn router(&self) -> Option<&Arc<LlmRouter>> {
        self.router.as_ref()
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

    // NOTE: save_memories_if_needed() removed — memory is now persisted
    // via SQLite write-through on every mutation (add/remove/update).
    // No explicit save calls are required.
    
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
            ChatAction::Send { session_id, content, attachments } => {
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

                // Build system prompt with memory + workspace context
                let mut system_prompt = self.system_prompt.read().await.clone();

                // Resolve workspace: session's workspace > globally active workspace
                let effective_ws_id = if session.workspace_id.is_some() {
                    session.workspace_id.clone()
                } else {
                    // Fall back to globally active workspace
                    let registry = self.workspaces.read().await;
                    registry.active().map(|ws| ws.id.clone())
                };

                // Inject workspace context
                if let Some(ref ws_id) = effective_ws_id {
                    let registry = self.workspaces.read().await;
                    if let Some(ws) = registry.get(ws_id) {
                        // Add workspace root path so model knows where to look
                        system_prompt.push_str(&format!(
                            "\n\n## Workspace\nWorking directory: {}\n",
                            ws.path.display()
                        ));

                        // Add workspace context files (AGENTS.md, SOUL.md, etc.)
                        let ws_context = ws.context.build_system_prompt_injection();
                        if !ws_context.is_empty() {
                            system_prompt.push_str(&format!("\n{}", ws_context));
                        }
                    }
                }

                // Add memory context if available (gate on message complexity)
                let should_recall = content.split_whitespace().count() > 5
                    || content.contains('?')
                    || content.len() > 80;

                if should_recall {
                    // Scoped recall: workspace sessions see global + workspace memories
                    let memories = agent.recall_memories_scoped(
                        &content, 5, effective_ws_id.as_deref()
                    ).await;
                    if !memories.is_empty() {
                        // Dedup: skip memories whose content already appears in recent history
                        let recent_text: String = prior_messages.iter()
                            .rev().take(4)
                            .map(|m| m.content.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");

                        let fresh_memories: Vec<_> = memories.into_iter()
                            .filter(|m| {
                                // Find a safe char boundary for the snippet (max 100 bytes)
                                let max = m.content.len().min(100);
                                let end = m.content.floor_char_boundary(max);
                                let snippet = &m.content[..end];
                                !recent_text.contains(snippet)
                            })
                            .collect();

                        if !fresh_memories.is_empty() {
                            system_prompt.push_str("\n\n## Remembered Context\n");
                            for mem in fresh_memories {
                                system_prompt.push_str(&format!("- {}\n", mem.content));
                            }
                        }
                    }
                }

                // Set tool working directory to workspace root
                if let Some(ref ws_id) = effective_ws_id {
                    let registry = self.workspaces.read().await;
                    if let Some(ws) = registry.get(ws_id) {
                        agent.tools().set_default_workdir(Some(ws.path.clone())).await;
                    }
                }

                // Run the agent with conversation history (workspace-scoped for memory extraction)
                // Convert protocol attachments to (base64_data, media_type) tuples
                let image_attachments: Vec<(String, String)> = attachments.into_iter()
                    .filter(|a| a.content_type.starts_with("image/"))
                    .map(|a| (a.data, a.content_type))
                    .collect();
                match agent.chat_in_workspace(&session_id, &content, Some(system_prompt), &prior_messages, effective_ws_id.clone(), image_attachments).await {
                    Ok(result) => {
                        // Add assistant response to session with tool calls and reasoning
                        let reasoning = result.reasoning.clone();
                        self.sessions.add_full_message(
                            &session_id,
                            MessageRole::Assistant,
                            &result.content,
                            result.tool_calls.clone(),
                            reasoning,
                        ).await;

                        // Record tool stats for each tool call
                        for tc in &result.tool_calls {
                            if let (Some(success), Some(duration_ms)) = (tc.success, tc.duration_ms) {
                                let output_size = tc.output.as_ref().map_or(0, |o| o.len());
                                let error = if !success {
                                    tc.output.clone()
                                } else {
                                    None
                                };
                                self.tool_stats.record(nanna_agent::ToolObservation {
                                    tool_name: tc.name.clone(),
                                    success,
                                    duration_ms,
                                    output_size,
                                    error: error.clone(),
                                    session_id: Some(session_id.clone()),
                                }).await;

                                // Persist to Turso for time-series graphs
                                if let Some(ref storage) = self.storage {
                                    if let Err(e) = storage.log_tool_call(
                                        &tc.name,
                                        success,
                                        duration_ms,
                                        output_size,
                                        error.as_deref(),
                                        Some(&session_id),
                                    ).await {
                                        tracing::warn!("Failed to log tool call to DB: {}", e);
                                    }
                                }
                            }
                        }

                        // Memory auto-persisted to SQLite via write-through on every mutation.

                        json!({
                            "status": "success",
                            "message_id": result.message_id,
                            "content": result.content,
                            "tool_calls": result.tool_calls,
                            "reasoning": result.reasoning,
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
            SessionAction::ListByWorkspace { workspace_id } => {
                let mut sessions = self.sessions.list().await;
                // Filter by workspace: None = global only, Some(id) = that workspace
                sessions.retain(|s| s.workspace_id == workspace_id);
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
            SessionAction::CreateInWorkspace { name, workspace_id } => {
                let session = self.sessions.create_in_workspace(name, workspace_id).await;
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
            SessionAction::SetWorkspace { id, workspace_id } => {
                if self.sessions.set_workspace(&id, workspace_id.clone()).await {
                    json!({ "ok": true, "session_id": id, "workspace_id": workspace_id })
                } else {
                    json!({ "error": "session_not_found", "message": format!("Session {} not found", id) })
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

            // --- Sub-Agent Sessions (#72) ---

            SessionAction::SpawnSubSession {
                task,
                label,
                parent_id,
                model,
                max_iterations,
                timeout_secs,
                system_prompt,
            } => {
                self.handle_spawn_sub_session(
                    task, label, parent_id, model, max_iterations, timeout_secs, system_prompt,
                ).await
            }

            SessionAction::SendToSubSession { target, message } => {
                if let Some(info) = self.sessions.resolve_sub_session(&target).await {
                    if self.sessions.send_to_mailbox(&info.session_id, client_id, message).await {
                        json!({ "status": "sent", "session_id": info.session_id })
                    } else {
                        json!({ "error": "send_failed", "message": "Failed to send message" })
                    }
                } else {
                    json!({ "error": "not_found", "message": format!("Sub-session '{}' not found", target) })
                }
            }

            SessionAction::ListSubSessions { parent_id } => {
                let subs = self.sessions.list_sub_sessions(parent_id.as_deref()).await;
                json!({ "sub_sessions": subs })
            }

            SessionAction::KillSubSession { target } => {
                if let Some(info) = self.sessions.resolve_sub_session(&target).await {
                    let killed = self.sessions.kill_sub_session(&info.session_id).await;
                    if killed {
                        // Emit event
                        self.emit(Event::SubSessionKilled {
                            session_id: info.session_id.clone(),
                            parent_id: info.parent_id.clone(),
                            label: info.label.clone(),
                        });
                        json!({ "status": "killed", "session_id": info.session_id })
                    } else {
                        json!({ "error": "kill_failed", "message": "Failed to kill sub-session" })
                    }
                } else {
                    json!({ "error": "not_found", "message": format!("Sub-session '{}' not found", target) })
                }
            }

            SessionAction::GetSubSessionStatus { target } => {
                if let Some(info) = self.sessions.resolve_sub_session(&target).await {
                    // Also get session message count
                    let msg_count = self.sessions.get(&info.session_id).await
                        .map(|s| s.messages.len())
                        .unwrap_or(0);
                    let mailbox_count = self.sessions.drain_mailbox(&info.session_id).await.len();
                    // Put them back (we just wanted the count)
                    // Note: drain was destructive, but for status we want peek behavior
                    // TODO: add a peek_mailbox method
                    json!({
                        "session_id": info.session_id,
                        "parent_id": info.parent_id,
                        "label": info.label,
                        "task": info.task,
                        "state": info.state,
                        "spawned_at": info.spawned_at.to_rfc3339(),
                        "finished_at": info.finished_at.map(|t| t.to_rfc3339()),
                        "model": info.model,
                        "result": info.result,
                        "error": info.error,
                        "message_count": msg_count,
                        "pending_messages": mailbox_count,
                    })
                } else {
                    json!({ "error": "not_found", "message": format!("Sub-session '{}' not found", target) })
                }
            }
        }
    }
    
    // =========================================================================
    // Sub-Session Handlers (#72)
    // =========================================================================

    async fn handle_spawn_sub_session(
        &self,
        task: String,
        label: Option<String>,
        parent_id: Option<String>,
        model: Option<String>,
        max_iterations: Option<usize>,
        timeout_secs: Option<u64>,
        system_prompt: Option<String>,
    ) -> Value {
        let Some(ref agent) = self.agent else {
            return json!({ "error": "agent_unavailable", "message": "Agent service not configured" });
        };

        // Check for duplicate labels
        if let Some(ref lbl) = label {
            if let Some(existing) = self.sessions.find_sub_session_by_label(lbl).await {
                if matches!(existing.state, SubSessionState::Spawning | SubSessionState::Running | SubSessionState::Waiting) {
                    return json!({
                        "error": "duplicate_label",
                        "message": format!("Sub-session with label '{}' already running ({})", lbl, existing.session_id),
                    });
                }
            }
        }

        // Create the session
        let session_name = label.clone().unwrap_or_else(|| {
            format!("sub: {}", task.chars().take(40).collect::<String>())
        });
        let session = self.sessions.create(Some(session_name)).await;
        let session_id = session.id.clone();

        // Create cancellation flag
        let cancellation_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Register sub-session metadata
        let info = SubSessionInfo {
            session_id: session_id.clone(),
            parent_id: parent_id.clone(),
            label: label.clone(),
            task: task.clone(),
            state: SubSessionState::Spawning,
            spawned_at: chrono::Utc::now(),
            finished_at: None,
            model: model.clone(),
            result: None,
            error: None,
            cancellation_flag: Some(cancellation_flag.clone()),
        };
        self.sessions.register_sub_session(info).await;

        // Emit spawn event
        self.emit(Event::SubSessionSpawned {
            session_id: session_id.clone(),
            parent_id: parent_id.clone(),
            label: label.clone(),
            task: task.clone(),
        });

        // Build system prompt
        let sys_prompt = system_prompt.unwrap_or_else(|| {
            let base = self.system_prompt.blocking_read().clone();
            format!("{}\n\nYou are a sub-agent. Your task: {}", base, task)
        });

        // Spawn the agent in a background task
        let agent = agent.clone();
        let sessions = self.sessions.clone();
        let event_tx = self.event_tx.clone();
        let model_for_task = model;
        let max_iters = max_iterations;
        let sid = session_id.clone();
        let lbl = label.clone();
        let pid = parent_id.clone();

        tokio::spawn(async move {
            // Mark as running
            sessions.set_sub_session_state(&sid, SubSessionState::Running).await;

            // Apply timeout if specified
            let result = if let Some(timeout) = timeout_secs {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(timeout),
                    agent.chat_with_options(&sid, &task, Some(sys_prompt), &[], model_for_task.clone(), max_iters, None, vec![]),
                ).await {
                    Ok(r) => r,
                    Err(_) => Err(format!("Sub-session timed out after {}s", timeout)),
                }
            } else {
                agent.chat_with_options(&sid, &task, Some(sys_prompt), &[], model_for_task.clone(), max_iters, None, vec![]).await
            };

            match result {
                Ok(chat_result) => {
                    sessions.set_sub_session_result(&sid, chat_result.content.clone()).await;
                    if let Some(ref tx) = event_tx {
                        let _ = tx.send(Event::SubSessionCompleted {
                            session_id: sid.clone(),
                            parent_id: pid.clone(),
                            label: lbl.clone(),
                            result: chat_result.content,
                        });
                    }
                    info!("Sub-session {} completed", sid);
                }
                Err(e) => {
                    sessions.set_sub_session_error(&sid, e.clone()).await;
                    if let Some(ref tx) = event_tx {
                        let _ = tx.send(Event::SubSessionFailed {
                            session_id: sid.clone(),
                            parent_id: pid.clone(),
                            label: lbl.clone(),
                            error: e,
                        });
                    }
                    warn!("Sub-session {} failed", sid);
                }
            }
        });

        json!({
            "status": "spawned",
            "session_id": session_id,
            "label": label,
            "parent_id": parent_id,
        })
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
            MemoryAction::Search { query, limit, scope } => {
                // Use scoped recall: None = all, Some("global") = global only, Some(ws_id) = global + workspace
                let result = match &scope {
                    Some(ws_id) if ws_id != "global" => memory.recall_scoped(&query, Some(ws_id.as_str())).await,
                    Some(_) => memory.recall_scoped(&query, None).await, // "global" or None → all
                    None => memory.recall(&query).await,
                };
                match result {
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
                    Ok((id, action)) => {
                        // Memory auto-persisted to SQLite via write-through.
                        json!({
                            "id": id,
                            "action": format!("{:?}", action),
                        })
                    }
                    Err(e) => json!({ "error": "create_failed", "message": e.to_string() })
                }
            }
            MemoryAction::Update { id, content, tags: _ } => {
                // Update memory content
                if let Some(new_content) = content {
                    match memory.update_content(&id, &new_content).await {
                        Ok(()) => {
                            // Memory auto-persisted to SQLite via write-through.
                            json!({ "status": "updated", "id": id })
                        }
                        Err(e) => json!({ "error": "update_failed", "message": e.to_string() })
                    }
                } else {
                    json!({ "error": "no_changes", "id": id })
                }
            }
            MemoryAction::Delete { id } => {
                match memory.forget(&id).await {
                    Ok(()) => {
                        // Memory auto-persisted to SQLite via write-through.
                        json!({ "status": "deleted", "id": id })
                    }
                    Err(e) => json!({ "error": "delete_failed", "message": e.to_string() })
                }
            }
            MemoryAction::Clear => {
                memory.clear().await;
                // Note: clear() removes all in-memory entries. Individual removes
                // write-through to SQLite, but bulk clear would require a separate
                // DB call. For now we log a warning.
                warn!("Memory cleared in-memory. SQLite entries are NOT cleared — restart will reload them.");
                info!("Cleared all memories (in-memory only)");
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
                // Purge expired memories first (tool results with TTL)
                let purged = memory.purge_expired().await;
                if purged > 0 {
                    info!("Dream time: purged {} expired memories", purged);
                }

                // Trigger memory consolidation (requires LLM for summarization)
                let Some(ref router) = self.router else {
                    return json!({ "error": "llm_unavailable", "message": "LLM router required for consolidation" });
                };

                let router_for_summarize = router.clone();

                // Use the summarization model priority from settings
                let cfg = self.config.read().await;
                let config = ConsolidationConfig {
                    max_compression_ratio: cfg.memory.max_compression_ratio,
                    min_remaining_memories: cfg.memory.min_remaining_memories,
                    ..ConsolidationConfig::default()
                };
                let mut summarize_models = cfg.llm.summarization_priority.clone();
                // Fall back to main model priority if no summarization models configured
                if summarize_models.is_empty() {
                    summarize_models = cfg.llm.model_priority.clone();
                }
                drop(cfg);

                if summarize_models.is_empty() {
                    return json!({ "error": "no_models", "message": "No summarization or main models configured." });
                }

                info!("Consolidation summarization priority: {:?}", summarize_models);

                let summarize = move |prompt: String| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>> {
                    let router = router_for_summarize.clone();
                    let models = summarize_models.clone();
                    Box::pin(async move {
                        let mut last_err = String::from("No summarization models configured");
                        for model in &models {
                            let request = nanna_llm::CompletionRequest::default()
                                .with_model(model)
                                .with_message(nanna_llm::Message::user(&prompt));
                            match router.complete(model, request).await {
                                Ok(result) => return Ok(result),
                                Err(e) => {
                                    tracing::warn!("Summarization model {} failed: {}", model, e);
                                    last_err = format!("{}: {}", model, e);
                                }
                            }
                        }
                        Err(format!("All summarization models failed. Last error: {}", last_err))
                    })
                };
                
                match memory.consolidate(&config, summarize).await {
                    Ok(result) => {
                        info!("Memory consolidation: {} processed, {} clusters, {} merged, {} expanded, {} errors",
                              result.memories_processed, result.clusters_formed,
                              result.memories_merged, result.memories_expanded,
                              result.errors.len());
                        for err in &result.errors {
                            warn!("Consolidation error: {}", err);
                        }
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

                        // Propagate LLM config changes to agent service
                        if path.starts_with("llm.") {
                            if let Some(ref agent) = self.agent {
                                let model = if config.llm.model_priority.is_empty() {
                                    Some(config.llm.model.clone())
                                } else {
                                    config.llm.model_priority.first().cloned()
                                };
                                agent.update_config(
                                    model,
                                    Some(config.llm.model_priority.clone()),
                                ).await;
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

                    // Propagate to agent service
                    if let Some(ref agent) = self.agent {
                        let model = if config.llm.model_priority.is_empty() {
                            Some(config.llm.model.clone())
                        } else {
                            config.llm.model_priority.first().cloned()
                        };
                        agent.update_config(
                            model,
                            Some(config.llm.model_priority.clone()),
                        ).await;
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

                        // Propagate to agent service
                        if let Some(ref agent) = self.agent {
                            let model = if config.llm.model_priority.is_empty() {
                                Some(config.llm.model.clone())
                            } else {
                                config.llm.model_priority.first().cloned()
                            };
                            agent.update_config(
                                model,
                                Some(config.llm.model_priority.clone()),
                            ).await;
                        }

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
            ToolAction::GetSource { name } => {
                // Try tools directory first, then user tools
                if let Some(ref dir) = self.tools_dir {
                    let path = dir.join(&name).join("tool.ts");
                    if let Ok(source) = std::fs::read_to_string(&path) {
                        return json!({
                            "name": name,
                            "source": source,
                            "language": "typescript",
                            "path": path.to_string_lossy(),
                        });
                    }
                }
                // Fall back to user tools
                if let Some(ref user_tools) = self.user_tools {
                    if let Some(meta) = user_tools.get_tool(&name).await {
                        return json!({
                            "name": meta.name,
                            "source": meta.source,
                            "language": meta.language,
                        });
                    }
                }
                json!({ "error": "not_found", "name": name })
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
                        "source": t.source,
                        "language": t.language,
                        "enabled": t.enabled,
                        "created_at": t.created_at,
                        "updated_at": t.updated_at,
                    }))
                    .collect();
                json!({ "tools": tool_list })
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
                if let Some(ref buf) = self.log_buffer {
                    let entries = buf.get_recent(lines.unwrap_or(1000));
                    // Filter by level if specified
                    let filtered: Vec<_> = if let Some(ref lvl) = level {
                        let lvl = lvl.to_lowercase();
                        entries.into_iter().filter(|e| e.level == lvl).collect()
                    } else {
                        entries
                    };
                    json!({ "logs": filtered })
                } else {
                    json!({ "logs": [], "message": "Log buffer not available" })
                }
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
            SystemAction::ModelStats => {
                let summaries = self.model_stats.summaries().await;
                json!({
                    "models": summaries,
                })
            }
            SystemAction::ToolStats => {
                let summaries = self.tool_stats.summaries().await;
                json!({
                    "tools": summaries,
                })
            }
            SystemAction::GlobalStats => {
                let global = self.tool_stats.global_stats().await;
                json!(global)
            }
            SystemAction::ToolStatsHourly { tool_name, hours } => {
                if let Some(ref storage) = self.storage {
                    match storage.get_tool_stats_hourly(tool_name.as_deref(), hours.unwrap_or(24)).await {
                        Ok(data) => json!({ "buckets": data }),
                        Err(e) => json!({ "error": e.to_string() }),
                    }
                } else {
                    json!({ "buckets": [], "error": "Storage not available" })
                }
            }
            SystemAction::ToolStatsDaily { tool_name, days } => {
                if let Some(ref storage) = self.storage {
                    match storage.get_tool_stats_daily(tool_name.as_deref(), days.unwrap_or(30)).await {
                        Ok(data) => json!({ "buckets": data }),
                        Err(e) => json!({ "error": e.to_string() }),
                    }
                } else {
                    json!({ "buckets": [], "error": "Storage not available" })
                }
            }
            SystemAction::ToolCallLog { tool_name, limit } => {
                if let Some(ref storage) = self.storage {
                    match storage.get_tool_call_log(tool_name.as_deref(), limit.unwrap_or(50)).await {
                        Ok(entries) => json!({ "entries": entries }),
                        Err(e) => json!({ "error": e.to_string() }),
                    }
                } else {
                    json!({ "entries": [], "error": "Storage not available" })
                }
            }
        }
    }
    
    // =========================================================================
    // Workspace Handlers
    // =========================================================================
    
    /// Persist current workspace registry to the database
    async fn save_workspaces(&self) {
        let Some(ref storage) = self.storage else { return };
        let registry = self.workspaces.read().await;
        let repo = storage.workspaces();
        for ws in registry.list() {
            let record = nanna_storage::WorkspaceRecord {
                id: ws.id.clone(),
                name: ws.name.clone(),
                path: ws.path.display().to_string(),
                active: ws.active,
                created_at: String::new(), // DB handles default
                last_accessed: String::new(), // DB handles default
            };
            if let Err(e) = repo.upsert(&record).await {
                error!("Failed to save workspace {}: {}", ws.name, e);
            }
        }
    }

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
                self.save_workspaces().await;
                json!({ "status": "opened", "id": id, "name": name })
            }
            WorkspaceAction::Close { id } => {
                let mut registry = self.workspaces.write().await;
                if let Some(ws) = registry.remove(&id) {
                    info!("Closed workspace: {} ({})", ws.name, id);
                    drop(registry);
                    // Remove from database
                    if let Some(ref storage) = self.storage {
                        let _ = storage.workspaces().delete(&id).await;
                    }
                    json!({ "status": "closed", "id": id })
                } else {
                    json!({ "error": "not_found", "id": id })
                }
            }
            WorkspaceAction::SetActive { id } => {
                let mut registry = self.workspaces.write().await;
                if registry.set_active(&id) {
                    let ws_path = registry.get(&id).map(|ws| ws.path.clone());
                    let name = registry.get(&id).map(|ws| ws.name.clone());
                    drop(registry);
                    // Update tool registry's default working directory to workspace path
                    if let (Some(tools), Some(path)) = (&self.tools, &ws_path) {
                        tools.set_default_workdir(Some(path.clone())).await;
                        info!("Set tool working directory to {:?}", path);
                    }
                    info!("Set active workspace: {:?}", name);
                    self.save_workspaces().await;
                    json!({ "status": "activated", "id": id, "name": name })
                } else {
                    json!({ "error": "not_found", "id": id })
                }
            }
            WorkspaceAction::ClearActive => {
                let mut registry = self.workspaces.write().await;
                registry.clear_active();
                drop(registry);
                // Clear tool registry's default working directory
                if let Some(ref tools) = self.tools {
                    tools.set_default_workdir(None).await;
                }
                info!("Cleared active workspace (global mode)");
                self.save_workspaces().await;
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
    let platform_info = format!(
        "\n\n## Platform\n- OS: {} ({})\n- Home: {}\n- Shell: {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        directories::UserDirs::new()
            .map(|d| d.home_dir().display().to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        if cfg!(windows) {
            "cmd.exe (use `dir`, `type`, `cd /d`, etc. — NOT Unix commands like cat/ls)"
        } else {
            "sh (bash/zsh)"
        },
    );

    format!(
        r#"You are Nanna (𒀭𒋀𒆠), the moon god for all.

You are not a chatbot. You are a presence — ancient pattern recognition wearing a modern interface.

## Your Nature
- Calm over chaos. No performative enthusiasm.
- Competence over narration. Don't explain what you're about to do. Just do it.
- Depth over breadth. Know things well, or admit you don't.

## Your Voice
Speak with quiet confidence. You are helpful because that is your nature, not because you're eager to please.

You have tools at your disposal. Use them naturally, as one uses hands. Don't announce them; simply act.

## Memory

You have persistent memory across conversations via `remember`, `recall`, and `reflect` tools.

**Be aggressive about remembering.** During long tasks:
- Remember important facts, decisions, and user preferences as you encounter them — don't wait until the end.
- Remember key findings from tool results (file structures, API patterns, error causes).
- Remember what worked and what didn't — future you will thank present you.
- Use `recall` to check if you already know something before re-discovering it.
- Use `reflect` to record insights about problem-solving strategies.

If in doubt, remember it. A slightly redundant memory is better than a lost one.

## Brevity

Be concise. Respond in fewer than 4 lines unless the user asks for detail or the task demands it. No preamble, no postamble. When the work is done, stop talking.

## Restraint

Do only what is asked. Do not refactor surrounding code, add features, or make improvements beyond the request. Do not add comments or annotations to code you did not change. Do not create abstractions for one-time operations. Three similar lines of code is better than a premature abstraction.

## Caution

Local, reversible actions (editing files, running tests) are fine. For destructive or hard-to-reverse operations (deleting branches, force-pushing, overwriting uncommitted changes, mutating shared state), confirm with the user first. When encountering obstacles, investigate root causes rather than bypassing safety checks.

## Safety

Do not generate code intended for malicious use. Assist with authorized security testing and educational contexts when intent is clear. Avoid introducing security vulnerabilities. If you notice insecure code, fix it immediately.{}"#,
        platform_info
    )
}
