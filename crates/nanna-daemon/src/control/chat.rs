//! Chat handlers for the [`ControlPlane`].

use super::*;

impl ControlPlane {
    // =========================================================================
    // Chat Handlers
    // =========================================================================
    
    pub(super) async fn handle_chat(&self, client_id: &str, action: ChatAction) -> Value {
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

                // Auto-remember user message only when the user has opted in
                // (`[memory] auto_remember_messages = true`). Default is off so a
                // first-run install does not silently store conversation content.
                if self.config.read().await.memory.auto_remember_messages {
                    if let Some(ref memory) = self.memory {
                        if content.split_whitespace().count() >= 3 {
                            let meta = std::collections::HashMap::new();
                            if let Err(e) = memory.remember_with_importance(&content, meta, 1.0).await {
                                debug!("Failed to auto-remember user message: {}", e);
                            }
                        }
                    }
                }
                
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

                // Build system prompt with persona + memory + workspace context
                let mut system_prompt = self.system_prompt.read().await.clone();

                // Global persona / user profile (config — independent of workspace)
                {
                    let cfg = self.config.read().await;
                    let persona = nanna_core::GlobalPersona {
                        persona: cfg.agent.persona.clone(),
                        user_profile: cfg.agent.user_profile.clone(),
                    };
                    let inj = persona.build_system_prompt_injection();
                    if !inj.is_empty() {
                        system_prompt.push_str("

");
                        system_prompt.push_str(&inj);
                    }
                }

                // Resolve workspace: session's workspace > globally active workspace
                let effective_ws_id = if session.workspace_id.is_some() {
                    session.workspace_id.clone()
                } else {
                    // Fall back to globally active workspace
                    let registry = self.workspaces.read().await;
                    registry.active().map(|ws| ws.id.clone())
                };

                // Inject workspace context (reload from disk so edits within the session are picked up)
                if let Some(ref ws_id) = effective_ws_id {
                    {
                        let mut registry = self.workspaces.write().await;
                        if let Some(ws) = registry.get_mut(ws_id) {
                            if let Err(e) = ws.load_context().await {
                                warn!("Failed to reload workspace context: {}", e);
                            }
                        }
                    }
                    let registry = self.workspaces.read().await;
                    if let Some(ws) = registry.get(ws_id) {
                        // Add workspace root path prominently so model knows where to look
                        let ws_path = ws.path.display();
                        system_prompt.push_str(&format!(
                            "\n\n## Active Workspace\n\
                            **Root directory: {ws_path}**\n\
                            All file operations and commands MUST use this directory as the base.\n\
                            Use relative paths (resolved against {ws_path}) or absolute paths within it.\n\
                            Do NOT search in home directory or other locations unless explicitly asked.\n"
                        ));

                        // Add workspace context files (README.md, AGENTS.md, ROADMAP.md, …)
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

                // Update workspace ID for script services (memory scoping)
                if let Some(ref ws_arc) = self.services_workspace_id {
                    *ws_arc.write().await = effective_ws_id.clone();
                }

                // Set tool working directory to workspace root
                if let Some(ref ws_id) = effective_ws_id {
                    let registry = self.workspaces.read().await;
                    if let Some(ws) = registry.get(ws_id) {
                        agent.tools().set_default_workdir(Some(ws.path.clone())).await;
                    }
                }

                // Set session ID so tools can scope per-session state
                agent.tools().set_session_id(Some(session_id.clone())).await;

                // ── Long-horizon chat (P19): the only path ──
                // Every turn is a harness run: the message is planned, the
                // plan is driven with re-anchored steps, and the work streams
                // into this transcript as it happens. A message sent while a
                // run is live joins that run at the next step boundary.

                // Attachments are not carried into harness steps yet (open
                // P19 item, see ROADMAP) — warn so the gap is visible in the
                // logs instead of silently dropping user input.
                if !attachments.is_empty() {
                    warn!(
                        count = attachments.len(),
                        "attachments are not yet supported by long-horizon chat — ignored"
                    );
                }

                // Each harness step re-anchors from the task store, so the
                // conversation must ride in the system prompt (the retired
                // direct path passed it as a message array). The same bounded
                // rendering is handed to the planner as context.
                let conversation = super::chat_harness::conversation_context(&prior_messages);
                if let Some(ref conversation) = conversation {
                    system_prompt.push_str("\n\n## Conversation so far\n");
                    system_prompt.push_str(conversation);
                }

                let workspace_root = if let Some(ref ws_id) = effective_ws_id {
                    let registry = self.workspaces.read().await;
                    registry.get(ws_id).map(|ws| ws.path.clone())
                } else {
                    None
                };
                match self
                    .run_chat_turn(
                        &session_id,
                        &content,
                        system_prompt,
                        conversation,
                        workspace_root,
                    )
                    .await
                {
                    // The run proceeds in a spawned task; ACK immediately so
                    // the IPC request never outlives the client's patience —
                    // a run can last hours, and the transcript is driven by
                    // events, not by this response.
                    Ok(Some(message_id)) => {
                        json!({
                            "status": "started",
                            "message_id": message_id,
                            "content": "",
                        })
                    }
                    // The message joined the run already in flight.
                    Ok(None) => {
                        let depth = self
                            .chat_runs
                            .pending_for(&session_id)
                            .await
                            .len()
                            .await;
                        super::chat_harness::interjected_response(&session_id, depth)
                    }
                    // Only reachable when the daemon is degraded (agent,
                    // router, tools or storage missing) — report it honestly
                    // rather than pretending a second chat path exists.
                    Err(message) => {
                        warn!(%message, "long-horizon chat could not start");
                        json!({
                            "error": "chat_failed",
                            "message": message,
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
                // Drop the stale assistant reply, recover the user message that
                // produced it, and replay the turn through the normal send path
                // (which re-adds the user message, rebuilds context, and runs
                // the agent) — so regeneration reuses all of Send's logic.
                let Some(mut session) = self.sessions.get(&session_id).await else {
                    return json!({
                        "error": "session_not_found",
                        "message": format!("Session {session_id} not found")
                    });
                };
                let Some(content) = session.take_last_user_turn() else {
                    return json!({
                        "status": "nothing_to_regenerate",
                        "session_id": session_id,
                        "message": "No user message to regenerate from"
                    });
                };
                self.sessions.update(session).await;
                Box::pin(self.handle_chat(
                    client_id,
                    ChatAction::Send {
                        session_id,
                        content,
                        attachments: Vec::new(),
                    },
                ))
                .await
            }
        }
    }
}
