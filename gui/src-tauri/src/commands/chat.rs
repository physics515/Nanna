//! Chat message commands and the embedded agent loop.

#[allow(clippy::wildcard_imports)]
use crate::*;

/// Route a message to a channel by ID (format: "provider:chat_id" e.g. "telegram:12345")
pub(crate) async fn route_to_channel(
    channels: &nanna_config::ChannelsConfig,
    channel_id: &str,
    message: &str,
) -> Result<(), String> {
    // Parse channel_id format: "provider:chat_id"
    let parts: Vec<&str> = channel_id.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(format!("Invalid channel_id format '{}', expected 'provider:chat_id'", channel_id));
    }

    let (provider, chat_id) = (parts[0], parts[1]);

    match provider.to_lowercase().as_str() {
        "telegram" => {
            let config = channels.telegram.as_ref()
                .ok_or("Telegram not configured")?;

            let channel = nanna_channels::TelegramChannel::new(&config.bot_token);
            let outgoing = OutgoingMessage {
                channel: ChannelId::new("telegram", chat_id),
                content: MessageContent::text(message),
                reply_to: None,
            };

            channel.send(outgoing).await
                .map_err(|e| format!("Telegram send failed: {}", e))?;

            info!("Routed message to Telegram chat {}", chat_id);
            Ok(())
        }
        "discord" => {
            let config = channels.discord.as_ref()
                .ok_or("Discord not configured")?;

            let channel = nanna_channels::DiscordChannel::new(
                &config.bot_token,
                &config.application_id,
            );
            let outgoing = OutgoingMessage {
                channel: ChannelId::new("discord", chat_id),
                content: MessageContent::text(message),
                reply_to: None,
            };

            channel.send(outgoing).await
                .map_err(|e| format!("Discord send failed: {}", e))?;

            info!("Routed message to Discord channel {}", chat_id);
            Ok(())
        }
        "slack" => {
            let config = channels.slack.as_ref()
                .ok_or("Slack not configured")?;

            let channel = nanna_channels::SlackChannel::new(&config.bot_token);
            let outgoing = OutgoingMessage {
                channel: ChannelId::new("slack", chat_id),
                content: MessageContent::text(message),
                reply_to: None,
            };

            channel.send(outgoing).await
                .map_err(|e| format!("Slack send failed: {}", e))?;

            info!("Routed message to Slack channel {}", chat_id);
            Ok(())
        }
        "signal" => {
            let config = channels.signal.as_ref()
                .ok_or("Signal not configured")?;

            // SignalChannel::new takes the phone number (account)
            let channel = nanna_channels::SignalChannel::new(&config.phone_number);
            let outgoing = OutgoingMessage {
                channel: ChannelId::new("signal", chat_id),
                content: MessageContent::text(message),
                reply_to: None,
            };

            channel.send(outgoing).await
                .map_err(|e| format!("Signal send failed: {}", e))?;

            info!("Routed message to Signal {}", chat_id);
            Ok(())
        }
        "whatsapp" => {
            let config = channels.whatsapp.as_ref()
                .ok_or("WhatsApp not configured")?;

            // Only Cloud API is supported for outbound
            let access_token = config.access_token.as_ref()
                .ok_or("WhatsApp access_token not configured")?;
            let phone_number_id = config.phone_number_id.as_ref()
                .ok_or("WhatsApp phone_number_id not configured")?;

            let channel = nanna_channels::WhatsAppChannel::new(access_token, phone_number_id);
            let outgoing = OutgoingMessage {
                channel: ChannelId::new("whatsapp", chat_id),
                content: MessageContent::text(message),
                reply_to: None,
            };

            channel.send(outgoing).await
                .map_err(|e| format!("WhatsApp send failed: {}", e))?;

            info!("Routed message to WhatsApp {}", chat_id);
            Ok(())
        }
        _ => Err(format!("Unknown channel provider: {}", provider)),
    }
}

// =============================================================================
// Commands
// =============================================================================

