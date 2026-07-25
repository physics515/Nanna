//! Task-store commands: the read-only view behind the chat's task checklist.
//!
//! The task store is the chat engine (P19): the planner seeds it, the harness
//! drains it, and the agent's `todo` skill writes to it. The GUI's only
//! surface for it is the checklist sidebar on the chat page — there is no
//! task-management page, deliberately. Mutations go through chat.

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
