#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! Persistent storage for Nanna using Turso
//!
//! Turso is a Rust-native `SQLite` implementation.

mod migrations;
mod models;
mod repositories;

pub use models::*;
pub use repositories::*;

use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Mutex;
use tracing::info;
use turso::{Builder, Connection};

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Database error: {0}")]
    Database(#[from] turso::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Migration error: {0}")]
    Migration(String),
}

/// Storage configuration
#[derive(Debug, Clone)]
pub struct StorageConfig {
    /// Path to local database file (or ":memory:")
    pub path: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            path: "nanna.db".to_string(),
        }
    }
}

/// Main storage interface
pub struct Storage {
    conn: Arc<Mutex<Connection>>,
}

impl Storage {
    /// Create a new storage instance
    pub async fn new(config: &StorageConfig) -> Result<Self, StorageError> {
        info!("Opening database: {}", config.path);
        let db = Builder::new_local(&config.path).build().await?;
        let conn = db.connect()?;
        let storage = Self {
            conn: Arc::new(Mutex::new(conn)),
        };

        storage.migrate().await?;
        Ok(storage)
    }

    /// Create an in-memory storage (for testing)
    pub async fn in_memory() -> Result<Self, StorageError> {
        let db = Builder::new_local(":memory:").build().await?;
        let conn = db.connect()?;
        let storage = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        storage.migrate().await?;
        Ok(storage)
    }

    /// Run database migrations
    async fn migrate(&self) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;

