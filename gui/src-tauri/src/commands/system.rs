//! Window, notification, stats, and lifecycle commands.

#[allow(clippy::wildcard_imports)]
use crate::*;

/// Show the main window (called from system tray)
#[tauri::command]
pub async fn show_window(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Hide the main window to tray
#[tauri::command]
pub async fn hide_to_tray(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

// =============================================================================
// Notification Commands
// =============================================================================

/// Send a native notification
#[tauri::command]
pub async fn send_notification(
    app: AppHandle,
    title: String,
    body: String,
) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;

    app.notification()
        .builder()
        .title(&title)
        .body(&body)
        .show()
        .map_err(|e| format!("Failed to send notification: {}", e))?;

    info!("Sent notification: {} - {}", title, body);
    Ok(())
}

/// Request notification permission (needed on some platforms)
#[tauri::command]
pub async fn request_notification_permission(app: AppHandle) -> Result<bool, String> {
    use tauri_plugin_notification::NotificationExt;

    let permission = app.notification()
        .request_permission()
        .map_err(|e| format!("Failed to request permission: {}", e))?;

    Ok(matches!(permission, tauri_plugin_notification::PermissionState::Granted))
}

/// Check if notifications are permitted
#[tauri::command]
pub async fn check_notification_permission(app: AppHandle) -> Result<String, String> {
    use tauri_plugin_notification::NotificationExt;

    let permission = app.notification()
        .permission_state()
        .map_err(|e| format!("Failed to check permission: {}", e))?;

    Ok(match permission {
        tauri_plugin_notification::PermissionState::Granted => "granted",
        tauri_plugin_notification::PermissionState::Denied => "denied",
        _ => "unknown",
    }.to_string())
}

// =============================================================================
// Model Status Commands
// =============================================================================

/// Get current model status (active model, rate-limited models)
#[tauri::command]
pub async fn get_model_status(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<ModelStatusEvent, String> {
    let state_guard = state.read().await;
    let active = state_guard.active_model.read().await.clone();
    let rate_limited = state_guard.rate_limited_models.read().await;

    // Filter to only models that are still rate limited
    let now = chrono::Utc::now().timestamp();
    let still_limited: Vec<String> = rate_limited
        .iter()
        .filter(|(_, until)| now < **until)
        .map(|(model, _)| model.clone())
        .collect();

    Ok(ModelStatusEvent {
        active_model: active,
        fallback_reason: None,
        rate_limited_models: still_limited,
    })
}

/// Clear rate limit for a specific model (or all if model is None)
#[tauri::command]
pub async fn clear_rate_limit(
    state: State<'_, Arc<RwLock<AppState>>>,
    model: Option<String>,
) -> Result<(), String> {
    let state_guard = state.read().await;
    let mut rate_limited = state_guard.rate_limited_models.write().await;

    if let Some(model_id) = model {
        rate_limited.remove(&model_id);
        info!("Cleared rate limit for model: {}", model_id);
    } else {
        rate_limited.clear();
        info!("Cleared all rate limits");
    }

    Ok(())
}

/// Get detailed model performance statistics
#[tauri::command]
pub async fn get_model_stats(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;

    // Try daemon mode first
    let backend = &state_guard.backend;
    let status = backend.status().await;
    if status.connected {
        return backend.daemon_request(serde_json::json!({
            "type": "system",
            "action": "model_stats"
        })).await;
    }

    // Embedded mode: no model stats tracker available
    Ok(serde_json::json!({
        "models": [],
        "note": "Model stats are only available in daemon mode"
    }))
}

/// Get per-tool performance statistics
#[tauri::command]
pub async fn get_tool_stats(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;

    let backend = &state_guard.backend;
    let status = backend.status().await;
    if status.connected {
        let result = backend.daemon_request(serde_json::json!({
            "type": "system",
            "action": "tool_stats"
        })).await;
        info!("📊 get_tool_stats: daemon responded with {} tools",
            result.as_ref().ok()
                .and_then(|v| v.get("tools"))
                .and_then(|v| v.as_array())
                .map_or(0, |a| a.len()));
        return result;
    }

    warn!("📊 get_tool_stats: NOT CONNECTED (mode={:?}, daemon_state={})",
        status.mode, status.daemon_state);
    Ok(serde_json::json!({
        "tools": [],
        "note": "Tool stats are only available in daemon mode"
    }))
}

/// Get global tool + session dashboard stats
#[tauri::command]
pub async fn get_global_stats(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;

    let backend = &state_guard.backend;
    let status = backend.status().await;
    if status.connected {
        let result = backend.daemon_request(serde_json::json!({
            "type": "system",
            "action": "global_stats"
        })).await;
        info!("📊 get_global_stats: daemon responded, total_calls={}",
            result.as_ref().ok()
                .and_then(|v| v.get("total_calls"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0));
        return result;
    }

    warn!("📊 get_global_stats: NOT CONNECTED (mode={:?}, daemon_state={})",
        status.mode, status.daemon_state);
    Ok(serde_json::json!({
        "total_calls": 0,
        "avg_latency_ms": 0,
        "success_rate": 1.0,
        "slowest_tools": [],
        "most_used_tools": [],
        "most_failed_tools": [],
        "session_totals": {
            "total_iterations": 0,
            "total_tool_calls": 0,
            "total_tool_time_ms": 0,
            "total_llm_time_ms": 0,
            "total_input_tokens": 0,
            "total_output_tokens": 0
        },
        "note": "Global stats are only available in daemon mode"
    }))
}

/// Get hourly tool stats time-series for graphs
#[tauri::command]
pub async fn get_tool_stats_hourly(
    state: State<'_, Arc<RwLock<AppState>>>,
    tool_name: Option<String>,
    hours: Option<u32>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    let backend = &state_guard.backend;
    let status = backend.status().await;
    if status.connected {
        return backend.daemon_request(serde_json::json!({
            "type": "system",
            "action": "tool_stats_hourly",
            "tool_name": tool_name,
            "hours": hours.unwrap_or(24)
        })).await;
    }
    warn!("📊 get_tool_stats_hourly: NOT CONNECTED (mode={:?}, daemon_state={})",
        status.mode, status.daemon_state);
    Ok(serde_json::json!({ "buckets": [] }))
}

/// Get daily tool stats time-series for graphs
#[tauri::command]
pub async fn get_tool_stats_daily(
    state: State<'_, Arc<RwLock<AppState>>>,
    tool_name: Option<String>,
    days: Option<u32>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    let backend = &state_guard.backend;
    let status = backend.status().await;
    if status.connected {
        return backend.daemon_request(serde_json::json!({
            "type": "system",
            "action": "tool_stats_daily",
            "tool_name": tool_name,
            "days": days.unwrap_or(30)
        })).await;
    }
    warn!("📊 get_tool_stats_daily: NOT CONNECTED (mode={:?}, daemon_state={})",
        status.mode, status.daemon_state);
    Ok(serde_json::json!({ "buckets": [] }))
}

/// Get recent tool call log entries
#[tauri::command]
pub async fn get_tool_call_log(
    state: State<'_, Arc<RwLock<AppState>>>,
    tool_name: Option<String>,
    limit: Option<u32>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    let backend = &state_guard.backend;
    let status = backend.status().await;
    if status.connected {
        return backend.daemon_request(serde_json::json!({
            "type": "system",
            "action": "tool_call_log",
            "tool_name": tool_name,
            "limit": limit.unwrap_or(50)
        })).await;
    }
    Ok(serde_json::json!({ "entries": [] }))
}

// =============================================================================
// Backend Mode Commands
// =============================================================================

/// Get current backend status (daemon or embedded mode)
#[tauri::command]
pub async fn get_backend_status(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<backend::BackendStatus, String> {
    let state = state.read().await;
    Ok(state.backend.status().await)
}

/// Initialize the backend - starts daemon sidecar and connects
#[tauri::command]
pub async fn init_backend(
    app: AppHandle,
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<String, String> {
    let state = state.read().await;
    if state.backend.is_daemon_mode().await {
        return Ok("daemon".to_string());
    }
    // Embedded mode with local storage open: this process holds the exclusive
    // nanna.db lock, so a daemon spawned now would boot storage-less (no
    // session or memory persistence) — worse than staying embedded. Switching
    // to daemon mode requires an app restart so the daemon can own the DB.
    if state.storage.is_some() {
        info!("init_backend: staying embedded — local storage owns nanna.db (restart to use daemon mode)");
        return Ok("embedded".to_string());
    }
    let mode = state.backend.init(&app).await;
    Ok(match mode {
        BackendMode::Daemon => "daemon".to_string(),
        BackendMode::Embedded => "embedded".to_string(),
    })
}

/// Get daemon logs
#[tauri::command]
pub async fn get_daemon_logs(
    state: State<'_, Arc<RwLock<AppState>>>,
    limit: Option<usize>,
) -> Result<Vec<serde_json::Value>, String> {
    let state_guard = state.read().await;
    if state_guard.backend.is_daemon_mode().await {
        state_guard.backend.get_logs(limit).await
    } else {
        Ok(vec![])
    }
}

// =============================================================================
// Window Close Behavior
// =============================================================================

/// Get current close mode preference
#[tauri::command]
pub async fn get_close_mode(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<String, String> {
    let state = state.read().await;
    let mode = *state.close_mode.read().await;
    Ok(match mode {
        CloseMode::Ask => "ask".to_string(),
        CloseMode::MinimizeToTray => "minimize_to_tray".to_string(),
        CloseMode::QuitCompletely => "quit_completely".to_string(),
    })
}

/// Set close mode preference
#[tauri::command]
pub async fn set_close_mode(
    state: State<'_, Arc<RwLock<AppState>>>,
    mode: String,
) -> Result<(), String> {
    let close_mode = match mode.as_str() {
        "ask" => CloseMode::Ask,
        "minimize_to_tray" => CloseMode::MinimizeToTray,
        "quit_completely" => CloseMode::QuitCompletely,
        _ => return Err(format!("Unknown close mode: {}", mode)),
    };

    let state = state.read().await;
    *state.close_mode.write().await = close_mode;
    info!("Close mode set to: {:?}", close_mode);
    Ok(())
}

/// Handle window close - returns what action to take
/// Called from frontend before actual close
#[tauri::command]
pub async fn handle_window_close(
    app: AppHandle,
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<String, String> {
    let state_guard = state.read().await;
    let mode = *state_guard.close_mode.read().await;

    match mode {
        CloseMode::Ask => {
            // Frontend should show dialog
            Ok("ask".to_string())
        }
        CloseMode::MinimizeToTray => {
            // Hide window to tray
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }
            Ok("minimized".to_string())
        }
        CloseMode::QuitCompletely => {
            // Will trigger actual exit
            Ok("quit".to_string())
        }
    }
}

/// Perform actual quit (called after user confirms or preference is quit)
#[tauri::command]
pub async fn perform_quit(
    app: AppHandle,
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Shutdown backend (stop daemon)
    info!("Performing quit - shutting down backend...");
    state_guard.backend.shutdown().await;

    // Save memories
    let count = state_guard.memory.count().await;
    if count > 0 {
        let backup_path = state_guard.memory_path.with_extension("json.bak");
        if state_guard.memory_path.exists() {
            if let Err(e) = std::fs::copy(&state_guard.memory_path, &backup_path) {
                warn!("Failed to create memory backup: {}", e);
            }
        }

        if let Err(e) = state_guard.memory.save(&state_guard.memory_path).await {
            error!("Failed to save memories on exit: {}", e);
        } else {
            info!("Saved {} memories", count);
        }
    }

    // Exit the app
    app.exit(0);
    Ok(())
}
