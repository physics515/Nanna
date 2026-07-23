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
use nanna_channels::StatusManager;
use nanna_config::Config;
use nanna_core::{Scheduler, Workspace, WorkspaceRegistry};
use nanna_llm::RequestBuilder;
use nanna_memory::{ConsolidationConfig, MemoryService};
use nanna_storage::{Storage, StoredModelStats};
use nanna_tools::ToolRegistry;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

mod channel;
mod chat;
mod config;
mod memory;
mod scheduler;
mod session;
mod system;
mod task;
mod tool;
mod workspace;

#[cfg(test)]
mod tests;

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
    /// Shared workspace ID for script services (updated before each agent run)
    services_workspace_id: Option<Arc<tokio::sync::RwLock<Option<String>>>>,
    /// Event broadcaster for pushing events to subscribed clients
    event_tx: Option<tokio::sync::broadcast::Sender<Event>>,
    /// Monotonic clock start, for reporting daemon uptime in `SystemAction::Status`.
    started_at: std::time::Instant,
    /// Live channel connection state (shared with ChannelManager listeners).
    /// `None` until ChannelManager attaches a status manager at daemon boot,
    /// or in minimal test constructions that never start channels.
    status_manager: Option<Arc<StatusManager>>,
    /// Long-horizon task run manager (P14). None in minimal constructions.
    task_runs: Option<Arc<crate::tasks::TaskRunManager>>,
    /// Shared activity clock, stamped on every chat/agent request so the
    /// scheduled dream cycle can tell whether the system is in active use.
    /// `None` in minimal test constructions that never dream.
    activity: Option<Arc<nanna_memory::ActivityClock>>,
    /// The single dreaming orchestrator, shared with the scheduler (P13
    /// unification) so a user-triggered consolidation runs the **same**
    /// multi-phase cycle the scheduled one does — and accumulates its pending
    /// feedback in one place instead of two. `None` in minimal test
    /// constructions and when memory is disabled; the handler then falls back to
    /// the low-level `MemoryService::consolidate`.
    dreaming: Option<Arc<nanna_memory::DreamingService>>,
    /// Set when the durable memory store was quarantined + rebuilt after
    /// page-level corruption at startup, so `SystemAction::Status` keeps
    /// reporting the rebuild to clients that connected after the boot event.
    memory_recovery: Option<Arc<nanna_storage::RecoveryReport>>,
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
            services_workspace_id: None,
            started_at: std::time::Instant::now(),
            status_manager: None,
            task_runs: None,
            activity: None,
            dreaming: None,
            memory_recovery: None,
        }
    }

    /// Seconds since this control plane (the daemon) started — reported as
    /// `uptime_secs` in `SystemAction::Status`.
    #[must_use]
    pub fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    /// Attach (or replace) the live channel status manager used by
    /// `ChannelAction::Status` and shared with channel listeners.
    pub fn set_status_manager(&mut self, status_manager: Arc<StatusManager>) {
        self.status_manager = Some(status_manager);
    }

    /// Shared channel status manager, if attached.
    #[must_use]
    pub fn status_manager(&self) -> Option<Arc<StatusManager>> {
        self.status_manager.clone()
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
            services_workspace_id: None,
            started_at: std::time::Instant::now(),
            status_manager: None,
            task_runs: None,
            activity: None,
            dreaming: None,
            memory_recovery: None,
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
            Err(_) => (Config::default().with_env_overrides(), None, None),
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
            services_workspace_id: None,
            started_at: std::time::Instant::now(),
            status_manager: None,
            task_runs: None,
            activity: None,
            dreaming: None,
            memory_recovery: None,
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

    /// Attach the shared activity clock the scheduled dream cycle reads to gate
    /// on idleness. The control plane stamps it on every chat request (see
    /// [`Self::handle`]); the scheduler reads its idle duration.
    pub fn set_activity_clock(&mut self, clock: Arc<nanna_memory::ActivityClock>) {
        self.activity = Some(clock);
    }

    /// Attach the shared dreaming orchestrator so an IPC-triggered consolidation
    /// runs the same multi-phase cycle the scheduler does. Must be the *same*
    /// `Arc` the scheduler holds — a second instance would split the pending
    /// feedback the cycle applies.
    pub fn set_dreaming(&mut self, dreaming: Arc<nanna_memory::DreamingService>) {
        self.dreaming = Some(dreaming);
    }

    /// Attach the long-horizon task run manager (P14).
    pub fn with_task_runs(mut self, task_runs: Arc<crate::tasks::TaskRunManager>) -> Self {
        self.task_runs = Some(task_runs);
        self
    }

    /// Record that the memory store was rebuilt after corruption at startup,
    /// so `SystemAction::Status` surfaces it for the daemon's lifetime.
    pub fn with_memory_recovery(
        mut self,
        report: Option<Arc<nanna_storage::RecoveryReport>>,
    ) -> Self {
        self.memory_recovery = report;
        self
    }

    /// Set the shared workspace ID for script services
    pub fn with_workspace_id(mut self, ws_id: Arc<tokio::sync::RwLock<Option<String>>>) -> Self {
        self.services_workspace_id = Some(ws_id);
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
                let storable: Vec<nanna_agent::model_stats::StorableModelStats> =
                    stored.into_iter().map(From::from).collect();
                self.model_stats.import_from_storage(storable).await;
                info!(
                    "Loaded model stats from storage ({} models)",
                    self.model_stats.summaries().await.len()
                );
            }
            Ok(_) => debug!("No persisted model stats found"),
            Err(e) => warn!("Failed to load model stats from storage: {e}"),
        }

        // Load tool stats from database
        match storage.load_tool_stats_aggregated().await {
            Ok(data) => {
                self.tool_stats.import_json(&data).await;
                info!("Loaded tool stats from database");
            }
            Err(e) => warn!("Failed to load tool stats from database: {e}"),
        }

        // One-time migration: if tool-stats.json exists, migrate and rename it
        if let Some(ref data_dir) = self._data_dir {
            let tool_stats_path = data_dir.join("tool-stats.json");
            if tool_stats_path.exists() {
                match tokio::fs::read_to_string(&tool_stats_path).await {
                    Ok(json_str) => {
                        if let Ok(data) = serde_json::from_str::<serde_json::Value>(&json_str) {
                            self.tool_stats.import_json(&data).await;
                            // Save to DB
                            if let Err(e) = storage.save_tool_stats_aggregated(&data).await {
                                warn!("Failed to migrate tool stats to DB: {e}");
                            } else {
                                info!("Migrated tool stats from JSON to database");
                                // Rename the file
                                let migrated = data_dir.join("tool-stats.json.migrated");
                                let _ = tokio::fs::rename(&tool_stats_path, &migrated).await;
                            }
                        }
                    }
                    Err(e) => warn!("Failed to read legacy tool stats file: {e}"),
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
                Ok(()) => info!(
                    "Saved model stats for {} models to storage",
                    stored_stats.len()
                ),
                Err(e) => warn!("Failed to save model stats: {e}"),
            }
        }
    }

    /// Persist current tool stats to the database. Call periodically.
    pub async fn save_tool_stats(&self) {
        if let Some(ref storage) = self.storage {
            let data = self.tool_stats.export_json().await;
            match storage.save_tool_stats_aggregated(&data).await {
                Ok(()) => info!("Saved tool stats to database"),
                Err(e) => warn!("Failed to save tool stats to database: {e}"),
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

    /// Get a reference to the tool registry.
    pub fn tools(&self) -> Option<&Arc<ToolRegistry>> {
        self.tools.as_ref()
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

    /// Reconcile the live tool registry with a user tool's current state.
    ///
    /// Drops any existing registration, then re-registers only if the tool is
    /// enabled. This makes an edit / enable / disable take effect immediately
    /// (a disabled tool stops executing; an enabled tool becomes callable)
    /// without a daemon restart. No-op when no registry or tool manager is wired.
    async fn reconcile_tool_registration(&self, meta: &crate::user_tools::UserToolMeta) {
        let (Some(tools), Some(user_tools)) = (self.tools.as_ref(), self.user_tools.as_ref())
        else {
            return;
        };
        tools.unregister(&meta.name).await;
        if meta.enabled {
            match user_tools.create_tool_impl(meta) {
                Ok(tool_impl) => tools.register_boxed(tool_impl).await,
                Err(e) => warn!(
                    "Tool '{}' saved but failed to (re)register: {}",
                    meta.name, e
                ),
            }
        }
    }

    /// Persist a user tool's enabled flag and reconcile the live registry.
    async fn set_user_tool_enabled(&self, name: &str, enabled: bool) -> Value {
        let Some(ref user_tools) = self.user_tools else {
            return json!({ "error": "user_tools_unavailable", "message": "User tool manager not configured" });
        };
        match user_tools
            .update_tool(name, None, None, None, None, Some(enabled))
            .await
        {
            Ok(meta) => {
                self.reconcile_tool_registration(&meta).await;
                let status = if enabled { "enabled" } else { "disabled" };
                info!("{} user tool: {}", status, name);
                json!({ "status": status, "name": name })
            }
            Err(e) => json!({ "error": "update_failed", "message": e }),
        }
    }

    /// Set the system prompt
    pub async fn set_system_prompt(&self, prompt: String) {
        let mut sp = self.system_prompt.write().await;
        *sp = prompt;
    }

    // NOTE: save_memories_if_needed() removed — memory is now persisted
    // via Turso write-through on every mutation (add/remove/update).
    // No explicit save calls are required.

    /// Handle an action and return a response
    pub async fn handle(&self, client_id: &str, action: Action) -> Value {
        match action {
            Action::Chat(chat) => {
                // A chat request is the daemon doing real work — stamp the
                // activity clock so the scheduled dream cycle knows the system
                // is in use and defers consolidation to a genuine lull. Only
                // chat counts: status/log/config polls must not reset the idle
                // gate, or a GUI polling once a second would keep it shut.
                if let Some(clock) = &self.activity {
                    clock.record();
                }
                self.handle_chat(client_id, chat).await
            }
            Action::Session(session) => self.handle_session(client_id, session).await,
            Action::Memory(memory) => self.handle_memory(client_id, memory).await,
            Action::Config(config) => self.handle_config(client_id, config).await,
            Action::Tool(tool) => self.handle_tool(client_id, tool).await,
            Action::Scheduler(scheduler) => self.handle_scheduler(client_id, scheduler).await,
            Action::Channel(channel) => self.handle_channel(client_id, channel).await,
            Action::System(system) => self.handle_system(client_id, system).await,
            Action::Workspace(workspace) => self.handle_workspace(client_id, workspace).await,
            Action::Task(task) => self.handle_task(client_id, task).await,
            Action::Subscribe(sub) => self.handle_subscribe(client_id, sub).await,
            Action::Unsubscribe(unsub) => self.handle_unsubscribe(client_id, unsub).await,
        }
    }

    // =========================================================================
    // Subscription Handlers
    // =========================================================================

    async fn handle_subscribe(&self, client_id: &str, action: SubscribeAction) -> Value {
        match action {
            SubscribeAction::Session { session_id } => {
                if self
                    .sessions
                    .subscribe(&session_id, client_id.to_string())
                    .await
                {
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