        // Create migrations table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS _migrations (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                applied_at TEXT NOT NULL
            )",
            (),
        )
        .await?;

        // Run migrations
        for (name, sql) in migrations::MIGRATIONS {
            let mut rows = conn
                .query("SELECT 1 FROM _migrations WHERE name = ?1", turso::params![*name])
                .await?;

            let applied = rows.next().await?.is_some();

            if !applied {
                info!("Running migration: {}", name);
                // Execute each statement in the migration
                for statement in sql.split(';').filter(|s| !s.trim().is_empty()) {
                    conn.execute(statement, ()).await?;
                }
                conn.execute(
                    "INSERT INTO _migrations (name, applied_at) VALUES (?1, datetime('now'))",
                    turso::params![*name],
                )
                .await?;
            }
        }

        Ok(())
    }

    /// Get connection reference
    #[must_use] 
    pub const fn conn(&self) -> &Arc<Mutex<Connection>> {
        &self.conn
    }

    // Repository accessors
    #[must_use] 
    pub fn sessions(&self) -> SessionRepository {
        SessionRepository::new(self.conn.clone())
    }

    #[must_use] 
    pub fn messages(&self) -> MessageRepository {
        MessageRepository::new(self.conn.clone())
    }

    #[must_use] 
    pub fn memories(&self) -> MemoryRepository {
        MemoryRepository::new(self.conn.clone())
    }

    #[must_use] 
    pub fn config_store(&self) -> ConfigRepository {
        ConfigRepository::new(self.conn.clone())
    }

    #[must_use] 
    pub fn cron_jobs(&self) -> CronJobRepository {
        CronJobRepository::new(self.conn.clone())
    }

    #[must_use] 
    pub fn job_runs(&self) -> JobRunRepository {
        JobRunRepository::new(self.conn.clone())
    }

    // =========================================================================
    // Convenience methods for GUI
    // =========================================================================

    /// Create a new GUI session
    pub async fn create_gui_session(&self, name: &str) -> Result<Session, StorageError> {
        self.create_gui_session_with_workspace(name, None).await
    }

    /// Create a new GUI session with optional workspace
    pub async fn create_gui_session_with_workspace(&self, name: &str, workspace_id: Option<&str>) -> Result<Session, StorageError> {
        let session_id = uuid::Uuid::new_v4().to_string();
        let conn = self.conn.lock().await;

        conn.execute(
            "INSERT INTO sessions (session_id, channel, user_id, workspace_id, name, metadata) 
             VALUES (?1, 'gui', NULL, ?2, ?3, ?4)",
            turso::params![
                session_id.as_str(), 
                workspace_id,
                name,
                format!("{{\"name\":\"{}\"}}", name).as_str()
            ],
        )
        .await?;

        drop(conn);
        self.sessions().get(&session_id).await
    }

    /// List sessions for GUI (with names from metadata)
    pub async fn list_gui_sessions(&self, limit: i64) -> Result<Vec<Session>, StorageError> {
        self.sessions().list_recent(limit).await
    }

    /// List sessions for GUI filtered by workspace
    pub async fn list_gui_sessions_by_workspace(&self, workspace_id: Option<&str>, limit: i64) -> Result<Vec<Session>, StorageError> {
        self.sessions().list_by_workspace(workspace_id, limit).await
    }

    /// Get messages for a session
    pub async fn get_session_messages(&self, session_id: &str, limit: i64) -> Result<Vec<Message>, StorageError> {
        self.messages().get_by_session(session_id, limit).await
    }

    /// Add a message to a session
    pub async fn add_message(&self, session_id: &str, role: &str, content: &str) -> Result<Message, StorageError> {
        self.messages().create(NewMessage {
            session_id: session_id.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            content_type: "text".to_string(),
            tool_use_id: None,
            tokens_in: None,
            tokens_out: None,
            metadata: None,
        }).await
    }

    /// Add a message with tool calls to a session
    /// Tool calls are stored in the metadata field as JSON
    pub async fn add_message_with_tool_calls(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        tool_calls: Option<serde_json::Value>,
    ) -> Result<Message, StorageError> {
        let metadata = tool_calls.map(|tc| serde_json::json!({ "tool_calls": tc }));
        self.messages().create(NewMessage {
            session_id: session_id.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            content_type: "text".to_string(),
            tool_use_id: None,
            tokens_in: None,
            tokens_out: None,
            metadata,
        }).await
    }

    /// Count messages in a session
    pub async fn count_session_messages(&self, session_id: &str) -> Result<i64, StorageError> {
        let conn = self.conn.lock().await;
        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
                turso::params![session_id],
            )
            .await?;

        if let Some(row) = rows.next().await? {
            Ok(row.get(0)?)
        } else {
            Ok(0)
        }
    }

    /// Update session timestamp
    pub async fn touch_session(&self, session_id: &str) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE sessions SET updated_at = datetime('now') WHERE session_id = ?1",
            turso::params![session_id],
        )
        .await?;
        Ok(())
    }

    /// Rename a session
    pub async fn rename_session(&self, session_id: &str, name: &str) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE sessions SET metadata = json_set(COALESCE(metadata, '{}'), '$.name', ?1), updated_at = datetime('now') WHERE session_id = ?2",
            turso::params![name, session_id],
        )
        .await?;
        Ok(())
    }

    /// Delete a session and its messages
    pub async fn delete_session(&self, session_id: &str) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        conn.execute(
            "DELETE FROM messages WHERE session_id = ?1",
            turso::params![session_id],
        )
        .await?;
        conn.execute(
            "DELETE FROM sessions WHERE session_id = ?1",
            turso::params![session_id],
        )
        .await?;
        Ok(())
    }

    /// Get session name - prefers name field, falls back to metadata, then generates one
    pub fn get_session_name(session: &Session) -> String {
        // First try the name column
        if let Some(name) = &session.name {
            if !name.is_empty() {
                return name.clone();
            }
        }
        // Fall back to metadata
        session.metadata
            .as_ref()
            .and_then(|m| m.get("name"))
            .and_then(|n| n.as_str())
            .map(String::from)
            .unwrap_or_else(|| format!("Session {}", &session.session_id[..8]))
    }

    /// Update session's workspace
    pub async fn set_session_workspace(&self, session_id: &str, workspace_id: Option<&str>) -> Result<(), StorageError> {
        self.sessions().update_workspace(session_id, workspace_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_storage_creation() {
        let storage = Storage::in_memory().await.unwrap();

        let session = storage
            .sessions()
            .create("test-session", "cli", None)
            .await
            .unwrap();

        assert_eq!(session.session_id, "test-session");
    }

    #[tokio::test]
    async fn test_persistence_across_restarts() {
        // Create a temp file path
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join(format!("nanna_test_{}.db", std::process::id()));
        let db_path_str = db_path.to_string_lossy().to_string();

        // Clean up any existing test db
        let _ = std::fs::remove_file(&db_path);

        // Create storage, write data, then drop it
        {
            let config = StorageConfig { path: db_path_str.clone() };
            let storage = Storage::new(&config).await.unwrap();
            
            storage
                .sessions()
                .create("persistent-session", "cli", None)
                .await
                .unwrap();
        }

        // Reopen storage and verify data persisted
        {
            let config = StorageConfig { path: db_path_str.clone() };
            let storage = Storage::new(&config).await.unwrap();
            
            // get() returns Result<Session> - throws NotFound if missing
            let session = storage
                .sessions()
                .get("persistent-session")
                .await
                .expect("Session should persist across restarts");
            
            assert_eq!(session.session_id, "persistent-session");
        }

        // Cleanup
        let _ = std::fs::remove_file(&db_path);
    }
}

// =============================================================================
// Model Stats Persistence
// =============================================================================

/// Stored model statistics row (mirrors nanna-agent::ModelStats without the dependency)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredModelStats {
    pub model: String,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub consecutive_failures: u32,
    pub last_success_epoch_ms: u64,
    pub last_failure_epoch_ms: u64,
    pub tier_successes_simple: u64,
    pub tier_successes_medium: u64,
    pub tier_successes_complex: u64,
    pub tier_failures_simple: u64,
    pub tier_failures_medium: u64,
    pub tier_failures_complex: u64,
    pub escalations: u64,
    pub latencies_ms: Vec<u64>,
    pub throughput_tps: Vec<f64>,
}

