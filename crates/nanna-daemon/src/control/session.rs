//! Session and sub-session handlers for the [`ControlPlane`].

use std::fmt::Write as _;

use super::{json, info, warn, debug, ControlPlane, Event, SessionAction, Value, SubSessionState, Arc, SubSessionInfo, ToolRegistry};

/// The sub-agent parameters `SessionAction::SpawnSubSession` carries.
struct SubSessionSpawn {
    task: String,
    label: Option<String>,
    parent_id: Option<String>,
    model: Option<String>,
    max_iterations: Option<usize>,
    timeout_secs: Option<u64>,
    system_prompt: Option<String>,
}

/// Everything one spawned sub-agent run owns.
struct SubSessionRun {
    agent: Arc<crate::agent_service::AgentService>,
    sessions: Arc<crate::session::SessionManager>,
    event_tx: Option<tokio::sync::broadcast::Sender<Event>>,
    workspaces: Arc<tokio::sync::RwLock<nanna_core::WorkspaceRegistry>>,
    model: Option<String>,
    max_iterations: Option<usize>,
    timeout_secs: Option<u64>,
    session_id: String,
    label: Option<String>,
    parent_id: Option<String>,
    task: String,
    system_prompt: String,
    /// The parent's workdir snapshot, bound to this run's own session.
    parent_workdir: Option<std::path::PathBuf>,
}

impl SubSessionRun {
    /// Run the sub-agent to completion and record its outcome. Must execute
    /// inside a `ToolRegistry::with_run_session` scope for `session_id`.
    async fn run(self) {
        let Self {
            agent,
            sessions,
            event_tx,
            workspaces,
            model: model_for_task,
            max_iterations: max_iters,
            timeout_secs,
            session_id: sid,
            label: lbl,
            parent_id: pid,
            task,
            system_prompt: sys_prompt,
            parent_workdir,
        } = self;
        let task_for_extraction = task.clone();

        // Mark as running
        sessions.set_sub_session_state(&sid, SubSessionState::Running).await;

        // Set per-session workdir for the sub-agent from the parent's
        // snapshot. Explicitly keyed on `sid`, and now inside the scope
        // that reads it back.
        if let Some(ref wd) = parent_workdir {
            agent.tools().set_session_workdir(&sid, wd.clone()).await;
        }

        let chat_options = crate::agent_service::ChatOptions {
            model_override: model_for_task.clone(),
            max_iterations_override: max_iters,
            workspace_id: None,
            attachments: vec![],
            is_sub_agent: true,
        };

        // Apply timeout if specified
        let result = if let Some(timeout) = timeout_secs {
            tokio::time::timeout(
                std::time::Duration::from_secs(timeout),
                agent.chat_with_options(&sid, &task, Some(sys_prompt), &[], chat_options),
            )
            .await
            .unwrap_or_else(|_| {
                Err(crate::agent_service::ChatError {
                    message: format!("Sub-session timed out after {timeout}s"),
                    partial_result: None,
                })
            })
        } else {
            agent.chat_with_options(&sid, &task, Some(sys_prompt), &[], chat_options).await
        };

        match result {
            Ok(chat_result) => {
                sessions.set_sub_session_result(&sid, chat_result.content.clone()).await;
                if let Some(ref tx) = event_tx {
                    let _ = tx.send(Event::SubSessionCompleted {
                        session_id: sid.clone(),
                        parent_id: pid.clone(),
                        label: lbl.clone(),
                        result: chat_result.content.clone(),
                    });
                }
                info!("Sub-session {} completed", sid);

                // Extract project knowledge into root AGENTS.md (standard project file)
                let ws_info = {
                    let reg = workspaces.read().await;
                    reg.active().map(|ws| (
                        ws.path.clone(),
                        ws.context.agents.clone(),
                    ))
                };
                if let Some((ws_path, current_agents)) = ws_info {
                    let result_text = chat_result.content;
                    Self::spawn_agents_md_extraction(
                        agent.clone(),
                        ws_path,
                        current_agents,
                        task_for_extraction.clone(),
                        result_text,
                    );
                }
            }
            Err(e) => {
                // If there's partial work, persist it as the sub-session result
                let error_msg = if let Some(ref partial) = e.partial_result {
                    sessions.set_sub_session_result(&sid, partial.content.clone()).await;
                    format!("{} (partial result preserved)", e.message)
                } else {
                    e.message.clone()
                };
                sessions.set_sub_session_error(&sid, error_msg.clone()).await;
                if let Some(ref tx) = event_tx {
                    let _ = tx.send(Event::SubSessionFailed {
                        session_id: sid.clone(),
                        parent_id: pid.clone(),
                        label: lbl.clone(),
                        error: error_msg,
                    });
                }
                warn!("Sub-session {} failed", sid);
            }
        }

        // The binding outlives nothing: leaving it would grow the map one
        // permanent entry per sub-agent the daemon ever runs, and a stale
        // entry now WINS over the global default for that id.
        agent.tools().clear_session_workdir(&sid).await;
    }

