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

                // Auto-remember user message as semantic memory
                if let Some(ref memory) = self.memory {
                    if content.split_whitespace().count() >= 3 {
                        let meta = std::collections::HashMap::new();
                        if let Err(e) = memory.remember_with_importance(&content, meta, 1.0).await {
                            debug!("Failed to auto-remember user message: {}", e);
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

                        // Auto-remember assistant response as semantic memory
                        if let Some(ref memory) = self.memory {
                            if result.content.split_whitespace().count() >= 3 {
                                let meta = std::collections::HashMap::new();
                                if let Err(e) = memory.remember_with_importance(&result.content, meta, 1.0).await {
                                    debug!("Failed to auto-remember assistant response: {}", e);
                                }
                            }
                        }

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
                        // If there's a partial result (agent did work before failing),
                        // persist it so the user doesn't lose hours of streamed work
                        if let Some(partial) = e.partial_result {
                            warn!(
                                "Chat failed but {} chars of work were done — persisting partial result",
                                partial.content.len()
                            );
                            let reasoning = partial.reasoning.clone();
                            self.sessions.add_full_message(
                                &session_id,
                                MessageRole::Assistant,
                                &partial.content,
                                partial.tool_calls.clone(),
                                reasoning,
                            ).await;

                            json!({
                                "error": "chat_failed",
                                "message": e.message,
                                "partial_content": partial.content,
                                "partial": true
                            })
                        } else {
                            json!({
                                "error": "chat_failed",
                                "message": e.message
                            })
                        }
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