impl Storage {
    /// Save model stats to the database (upsert).
    pub async fn save_model_stats(&self, stats: &[StoredModelStats]) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        for s in stats {
            let latencies_json = serde_json::to_string(&s.latencies_ms)?;
            let throughput_json = serde_json::to_string(&s.throughput_tps)?;
            conn.execute(
                "INSERT INTO model_stats (
                    model, total_requests, successful_requests, failed_requests,
                    total_input_tokens, total_output_tokens,
                    total_cache_read_tokens, total_cache_creation_tokens,
                    consecutive_failures, last_success_epoch_ms, last_failure_epoch_ms,
                    tier_successes_simple, tier_successes_medium, tier_successes_complex,
                    tier_failures_simple, tier_failures_medium, tier_failures_complex,
                    escalations, latencies_ms_json, throughput_tps_json, updated_at
                ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,datetime('now'))
                ON CONFLICT(model) DO UPDATE SET
                    total_requests=?2, successful_requests=?3, failed_requests=?4,
                    total_input_tokens=?5, total_output_tokens=?6,
                    total_cache_read_tokens=?7, total_cache_creation_tokens=?8,
                    consecutive_failures=?9, last_success_epoch_ms=?10, last_failure_epoch_ms=?11,
                    tier_successes_simple=?12, tier_successes_medium=?13, tier_successes_complex=?14,
                    tier_failures_simple=?15, tier_failures_medium=?16, tier_failures_complex=?17,
                    escalations=?18, latencies_ms_json=?19, throughput_tps_json=?20, updated_at=datetime('now')",
                turso::params![
                    s.model.clone(),
                    s.total_requests as i64,
                    s.successful_requests as i64,
                    s.failed_requests as i64,
                    s.total_input_tokens as i64,
                    s.total_output_tokens as i64,
                    s.total_cache_read_tokens as i64,
                    s.total_cache_creation_tokens as i64,
                    s.consecutive_failures as i64,
                    s.last_success_epoch_ms as i64,
                    s.last_failure_epoch_ms as i64,
                    s.tier_successes_simple as i64,
                    s.tier_successes_medium as i64,
                    s.tier_successes_complex as i64,
                    s.tier_failures_simple as i64,
                    s.tier_failures_medium as i64,
                    s.tier_failures_complex as i64,
                    s.escalations as i64,
                    latencies_json,
                    throughput_json
                ],
            ).await?;
        }
        Ok(())
    }

    /// Load all model stats from the database.
    pub async fn load_model_stats(&self) -> Result<Vec<StoredModelStats>, StorageError> {
        let conn = self.conn.lock().await;
        let mut rows = conn.query(
            "SELECT model, total_requests, successful_requests, failed_requests,
                    total_input_tokens, total_output_tokens,
                    total_cache_read_tokens, total_cache_creation_tokens,
                    consecutive_failures, last_success_epoch_ms, last_failure_epoch_ms,
                    tier_successes_simple, tier_successes_medium, tier_successes_complex,
                    tier_failures_simple, tier_failures_medium, tier_failures_complex,
                    escalations, latencies_ms_json, throughput_tps_json
             FROM model_stats",
            (),
        ).await?;

        let mut result = Vec::new();
        while let Some(row) = rows.next().await? {
            let latencies_json: String = row.get::<String>(18)?;
            let throughput_json: String = row.get::<String>(19)?;
            result.push(StoredModelStats {
                model: row.get::<String>(0)?,
                total_requests: row.get::<i64>(1)? as u64,
                successful_requests: row.get::<i64>(2)? as u64,
                failed_requests: row.get::<i64>(3)? as u64,
                total_input_tokens: row.get::<i64>(4)? as u64,
                total_output_tokens: row.get::<i64>(5)? as u64,
                total_cache_read_tokens: row.get::<i64>(6)? as u64,
                total_cache_creation_tokens: row.get::<i64>(7)? as u64,
                consecutive_failures: row.get::<i64>(8)? as u32,
                last_success_epoch_ms: row.get::<i64>(9)? as u64,
                last_failure_epoch_ms: row.get::<i64>(10)? as u64,
                tier_successes_simple: row.get::<i64>(11)? as u64,
                tier_successes_medium: row.get::<i64>(12)? as u64,
                tier_successes_complex: row.get::<i64>(13)? as u64,
                tier_failures_simple: row.get::<i64>(14)? as u64,
                tier_failures_medium: row.get::<i64>(15)? as u64,
                tier_failures_complex: row.get::<i64>(16)? as u64,
                escalations: row.get::<i64>(17)? as u64,
                latencies_ms: serde_json::from_str(&latencies_json).unwrap_or_default(),
                throughput_tps: serde_json::from_str(&throughput_json).unwrap_or_default(),
            });
        }
        Ok(result)
    }

    /// Log a single model request observation (detailed per-request log).
    pub async fn log_model_request(
        &self,
        model: &str,
        success: bool,
        latency_ms: u64,
        input_tokens: u32,
        output_tokens: u32,
        cache_read_tokens: u32,
        cache_creation_tokens: u32,
        tier: Option<&str>,
        escalated: bool,
        session_id: Option<&str>,
    ) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO model_request_log (model, success, latency_ms, input_tokens, output_tokens,
                cache_read_tokens, cache_creation_tokens, tier, escalated, session_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            turso::params![
                model,
                success as i64,
                latency_ms as i64,
                input_tokens as i64,
                output_tokens as i64,
                cache_read_tokens as i64,
                cache_creation_tokens as i64,
                tier.unwrap_or(""),
                escalated as i64,
                session_id.unwrap_or("")
            ],
        ).await?;
        Ok(())
    }
}