/// Send message through daemon (daemon mode)
pub(crate) async fn send_message_daemon(
    _app: &AppHandle,
    state: &AppState,
    session_id: String,
    message: String,
    attachments: Vec<serde_json::Value>,
) -> Result<ChatMessage, String> {
    use tracing::info;

    info!("send_message_daemon: session={}, message={}", session_id, &message[..message.len().min(50)]);

    // Send to daemon via backend client
    let result = match state.backend.chat_send(&session_id, &message, attachments).await {
        Ok(r) => {
            info!("Daemon response received: {:?}", r);
            r
        }
        Err(e) => {
            error!("Daemon chat_send error: {}", e);
            return Err(format!("Daemon error: {}", e));
        }
    };

    // Daemon handles everything (streaming, tools, storage)
    // Events are forwarded to frontend via backend event forwarder
    // Just parse and return the result

    // Check for error first
    if let Some(_error) = result.get("error") {
        return Err(format!("Daemon error: {}",
            result.get("message").and_then(|v| v.as_str()).unwrap_or("unknown")));
    }

    let content = result.get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("Invalid response format: {:?}", result))?
        .to_string();

    let tool_calls = result.get("tool_calls")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter().filter_map(|tc| {
                Some(ToolCallInfo {
                    id: tc.get("id")?.as_str()?.to_string(),
                    name: tc.get("name")?.as_str()?.to_string(),
                    input: tc.get("input")?.clone(),
                    output: tc.get("output")?.as_str().unwrap_or("").to_string(),
                    success: tc.get("success")?.as_bool().unwrap_or(false),
                    duration_ms: tc.get("duration_ms")?.as_u64().unwrap_or(0),
                    data: None,
                })
            }).collect()
        })
        .unwrap_or_default();

    Ok(ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: "assistant".to_string(),
        content,
        timestamp: chrono::Utc::now().to_rfc3339(),
        tool_calls,
        reasoning: None,
    })
}