    /// Extract project knowledge from a finished sub-agent task into the
    /// workspace's root AGENTS.md, as a detached agent turn.
    fn spawn_agents_md_extraction(
        agent_for_extract: Arc<crate::agent_service::AgentService>,
        ws_path: std::path::PathBuf,
        current_agents: Option<String>,
        extraction_task: String,
        result_text: String,
    ) {
        // The extraction is an agent turn of its own, so it
        // gets its own scope: without one it would run under
        // whatever the shared slot named and file its
        // session-scoped tool state against a stranger.
        let extract_session = format!("extract-{}", uuid::Uuid::new_v4());
        let extract_scope = extract_session.clone();
        tokio::spawn(ToolRegistry::with_run_session(extract_scope, Box::pin(async move {
            let agents_ctx = current_agents.as_deref().unwrap_or("(empty)");
            let extract_prompt = format!(
                "You maintain the project's root AGENTS.md (agent instructions for this repo).\n\
                 Task just completed: {extraction_task}\n\n\
                 Result summary:\n{result_text}\n\n\
                 Current AGENTS.md:\n{agents_ctx}\n\n\
                 Reply with ONLY new bullet points worth adding to AGENTS.md \
                 (build commands, architecture, pitfalls). If nothing lasting, reply NONE.\n\
                 Keep under 800 characters. No preamble."
            );
            match agent_for_extract
                .chat(&extract_session, &extract_prompt, None, &[])
                .await
            {
                Ok(extract_result) => {
                    let agents_new = extract_result.content.trim();
                    if !agents_new.eq_ignore_ascii_case("NONE")
                        && !agents_new.is_empty()
                        && agents_new.len() < 2000
                    {
                        let agents_path = ws_path.join("AGENTS.md");
                        let existing = tokio::fs::read_to_string(&agents_path)
                            .await
                            .unwrap_or_default();
                        let updated = if existing.trim().is_empty() {
                            format!("# AGENTS.md\n\n### Learned\n{agents_new}\n")
                        } else {
                            format!("{}\n\n### Learned\n{}\n", existing.trim_end(), agents_new)
                        };
                        if let Err(e) = tokio::fs::write(&agents_path, updated).await {
                            warn!("Failed to update AGENTS.md: {e}");
                        } else {
                            info!("Updated AGENTS.md with knowledge from sub-agent task");
                        }
                    }
                }
                Err(e) => {
                    debug!("Knowledge extraction skipped (LLM error): {}", e.message);
                }
            }
        })));
    }
}

impl ControlPlane {
    // =========================================================================
    // Session Handlers
    // =========================================================================
    
