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
