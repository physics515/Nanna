//! Chat message command. The daemon owns the agent loop, streaming, tools, and
//! storage; this just forwards the turn and shapes the reply for the frontend.

#[allow(clippy::wildcard_imports)]
use crate::*;

/// Send a message through the daemon and return the assistant reply.
///
/// Streaming (`stream-chunk` / `thinking-chunk` / `tool-call` / `model-status`)
/// is delivered separately via the backend's event-forwarding task; this returns
/// the final message once the daemon reports the turn complete.
#[tauri::command]
pub async fn send_message(
    _app: AppHandle,
    state: State<'_, Arc<RwLock<AppState>>>,
    session_id: String,
    message: String,
    attachments: Option<Vec<serde_json::Value>>,
) -> Result<ChatMessage, String> {
    info!("send_message: session={}, message_len={}", session_id, message.len());

    let state_guard = state.read().await;

    let result = match state_guard
        .backend
        .chat_send(&session_id, &message, attachments.unwrap_or_default())
        .await
    {
        Ok(r) => r,
        Err(e) => {
            error!("Daemon chat_send error: {e}");
            return Err(format!("Daemon error: {e}"));
        }
    };

    // Surface a daemon-reported error.
    if result.get("error").is_some() {
        return Err(format!(
            "Daemon error: {}",
            result.get("message").and_then(|v| v.as_str()).unwrap_or("unknown")
        ));
    }

    let content = result
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("Invalid response format: {result:?}"))?
        .to_string();

    let tool_calls = result
        .get("tool_calls")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|tc| {
                    Some(ToolCallInfo {
                        id: tc.get("id")?.as_str()?.to_string(),
                        name: tc.get("name")?.as_str()?.to_string(),
                        input: tc.get("input")?.clone(),
                        output: tc.get("output")?.as_str().unwrap_or("").to_string(),
                        success: tc.get("success")?.as_bool().unwrap_or(false),
                        duration_ms: tc.get("duration_ms")?.as_u64().unwrap_or(0),
                        data: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: "assistant".to_string(),
        content,
        timestamp: chrono::Utc::now().to_rfc3339(),
        tool_calls,
        reasoning: result.get("reasoning").and_then(|v| v.as_str()).map(str::to_string),
    })
}
