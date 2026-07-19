//! Task store + long-horizon run commands (P15 store, P14 runs).
//!
//! Thin IPC forwarders: every command proxies to the daemon's `task` actions
//! through [`Backend`](crate::backend::Backend). The daemon reports domain
//! errors as an `{"error": code, "message": ...}` object INSIDE a success
//! envelope, so the raw `serde_json::Value` is returned and the frontend
//! checks for the `error` key.

#[allow(clippy::wildcard_imports)]
use crate::*;

/// List tasks in a scope ("session" | "workspace" | "global").
#[tauri::command]
pub async fn list_tasks(
    state: State<'_, Arc<RwLock<AppState>>>,
    scope: String,
    session_id: Option<String>,
    include_closed: Option<bool>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard
        .backend
        .task_list(&scope, session_id.as_deref(), include_closed)
        .await
}

/// Get one task with its notes + activity log.
#[tauri::command]
pub async fn get_task(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: i64,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.task_get(id).await
}

/// Create a task. `payload` is passed through to the daemon's `create` action
/// (title, scope, session_id, parent_id, description, priority, labels, ...).
#[tauri::command]
pub async fn create_task(
    state: State<'_, Arc<RwLock<AppState>>>,
    payload: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.task_create(payload).await
}

/// Partially update a task.
#[tauri::command]
pub async fn update_task(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: i64,
    patch: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.task_update(id, patch).await
}

/// Complete a task, running acceptance verification when configured.
#[tauri::command]
pub async fn complete_task(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: i64,
    workdir: Option<String>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.task_done(id, workdir.as_deref()).await
}

/// Delete a task subtree.
#[tauri::command]
pub async fn delete_task(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: i64,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.task_delete(id).await
}

/// Append a working note to a task.
#[tauri::command]
pub async fn add_task_note(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: i64,
    content: String,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.task_note(id, &content).await
}

/// Run a filter-language query over a scope's tasks.
#[tauri::command]
pub async fn query_tasks(
    state: State<'_, Arc<RwLock<AppState>>>,
    filter: String,
    scope: String,
    session_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard
        .backend
        .task_query(&filter, &scope, session_id.as_deref())
        .await
}

/// Start a long-horizon run. `payload` is passed through to the daemon's
/// `start_run` action (goal, scope, session_id, workdir, budgets).
#[tauri::command]
pub async fn start_task_run(
    state: State<'_, Arc<RwLock<AppState>>>,
    payload: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.task_start_run(payload).await
}

/// Status of the scope's run (live or last report).
#[tauri::command]
pub async fn get_task_run_status(
    state: State<'_, Arc<RwLock<AppState>>>,
    scope: String,
    session_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard
        .backend
        .task_run_status(&scope, session_id.as_deref())
        .await
}

/// Cancel the scope's active run.
#[tauri::command]
pub async fn cancel_task_run(
    state: State<'_, Arc<RwLock<AppState>>>,
    scope: String,
    session_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard
        .backend
        .task_cancel_run(&scope, session_id.as_deref())
        .await
}