    /// Tell every connected client about a session lifecycle change.
    ///
    /// These events were declared in the protocol and never sent, so a session
    /// renamed or deleted by one client (the CLI, a second window) never reached the
    /// others. Best-effort like every broadcast here: no receivers is not an error.
    fn notify_session_event(&self, event: Event) {
        debug_assert!(
            event.session_id().is_some(),
            "a lifecycle event names its session"
        );
        debug_assert!(
            matches!(
                event,
                Event::SessionCreated { .. }
                    | Event::SessionDeleted { .. }
                    | Event::SessionRenamed { .. }
            ),
            "only session lifecycle events go through here"
        );
        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(event);
        }
    }

    pub(super) async fn handle_session(&self, client_id: &str, action: SessionAction) -> Value {
        match action {
            SessionAction::List => {
                let mut sessions = self.sessions.list().await;
                // Sort by created_at descending (newest first)
                sessions.sort_by_key(|s| std::cmp::Reverse(s.created_at));
                json!({ "sessions": sessions })
            }
            SessionAction::ListByWorkspace { workspace_id } => {
                let mut sessions = self.sessions.list().await;
                // Filter by workspace: None = global only, Some(id) = that workspace
                sessions.retain(|s| s.workspace_id == workspace_id);
                sessions.sort_by_key(|s| std::cmp::Reverse(s.created_at));
                json!({ "sessions": sessions })
            }
            SessionAction::Get { id } => {
                self.sessions.get(&id).await.map_or_else(
                    || json!({ "error": "not_found", "message": format!("Session {} not found", id) }),
                    |session| json!({ "session": session }),
                )
            }
            SessionAction::Create { name } => self.session_create(client_id, name, None).await,
            SessionAction::CreateInWorkspace { name, workspace_id } => {
                self.session_create(client_id, name, workspace_id).await
            }
            SessionAction::Rename { id, name } => self.session_rename(id, name).await,
            SessionAction::Delete { id } => self.session_delete(id).await,
            SessionAction::DeleteAll => self.delete_all_sessions().await,
            SessionAction::Clear { id } => {
                if self.sessions.clear(&id).await {
                    json!({ "status": "cleared", "id": id })
                } else {
                    json!({ "error": "not_found", "message": format!("Session {} not found", id) })
                }
            }
            SessionAction::History { id, limit, before: _ } => self.session_history(id, limit).await,
            SessionAction::Export { id, format } => self.session_export(id, format).await,
            SessionAction::Switch { id } => self.session_switch(client_id, id).await,
            SessionAction::GetRunState { id, light } => self.session_run_state(id, light).await,
            SessionAction::Liveness { id } => self.session_liveness(id).await,
            SessionAction::SetWorkspace { id, workspace_id } => {
                if self.sessions.set_workspace(&id, workspace_id.clone()).await {
                    json!({ "ok": true, "session_id": id, "workspace_id": workspace_id })
                } else {
                    json!({ "error": "session_not_found", "message": format!("Session {} not found", id) })
                }
            }
            SessionAction::SetModel { id, model } => {
                if self.sessions.set_chat_model(&id, model.clone()).await {
                    json!({ "ok": true, "session_id": id, "model": model })
                } else {
                    json!({ "error": "session_not_found", "message": format!("Session {} not found", id) })
                }
            }
            SessionAction::SetTools { id, tools } => {
                if self.sessions.set_chat_tools(&id, tools.clone()).await {
                    json!({ "ok": true, "session_id": id, "tools": tools })
                } else {
                    json!({ "error": "session_not_found", "message": format!("Session {} not found", id) })
                }
            }
            SessionAction::Fork { id, name } => self.fork_session(id, name).await,

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
                self.handle_spawn_sub_session(SubSessionSpawn {
                    task,
                    label,
                    parent_id,
                    model,
                    max_iterations,
                    timeout_secs,
                    system_prompt,
                })
                .await
            }

            SessionAction::SendToSubSession { target, message } => self.send_to_sub_session(client_id, target, message).await,

            SessionAction::ListSubSessions { parent_id } => {
                let subs = self.sessions.list_sub_sessions(parent_id.as_deref()).await;
                json!({ "sub_sessions": subs })
            }

            SessionAction::KillSubSession { target } => self.kill_sub_session(target).await,

            SessionAction::GetSubSessionStatus { target } => self.sub_session_status(target).await,
        }
    }
    
    // =========================================================================
    // Sub-Session Handlers (#72)
    // =========================================================================

    async fn handle_spawn_sub_session(&self, spawn: SubSessionSpawn) -> Value {
        let SubSessionSpawn {
            task,
            label,
            parent_id,
            model,
            max_iterations,
            timeout_secs,
            system_prompt,
        } = spawn;
        let Some(ref agent) = self.agent else {
            return json!({ "error": "agent_unavailable", "message": "Agent service not configured" });
        };

        // Check for duplicate labels
        if let Some(ref lbl) = label
            && let Some(existing) = self.sessions.find_sub_session_by_label(lbl).await
            && matches!(existing.state, SubSessionState::Spawning | SubSessionState::Running | SubSessionState::Waiting)
        {
            return json!({
                "error": "duplicate_label",
                "message": format!("Sub-session with label '{}' already running ({})", lbl, existing.session_id),
            });
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

        // Build system prompt (include workspace context so sub-agents know the codebase)
        let sys_prompt =
            system_prompt.unwrap_or_else(|| self.default_sub_session_prompt(&task));

        // Inherit the parent's root. Read it AS the parent wherever we know who
        // that is — outside any run scope this call reads the shared slot,
        // which names whichever chat wrote it last, not necessarily the session
        // that spawned us.
        let parent_workdir = match parent_id {
            Some(ref parent) => {
                ToolRegistry::with_run_session(parent.clone(), agent.tools().default_workdir())
                    .await
            }
            None => agent.tools().default_workdir().await,
        };

        // The whole sub-agent run scopes to its own session, carried by its own
        // future. The snapshot above used to be the workaround for the shared
        // slot; the scope is the cure, so the run no longer writes that slot at
        // all and a chat turn starting mid-run cannot steal the binding back.
        // Boxed for the same reason the scheduled path boxes — the run future
        // is large and would otherwise sit inline in the spawned task's state
        // machine.
        let run = SubSessionRun {
            agent: agent.clone(),
            sessions: self.sessions.clone(),
            event_tx: self.event_tx.clone(),
            workspaces: self.workspaces.clone(),
            model,
            max_iterations,
            timeout_secs,
            session_id: session_id.clone(),
            label: label.clone(),
            parent_id: parent_id.clone(),
            task,
            system_prompt: sys_prompt,
            parent_workdir,
        };
        let scope_sid = session_id.clone();
        tokio::spawn(ToolRegistry::with_run_session(scope_sid, Box::pin(run.run())));

        json!({
            "status": "spawned",
            "session_id": session_id,
            "label": label,
            "parent_id": parent_id,
        })
    }

    /// The system prompt a sub-agent gets when the caller supplies none: the
    /// daemon prompt, the global persona, and the active workspace's context.
    fn default_sub_session_prompt(&self, task: &str) -> String {
        let base = self.system_prompt.blocking_read().clone();

        // Global persona / user profile
        let persona_inj = {
            let cfg = self.config.blocking_read();
            nanna_core::GlobalPersona {
                persona: cfg.agent.persona.clone(),
                user_profile: cfg.agent.user_profile.clone(),
            }
            .build_system_prompt_injection()
        };

        // Inject workspace context so sub-agents see README.md, AGENTS.md, ROADMAP.md
        let ws_context = {
            let registry = self.workspaces.blocking_read();
            registry.active()
                .map(|ws| {
                    let mut ctx = String::new();
                    let ws_path = ws.path.display();
                    let _ = write!(
                        ctx,
                        "

## Active Workspace
                            **Root directory: {ws_path}**
                            All file operations and commands MUST use this directory as the base.
"
                    );
                    let injection = ws.context.build_system_prompt_injection();
                    if !injection.is_empty() {
                        let _ = write!(ctx, "
{injection}");
                    }
                    ctx
                })
                .unwrap_or_default()
        };

        let persona_block = if persona_inj.is_empty() {
            String::new()
        } else {
            format!("

{persona_inj}")
        };

        format!("{base}{persona_block}{ws_context}

You are a sub-agent. All tools are pre-activated — use them directly (no need to call discover_tools). Execute the task immediately and return results when done.

Your task: {task}")
    }

    /// `SessionAction::Liveness`: the session's liveness ledger snapshot plus queue depth.
    async fn session_liveness(&self, id: String) -> Value {
        // "Working, wedged, or finished" from the daemon's own ledger
        // (P22) — constant-size, safe to poll, no log greps. A session
        // no turn has ever touched gets an honest idle default rather
        // than an error: "never ran" IS its liveness. The pending
        // count rides along so a poller can also see queued
        // interjections that no live run is consuming — the 50-minute
        // wedge signature from 2026-08-10.
        let snapshot = self.liveness.get(&id).map_or_else(
            || {
                json!({
                    "running": false,
                    "phase": "idle",
                    "awaiting": "idle — no turn this daemon lifetime",
                })
            },
            |live| {
                serde_json::to_value(live.snapshot())
                    .unwrap_or_else(|_| json!({ "running": false }))
            },
        );
        let pending = self.chat_runs.pending_for(&id).await.len().await;
        let mut body = snapshot;
        if let Some(map) = body.as_object_mut() {
            map.insert("session_id".to_string(), json!(id));
            map.insert("pending_interjections".to_string(), json!(pending));
            map.insert(
                "beat_interval_secs".to_string(),
                json!(crate::liveness::beat_interval_secs()),
            );
        }
        body
    }

    /// `SessionAction::History`: the newest `limit` messages (all by default), oldest first.
    async fn session_history(&self, id: String, limit: Option<usize>) -> Value {
        if let Some(session) = self.sessions.get(&id).await {
            // No limit = the WHOLE session. A long-horizon run's chat
            // page must be able to reload every message after
            // navigation — a silent default cap made remounts drop
            // history (observed live: a 4-hour mission showing only
            // its final slice).
            let mut messages: Vec<_> = session.messages.iter()
                .rev()
                .take(limit.unwrap_or(usize::MAX))
                .cloned()
                .collect();
            messages.reverse(); // Back to chronological order (oldest first)
            json!({ "messages": messages })
        } else {
            json!({ "error": "not_found", "message": format!("Session {} not found", id) })
        }
    }

    /// `SessionAction::Export`: render one session's transcript as a document.
    async fn session_export(&self, id: String, format: crate::protocol::ExportFormat) -> Value {
        // Rendered here, by the store's owner, so every client — the
        // CLI's `nanna export` today, a GUI button later — gets the
        // same document instead of re-deriving the transcript.
        let Some(session) = self.sessions.get(&id).await else {
            return json!({ "error": "not_found", "message": format!("Session {} not found", id) });
        };
        match crate::export::export_session(&session, format, chrono::Utc::now()) {
            Ok(document) => json!({
                "format": format,
                "filename": document.filename,
                "content": document.content,
            }),
            Err(e) => json!({
                "error": "export_failed",
                "message": format!("Session {id} could not be exported: {e}"),
            }),
        }
    }

    /// `SessionAction::GetSubSessionStatus`: a sub-session's metadata, message count and pending mail.
    async fn sub_session_status(&self, target: String) -> Value {
        if let Some(info) = self.sessions.resolve_sub_session(&target).await {
            // Also get session message count
            let msg_count = self.sessions.get(&info.session_id).await
                .map_or(0, |s| s.messages.len());
            // Non-destructive peek: a status check must never consume the
            // session's pending inter-session messages.
            let mailbox_count = self.sessions.peek_mailbox(&info.session_id).await.len();
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

    /// `SessionAction::KillSubSession`: cancel a sub-session and announce it.
    async fn kill_sub_session(&self, target: String) -> Value {
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

    /// `SessionAction::SendToSubSession`: queue a message in a sub-session's mailbox.
    async fn send_to_sub_session(&self, client_id: &str, target: String, message: String) -> Value {
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

    /// `SessionAction::Fork`: copy a session's messages into a new session.
    async fn fork_session(&self, id: String, name: Option<String>) -> Value {
        if let Some(original) = self.sessions.get(&id).await {
            let mut forked = self.sessions.create(
                name.or_else(|| original.name.as_ref().map(|n| format!("{n} (copy)")))
            ).await;
            // Copy messages
            forked.messages = original.messages.clone();
            self.sessions.update(forked.clone()).await;
            json!({ "session": forked })
        } else {
            json!({ "error": "not_found", "message": format!("Session {} not found", id) })
        }
    }

    /// `SessionAction::DeleteAll`: delete every session, announcing each deletion.
    async fn delete_all_sessions(&self) -> Value {
        // The store reports only a count, so read the ids first. A session created in
        // the gap between the two calls is deleted without an event; clients re-fetch
        // the list on any deletion, so the window costs one stale row at most.
        let ids: Vec<crate::SessionId> = self.sessions.list().await.into_iter().map(|s| s.id).collect();
        let count = self.sessions.delete_all().await;
        for id in ids {
            self.notify_session_event(Event::SessionDeleted { id });
        }
        json!({ "status": "deleted", "count": count })
    }

    /// `SessionAction::Create` / `CreateInWorkspace`: create the session (in
    /// `workspace_id`, or global when `None` — exactly what
    /// `SessionManager::create` does), subscribe the creating client, and
    /// announce it.
    async fn session_create(
        &self,
        client_id: &str,
        name: Option<String>,
        workspace_id: Option<String>,
    ) -> Value {
        let session = self.sessions.create_in_workspace(name, workspace_id).await;
        // Auto-subscribe the creating client
        self.sessions.subscribe(&session.id, client_id.to_string()).await;
        self.notify_session_event(Event::SessionCreated {
            id: session.id.clone(),
            name: session.name.clone(),
        });
        json!({ "session": session })
    }

    /// `SessionAction::Rename`: rename a session and announce it.
    async fn session_rename(&self, id: String, name: String) -> Value {
        if self.sessions.rename(&id, name.clone()).await {
            self.notify_session_event(Event::SessionRenamed {
                id: id.clone(),
                name: name.clone(),
            });
            json!({ "status": "renamed", "id": id, "name": name })
        } else {
            json!({ "error": "not_found", "message": format!("Session {} not found", id) })
        }
    }

    /// `SessionAction::Delete`: delete a session and announce it.
    async fn session_delete(&self, id: String) -> Value {
        if self.sessions.delete(&id).await {
            self.notify_session_event(Event::SessionDeleted { id: id.clone() });
            json!({ "status": "deleted", "id": id })
        } else {
            json!({ "error": "not_found", "message": format!("Session {} not found", id) })
        }
    }

    /// `SessionAction::Switch`: subscribe the client to an existing session.
    async fn session_switch(&self, client_id: &str, id: String) -> Value {
        if self.sessions.get(&id).await.is_some() {
            // Subscribe client to this session
            self.sessions.subscribe(&id, client_id.to_string()).await;
            json!({ "status": "switched", "session_id": id })
        } else {
            json!({ "error": "not_found", "message": format!("Session {} not found", id) })
        }
    }

    /// `SessionAction::GetRunState`: the in-flight run's streaming state.
    async fn session_run_state(&self, id: String, light: bool) -> Value {
        if let Some(ref agent) = self.agent {
            let state = agent.get_run_state(&id, &self.sessions, !light).await;
            serde_json::to_value(state).unwrap_or_else(|_| json!({ "is_running": false }))
        } else {
            json!({ "is_running": false })
        }
    }
}
