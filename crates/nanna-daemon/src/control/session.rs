//! Session and sub-session handlers for the [`ControlPlane`].

use super::*;

impl ControlPlane {
    // =========================================================================
    // Session Handlers
    // =========================================================================
    
    pub(super) async fn handle_session(&self, client_id: &str, action: SessionAction) -> Value {
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

        // Build system prompt (include workspace context so sub-agents know the codebase)
        let sys_prompt = system_prompt.unwrap_or_else(|| {
            let base = self.system_prompt.blocking_read().clone();

            // Inject workspace context so sub-agents see AGENTS.md, SOUL.md, etc.
            let ws_context = {
                let registry = self.workspaces.blocking_read();
                registry.active()
                    .map(|ws| {
                        let mut ctx = String::new();
                        let ws_path = ws.path.display();
                        ctx.push_str(&format!(
                            "\n\n## Active Workspace\n\
                            **Root directory: {ws_path}**\n\
                            All file operations and commands MUST use this directory as the base.\n"
                        ));
                        let injection = ws.context.build_system_prompt_injection();
                        if !injection.is_empty() {
                            ctx.push_str(&format!("\n{injection}"));
                        }
                        ctx
                    })
                    .unwrap_or_default()
            };

            format!("{base}{ws_context}\n\nYou are a sub-agent. All tools are pre-activated — use them directly (no need to call discover_tools). Execute the task immediately and return results when done.\n\nYour task: {task}")
        });

        // Spawn the agent in a background task
        let agent = agent.clone();
        let sessions = self.sessions.clone();
        let event_tx = self.event_tx.clone();
        let workspaces = self.workspaces.clone();
        let model_for_task = model;
        let max_iters = max_iterations;
        let sid = session_id.clone();
        let lbl = label.clone();
        let pid = parent_id.clone();
        let task_for_extraction = task.clone();

        // Snapshot the parent's workdir so the sub-agent can restore it.
        // The shared ToolRegistry has a single workdir that can be overwritten
        // by concurrent sessions, so we capture it here and re-set it inside
        // the spawned task to avoid races.
        let parent_workdir = agent.tools().default_workdir().await;

        tokio::spawn(async move {
            // Mark as running
            sessions.set_sub_session_state(&sid, SubSessionState::Running).await;

            // Set session ID so tools can scope per-session state
            agent.tools().set_session_id(Some(sid.clone())).await;

            // Set per-session workdir for the sub-agent from the parent's snapshot.
            // This avoids races with the global default_workdir which can be
            // overwritten by concurrent sessions.
            if let Some(ref wd) = parent_workdir {
                agent.tools().set_session_workdir(&sid, wd.clone()).await;
            }

            // Apply timeout if specified
            let result = if let Some(timeout) = timeout_secs {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(timeout),
                    agent.chat_with_options(&sid, &task, Some(sys_prompt), &[], model_for_task.clone(), max_iters, None, vec![], true),
                ).await {
                    Ok(r) => r,
                    Err(_) => Err(crate::agent_service::ChatError {
                        message: format!("Sub-session timed out after {}s", timeout),
                        partial_result: None,
                    }),
                }
            } else {
                agent.chat_with_options(&sid, &task, Some(sys_prompt), &[], model_for_task.clone(), max_iters, None, vec![], true).await
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

                    // Extract project knowledge and update AGENTS.md + TOOLS.md
                    let ws_info = {
                        let reg = workspaces.read().await;
                        reg.active().map(|ws| (
                            ws.path.clone(),
                            ws.context.agents.clone(),
                            ws.context.tools.clone(),
                        ))
                    };
                    if let Some((ws_path, current_agents, current_tools)) = ws_info {
                        let result_text = chat_result.content;
                        let extraction_task = task_for_extraction.clone();
                        let agent_for_extract = agent.clone();
                        tokio::spawn(async move {
                            // Trim result to last 2000 chars to keep extraction prompt small
                            let result_tail: String = if result_text.len() > 2000 {
                                result_text[result_text.len() - 2000..].to_string()
                            } else {
                                result_text.clone()
                            };
                            let extraction_prompt = format!(
                                "A sub-agent just completed a task. Extract NEW knowledge into two categories.\n\n\
                                Task: {}\n\n\
                                Result (tail):\n{}\n\n\
                                Current AGENTS.md:\n{}\n\n\
                                Current TOOLS.md:\n{}\n\n\
                                Respond in this EXACT format (include both headers even if empty):\n\
                                \n\
                                AGENTS:\n\
                                (bullet points about project knowledge: build commands, file locations, architecture, \
                                gotchas, conventions, error fixes. Or write NONE)\n\
                                \n\
                                TOOLS:\n\
                                (bullet points about tool/environment specifics: host names, paths, device names, \
                                CLI flags, environment variables, service URLs, config details. Or write NONE)\n\
                                \n\
                                Rules: be concise, no duplicates of existing content, bullet points only.",
                                extraction_task,
                                result_tail,
                                current_agents.as_deref().unwrap_or("(empty)"),
                                current_tools.as_deref().unwrap_or("(empty)")
                            );
                            let extract_sid = format!("_extract_{}", uuid::Uuid::new_v4());
                            match agent_for_extract.chat(
                                &extract_sid,
                                &extraction_prompt,
                                Some("You are a concise knowledge extractor. Follow the output format exactly.".to_string()),
                                &[],
                            ).await {
                                Ok(resp) => {
                                    let content = resp.content.trim();
                                    if content.len() > 4000 || content.is_empty() {
                                        return; // Sanity guard
                                    }

                                    // Parse the two sections
                                    let (agents_section, tools_section) = {
                                        let upper = content.to_uppercase();
                                        let agents_start = upper.find("AGENTS:");
                                        let tools_start = upper.find("TOOLS:");

                                        let agents_part = match (agents_start, tools_start) {
                                            (Some(a), Some(t)) if a < t => {
                                                Some(content[a + "AGENTS:".len()..t].trim())
                                            }
                                            (Some(a), None) => {
                                                Some(content[a + "AGENTS:".len()..].trim())
                                            }
                                            _ => None,
                                        };
                                        let tools_part = match tools_start {
                                            Some(t) => Some(content[t + "TOOLS:".len()..].trim()),
                                            None => None,
                                        };
                                        (agents_part, tools_part)
                                    };

                                    // Update AGENTS.md if there's new content
                                    if let Some(agents_new) = agents_section {
                                        if !agents_new.eq_ignore_ascii_case("NONE") && !agents_new.is_empty() && agents_new.len() < 2000 {
                                            let agents_path = ws_path.join(".nanna").join("AGENTS.md");
                                            if let Ok(existing) = tokio::fs::read_to_string(&agents_path).await {
                                                let updated = format!("{}\n\n### Learned\n{}\n", existing.trim_end(), agents_new);
                                                if let Err(e) = tokio::fs::write(&agents_path, updated).await {
                                                    warn!("Failed to update AGENTS.md: {e}");
                                                } else {
                                                    info!("Updated AGENTS.md with knowledge from sub-agent task");
                                                }
                                            }
                                        }
                                    }

                                    // Update TOOLS.md if there's new content
                                    if let Some(tools_new) = tools_section {
                                        if !tools_new.eq_ignore_ascii_case("NONE") && !tools_new.is_empty() && tools_new.len() < 2000 {
                                            let tools_path = ws_path.join(".nanna").join("TOOLS.md");
                                            if let Ok(existing) = tokio::fs::read_to_string(&tools_path).await {
                                                let updated = format!("{}\n\n### Learned\n{}\n", existing.trim_end(), tools_new);
                                                if let Err(e) = tokio::fs::write(&tools_path, updated).await {
                                                    warn!("Failed to update TOOLS.md: {e}");
                                                } else {
                                                    info!("Updated TOOLS.md with knowledge from sub-agent task");
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    debug!("Knowledge extraction skipped (LLM error): {}", e.message);
                                }
                            }
                        });
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
        });

        json!({
            "status": "spawned",
            "session_id": session_id,
            "label": label,
            "parent_id": parent_id,
        })
    }
}