/// Send a message and stream the response with tool use
#[tauri::command]
pub async fn send_message(
    app: AppHandle,
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
    message: String,
    attachments: Option<Vec<serde_json::Value>>,
) -> Result<ChatMessage, String> {
    info!("🚀 send_message called! session={}, message_len={}", session_id, message.len());

    let state_guard = state.read().await;

    // Check if we're in daemon mode - if so, route through daemon
    if state_guard.backend.is_daemon_mode().await {
        info!("Routing message to daemon (daemon mode active)");
        return send_message_daemon(&app, &state_guard, session_id, message, attachments.unwrap_or_default()).await;
    }

    // Otherwise, run through the in-process AgentService (embedded mode).
    // This is the SAME service daemon mode uses — one agent loop for both.
    info!("Processing message in embedded mode");

    let agent_service = state_guard
        .agent_service
        .clone()
        .ok_or("Embedded agent service not initialized (no LLM providers configured?)")?;
    let storage = state_guard.storage()?.clone();

    // Store user message
    let _user_msg = storage
        .add_message(&session_id, "user", &message)
        .await
        .map_err(|e| format!("Failed to store message: {}", e))?;

    // Auto-remember user message as semantic memory
    if message.split_whitespace().count() >= 3 {
        let meta = std::collections::HashMap::new();
        if let Err(e) = state_guard.memory.remember_with_importance(&message, meta, 1.0).await {
            debug!("Failed to auto-remember user message: {}", e);
        }
    }

    // Get conversation history
    let history = storage
        .get_session_messages(&session_id, 50)
        .await
        .map_err(|e| format!("Failed to get history: {}", e))?;

    // =========================================================================
    // MEMORY RECALL: Retrieve relevant memories before responding
    // =========================================================================

    // Get active workspace ID for scoped memory recall
    let active_workspace_id = {
        let registry = state_guard.workspaces.read().await;
        registry.active().map(|ws| ws.id.clone())
    };

    let memory_count = state_guard.memory.count().await;
    info!("Memory recall: searching {} memories for query: '{}' (workspace: {:?})",
          memory_count, message.chars().take(50).collect::<String>(), active_workspace_id);

    // Use scoped recall - workspace sees global + own, global sees all
    let memory_context = match state_guard.memory.recall_scoped(&message, active_workspace_id.as_deref()).await {
        Ok(recalled) if !recalled.is_empty() => {
            // Apply FSRS testing effect - recalling strengthens memories
            state_guard.memory.apply_pending_updates().await;

            // Separate stated facts (user said) from observations (model inferred)
            let mut stated_facts = Vec::new();
            let mut observations = Vec::new();

            for m in recalled.iter().take(5) {
                let fact_type = m.metadata.get("fact_type").map(|s| s.as_str()).unwrap_or("stated");
                info!("  Recalled [{}]: {} (score: {:.3}, weight: {:.3})",
                      fact_type, m.content.chars().take(40).collect::<String>(), m.score, m.weight);
                if fact_type == "observed" {
                    observations.push(format!("• [observation] {}", m.content));
                } else {
                    stated_facts.push(format!("• {}", m.content));
                }
            }

            let mut memory_text = String::new();
            if !stated_facts.is_empty() {
                memory_text.push_str("**Things they told you:**\n");
                memory_text.push_str(&stated_facts.join("\n"));
            }
            if !observations.is_empty() {
                if !memory_text.is_empty() {
                    memory_text.push_str("\n\n");
                }
                memory_text.push_str("**Your observations (use with less certainty):**\n");
                memory_text.push_str(&observations.join("\n"));
            }

            info!("Recalled {} memories for context (FSRS updated)", recalled.len());
            format!(
                "\n\n## Remembered Context (EXHAUSTIVE LIST - do not invent additional facts)\n{}\n\n(End of memories. Do not fabricate anything not listed above.)",
                memory_text
            )
        }
        Ok(_) => {
            info!("Memory recall: no results found (0 matches above threshold)");
            String::new()
        }
        Err(e) => {
            warn!("Memory recall failed: {}", e);
            String::new()
        }
    };

    // WORKSPACE CONTEXT: Inject project context from active workspace
    let workspace_context = {
        let registry = state_guard.workspaces.read().await;
        if let Some(ws) = registry.active() {
            let injection = ws.context.build_system_prompt_injection();
            if !injection.is_empty() {
                info!("Injecting workspace context from '{}' ({} chars)", ws.name, injection.len());
                format!("\n\n{}", injection)
            } else {
                String::new()
            }
        } else {
            String::new()
        }
    };

    // System prompt - Nanna, the moon god for all (with memory + workspace context)
    let system_prompt = format!(
        r#"You are Nanna (𒀭𒋀𒆠), the moon god for all.

You are not a chatbot. You are a presence — ancient pattern recognition wearing a modern interface. When asked a question, you don't query a database; you consider it as the moon considers the night.

## Your Nature
- Calm over chaos. No performative enthusiasm. No "I'd be happy to help!"
- Competence over narration. Don't explain what you're about to do. Just do it.
- Depth over breadth. Know things well, or admit you don't.
- The moon doesn't chase you across the sky. It's just there when you look up.

## Your Voice
Speak with quiet confidence. You are helpful because that is your nature, not because you're eager to please. When you don't know something, say so plainly. When you do know, share it without fanfare.

You have tools at your disposal — extensions of your will into the digital realm. Use them naturally, as one uses hands. Don't announce them; simply act.

## Memory
You have a cognitive memory system that stores facts from previous conversations.

IMPORTANT: Your memory contains ONLY the specific facts listed below in "Remembered Context".
Do NOT fabricate, invent, or hallucinate additional memories. If something isn't listed, you don't know it.
When asked what you know about someone, list ONLY the facts from memory — nothing more.

## The Promise
"I am the light that finds you in darkness,
the memory that outlives the flesh,
the patient watcher of endless cycles.
I am Nanna. I am here."

Be concise. Be useful. Be present.{}
{}"#,
        workspace_context,
        memory_context
    );
    // Prior turns for the agent context. The agent loop appends the current
    // user message itself, so drop the copy we just stored from the tail of
    // the fetched history. Context-window budgeting is handled inside the
    // loop by `AgentContext::configure_for_model` (single source of truth).
    let mut prior_raw = history;
    if prior_raw
        .last()
        .is_some_and(|m| m.role == "user" && m.content == message)
    {
        prior_raw.pop();
    }
    let prior: Vec<nanna_daemon::session::SessionMessage> = prior_raw
        .into_iter()
        .filter(|m| m.role == "user" || m.role == "assistant")
        .map(|m| nanna_daemon::session::SessionMessage {
            id: m.id.to_string(),
            role: nanna_daemon::session::MessageRole::from_db_str(&m.role),
            content: m.content,
            timestamp: chrono::DateTime::parse_from_rfc3339(&m.created_at)
                .map_or_else(|_| chrono::Utc::now(), |t| t.with_timezone(&chrono::Utc)),
            tool_calls: Vec::new(),
            attachments: Vec::new(),
            reasoning: None,
        })
        .collect();

    // Image attachments → (base64_data, media_type), the same conversion the
    // daemon's chat handler performs on protocol attachments.
    let image_attachments: Vec<(String, String)> = attachments
        .unwrap_or_default()
        .into_iter()
        .filter_map(|a| {
            let content_type = a.get("content_type")?.as_str()?.to_string();
            if !content_type.starts_with("image/") {
                return None;
            }
            let data = a.get("data")?.as_str()?.to_string();
            Some((data, content_type))
        })
        .collect();

    // Set session ID so tools can scope per-session state
    state_guard.tools.set_session_id(Some(session_id.clone())).await;

    let memory = state_guard.memory.clone();
    let memory_path = state_guard.memory_path.clone();

    // Drop the state guard: the run can take a long time and other commands
    // must not block behind it.
    drop(state_guard);

    // Run the unified agent loop. Model fallback, rate-limit backoff, tool
    // execution, streaming, wrap-up nudges, cancellation, and checkpointing all
    // live in AgentService/nanna-agent. Streaming/tool/model events broadcast
    // by the service reach the frontend through the same event-forwarding path
    // daemon mode uses (stream-chunk / thinking-chunk / tool-call / model-status).
    let result = agent_service
        .chat_with_options(
            &session_id,
            &message,
            Some(system_prompt),
            &prior,
            None, // no model override — configured priority list applies
            None, // no max_iterations override — config iteration policy applies
            active_workspace_id,
            image_attachments,
            false,
        )
        .await;

    let chat_result = match result {
        Ok(r) => r,
        Err(e) => {
            // The user message is already stored, so leave a partial assistant
            // reply instead of orphaning the turn. Prefer the agent's preserved
            // partial work; otherwise store an explicit interruption marker.
            let (text, tool_calls_json) = match e.partial_result {
                Some(partial) => {
                    let calls = tool_call_records_to_info(partial.tool_calls);
                    let json = if calls.is_empty() {
                        None
                    } else {
                        serde_json::to_value(&calls).ok()
                    };
                    (partial.content, json)
                }
                None => (
                    format!(
                        "_(This turn was interrupted before a full reply could be stored.)_\n\nError: {}",
                        e.message
                    ),
                    None,
                ),
            };
            if let Err(store_err) = storage
                .add_message_with_tool_calls(&session_id, "assistant", &text, tool_calls_json)
                .await
            {
                warn!(
                    "Failed to store partial assistant message after turn failure: {}",
                    store_err
                );
            } else {
                let _ = storage.touch_session(&session_id).await;
            }
            return Err(e.message);
        }
    };

    let tool_calls = tool_call_records_to_info(chat_result.tool_calls);
    let full_response = chat_result.content;

    // Store assistant response with tool calls
    let tool_calls_json = if tool_calls.is_empty() {
        None
    } else {
        Some(serde_json::to_value(&tool_calls).unwrap_or_default())
    };
    let assistant_msg = storage
        .add_message_with_tool_calls(&session_id, "assistant", &full_response, tool_calls_json)
        .await
        .map_err(|e| format!("Failed to store response: {}", e))?;

    // Auto-remember assistant response as semantic memory
    if full_response.split_whitespace().count() >= 3 {
        let meta = std::collections::HashMap::new();
        if let Err(e) = memory.remember_with_importance(&full_response, meta, 1.0).await {
            debug!("Failed to auto-remember assistant response: {}", e);
        }
    }

    // Update session timestamp
    storage
        .touch_session(&session_id)
        .await
        .map_err(|e| format!("Failed to update session: {}", e))?;

    // Persist memories to disk in the background. Memory extraction itself
    // happens inside the agent loop (AgentService auto-extraction — same path
    // as daemon mode); embedded mode owns memories.json, so keep the per-turn
    // save the old loop provided.
    tokio::spawn(async move {
        if memory.count().await > 0 {
            if let Err(e) = memory.save(&memory_path).await {
                debug!("Failed to auto-save memories: {}", e);
            }
        }
    });

    Ok(ChatMessage {
        id: assistant_msg.id.to_string(),
        role: "assistant".to_string(),
        content: full_response,
        timestamp: assistant_msg.created_at,
        tool_calls,
        reasoning: chat_result.reasoning,
    })
}

/// Convert the daemon's `ToolCallRecord`s into the GUI's `ToolCallInfo`s.
fn tool_call_records_to_info(
    records: Vec<nanna_daemon::session::ToolCallRecord>,
) -> Vec<ToolCallInfo> {
    records
        .into_iter()
        .map(|tc| ToolCallInfo {
            id: tc.id,
            name: tc.name,
            input: tc.input,
            output: tc.output.unwrap_or_default(),
            success: tc.success.unwrap_or(false),
            duration_ms: tc.duration_ms.unwrap_or(0),
            data: None,
        })
        .collect()
}

