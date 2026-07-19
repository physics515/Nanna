//! Database models

use serde::{Deserialize, Serialize};

/// Session model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: i64,
    pub session_id: String,
    pub channel: String,
    pub user_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub metadata: Option<serde_json::Value>,
    /// Optional workspace this session belongs to (None = global)
    pub workspace_id: Option<String>,
    /// Human-readable session name
    pub name: Option<String>,
}

/// Message model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: i64,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub content_type: String,
    pub tool_use_id: Option<String>,
    pub created_at: String,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub metadata: Option<serde_json::Value>,
}

/// Memory model (for vector search)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: i64,
    pub memory_id: String,
    pub content: String,
    pub embedding: Option<Vec<f32>>,
    pub embedding_model: Option<String>,
    pub session_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub metadata: Option<serde_json::Value>,
    pub tags: Vec<String>,
    /// Workspace scope (None = global)
    pub workspace_id: Option<String>,
    /// FSRS cognitive state fields
    pub fsrs_stability: f32,
    pub fsrs_difficulty: f32,
    pub fsrs_last_access: i64,
    pub fsrs_access_count: i64,
    pub fsrs_importance: f32,
    pub fsrs_storage_strength: f32,
    pub fsrs_generation: i64,
}

/// Cron job model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: i64,
    pub job_id: String,
    pub schedule: String,
    pub task: serde_json::Value,
    pub enabled: bool,
    pub last_run: Option<String>,
    pub next_run: Option<String>,
    pub created_at: String,
    pub metadata: Option<serde_json::Value>,
}

/// New message input
#[derive(Debug, Clone)]
pub struct NewMessage {
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub content_type: String,
    pub tool_use_id: Option<String>,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub metadata: Option<serde_json::Value>,
}

/// New memory input
#[derive(Debug, Clone)]
pub struct NewMemory {
    pub memory_id: String,
    pub content: String,
    pub embedding: Option<Vec<f32>>,
    pub embedding_model: Option<String>,
    pub session_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub tags: Vec<String>,
    /// Workspace scope (None = global)
    pub workspace_id: Option<String>,
    /// FSRS cognitive state fields
    pub fsrs_stability: f32,
    pub fsrs_difficulty: f32,
    pub fsrs_last_access: i64,
    pub fsrs_access_count: i64,
    pub fsrs_importance: f32,
    pub fsrs_storage_strength: f32,
    pub fsrs_generation: i64,
}

/// New cron job input
#[derive(Debug, Clone)]
pub struct NewCronJob {
    pub job_id: String,
    pub schedule: String,
    pub task: serde_json::Value,
    pub enabled: bool,
    pub next_run: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

/// Job run history entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRun {
    pub id: i64,
    pub job_id: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
    pub duration_ms: Option<i64>,
}

/// New job run input
#[derive(Debug, Clone)]
pub struct NewJobRun {
    pub job_id: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
    pub duration_ms: Option<i64>,
}

/// Registered workspace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceRecord {
    pub id: String,
    pub name: String,
    pub path: String,
    pub active: bool,
    pub created_at: String,
    pub last_accessed: String,
}

/// Task model (agent-grade task store, P15).
///
/// `status` is one of `pending | in_progress | done | cancelled`. `blocked` is
/// never stored: it is derived from `depends_on` at read time by
/// `TaskRepository` — a task is blocked while any dependency is not `done`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: i64,
    pub parent_id: Option<i64>,
    /// `session` | `workspace` | `global`
    pub scope: String,
    /// session_id or workspace_id depending on scope (None for global)
    pub scope_id: Option<String>,
    pub project: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    /// 1 (highest) ..= 4 (lowest), Todoist-style
    pub priority: i64,
    pub labels: Vec<String>,
    /// Tool names the current item scopes the agent to (P14 per-item tool hint)
    pub tool_scope: Vec<String>,
    pub due_at: Option<String>,
    /// Cron expression executed by the existing scheduler (one recurrence engine)
    pub recurrence: Option<String>,
    pub depends_on: Vec<i64>,
    /// Machine-checkable done-condition, run by the harness — JSON
    /// `{kind: "command"|"file_exists"|"regex", ...}`
    pub acceptance: Option<serde_json::Value>,
    /// Which agent owns this item (parent vs sub-agent)
    pub assignee: Option<String>,
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    /// Derived at read time: true while any dependency is not done.
    #[serde(default)]
    pub blocked: bool,
}

/// New task input
#[derive(Debug, Clone, Default)]
pub struct NewTask {
    pub parent_id: Option<i64>,
    pub scope: String,
    pub scope_id: Option<String>,
    pub project: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub priority: i64,
    pub labels: Vec<String>,
    pub tool_scope: Vec<String>,
    pub due_at: Option<String>,
    pub recurrence: Option<String>,
    pub depends_on: Vec<i64>,
    pub acceptance: Option<serde_json::Value>,
    pub assignee: Option<String>,
    pub sort_order: i64,
}

/// Partial task update; `None` fields are left untouched.
///
/// `status` accepts `pending | in_progress` only — `done` must go through
/// `TaskRepository::complete` (acceptance is a verdict, not an assertion) and
/// `blocked` is derived, never written.
#[derive(Debug, Clone, Default)]
pub struct TaskPatch {
    pub parent_id: Option<Option<i64>>,
    pub project: Option<Option<String>>,
    pub title: Option<String>,
    pub description: Option<Option<String>>,
    pub status: Option<String>,
    pub priority: Option<i64>,
    pub labels: Option<Vec<String>>,
    pub tool_scope: Option<Vec<String>>,
    pub due_at: Option<Option<String>>,
    pub recurrence: Option<Option<String>>,
    pub depends_on: Option<Vec<i64>>,
    pub acceptance: Option<Option<serde_json::Value>>,
    pub assignee: Option<Option<String>>,
    pub sort_order: Option<i64>,
}

/// Append-only working note on a task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNote {
    pub id: i64,
    pub task_id: i64,
    pub author: Option<String>,
    pub content: String,
    pub created_at: String,
}

/// Task activity log entry (every transition, with actor)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskActivityEntry {
    pub id: i64,
    pub task_id: i64,
    pub actor: Option<String>,
    pub action: String,
    pub detail: Option<serde_json::Value>,
    pub created_at: String,
}
