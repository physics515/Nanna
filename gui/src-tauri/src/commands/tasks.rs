//! Task-store commands for the GUI task management UI.
//!
//! The task store is the chat engine (P19): the planner seeds it, the harness
//! drains it, and the agent's `todo` skill writes to it. The GUI provides
//! a task checklist sidebar on the chat page, plus full CRUD operations
//! for manual task management.

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

/// Create a new task.
#[tauri::command]
pub async fn create_task(
    state: State<'_, Arc<RwLock<AppState>>>,
    title: String,
    scope: String,
    session_id: Option<String>,
    parent_id: Option<i64>,
    description: Option<String>,
    priority: Option<i64>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard
        .backend
        .task_create(
            &title,
            &scope,
            session_id.as_deref(),
            parent_id,
            description.as_deref(),
            priority,
        )
        .await
}

/// Update a task with a partial patch.
#[tauri::command]
pub async fn update_task(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: i64,
    patch: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.task_update(id, patch).await
}

/// Mark a task as done (with optional acceptance check).
#[tauri::command]
pub async fn complete_task(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: i64,
    workdir: Option<String>,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.task_done(id, workdir.as_deref()).await
}

/// Delete a task and its subtree.
#[tauri::command]
pub async fn delete_task(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: i64,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.task_delete(id).await
}

/// Reorder a task by updating its priority.
#[tauri::command]
pub async fn reorder_task(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: i64,
    new_priority: i64,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.task_reorder(id, new_priority).await
}
