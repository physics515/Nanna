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

    #[must_use]
    pub fn workspaces(&self) -> WorkspaceRepository {
        WorkspaceRepository::new(self.conn.clone())
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

    fn sample_new_memory(id: &str, content: &str) -> NewMemory {
        NewMemory {
            memory_id: id.into(),
            content: content.into(),
            embedding: Some(vec![0.1, 0.2, 0.3]),
            embedding_model: Some("test".into()),
            session_id: None,
            metadata: None,
            tags: vec![],
            workspace_id: None,
            fsrs_stability: 1.0,
            fsrs_difficulty: 5.0,
            fsrs_last_access: 0,
            fsrs_access_count: 0,
            fsrs_importance: 1.0,
            fsrs_storage_strength: 1.0,
            fsrs_generation: 0,
        }
    }

    #[test]
    fn is_corruption_error_matches_corruption_messages() {
        // The classifier's contract is a case-insensitive substring match on the
        // rendered message (the form it takes once it crosses into
        // `MemoryError::Persistence(String)`). NotFound is just a convenient
        // String-carrying variant to exercise that contract.
        assert!(is_corruption_error(&StorageError::NotFound(
            "inconsistent overflow chain observed during payload read".into()
        )));
        assert!(is_corruption_error(&StorageError::NotFound(
            "database disk image is CORRUPT".into()
        )));
        assert!(is_corruption_error(&StorageError::NotFound("malformed database page".into())));
        assert!(!is_corruption_error(&StorageError::NotFound("session xyz missing".into())));
    }

    #[tokio::test]
    async fn bulk_load_salvage_matches_bulk_load_on_clean_db() {
        let storage = Storage::in_memory().await.unwrap();
        let repo = storage.memories();
        for i in 0..5 {
            repo.create(sample_new_memory(&format!("m{i}"), &format!("content {i}")))
                .await
                .unwrap();
        }
        let bulk = repo.bulk_load().await.unwrap();
        let report = repo.bulk_load_salvage().await.unwrap();

        assert_eq!(report.expected, 5);
        assert!(report.corrupt_ids.is_empty());
        assert_eq!(report.memories.len(), bulk.len());
        // Same memory_ids in the same order (both ORDER BY id ASC) — the per-id
        // reconstruction is lossless on a clean DB.
        let bulk_ids: Vec<_> = bulk.iter().map(|m| m.memory_id.clone()).collect();
        let salv_ids: Vec<_> = report.memories.iter().map(|m| m.memory_id.clone()).collect();
        assert_eq!(bulk_ids, salv_ids);
        assert_eq!(report.memories[0].embedding, bulk[0].embedding);
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

    // =========================================================================
    // Tool Stats
    // =========================================================================

    /// Log a single tool call (time-series data for graphs).
    pub async fn log_tool_call(
        &self,
        tool_name: &str,
        success: bool,
        duration_ms: u64,
        output_size: usize,
        error_message: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO tool_call_log (tool_name, success, duration_ms, output_size, error_message, session_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            turso::params![
                tool_name,
                success as i64,
                duration_ms as i64,
                output_size as i64,
                error_message.unwrap_or(""),
                session_id.unwrap_or("")
            ],
        ).await?;

        // Also update hourly aggregate
        let hour = chrono::Utc::now().format("%Y-%m-%dT%H:00:00").to_string();
        conn.execute(
            "INSERT INTO tool_stats_hourly (tool_name, hour, call_count, success_count, failure_count, total_duration_ms, avg_duration_ms, p95_duration_ms)
             VALUES (?1, ?2, 1, ?3, ?4, ?5, ?5, ?5)
             ON CONFLICT(tool_name, hour) DO UPDATE SET
                call_count = call_count + 1,
                success_count = success_count + ?3,
                failure_count = failure_count + ?4,
                total_duration_ms = total_duration_ms + ?5,
                avg_duration_ms = (total_duration_ms + ?5) / (call_count + 1),
                p95_duration_ms = MAX(p95_duration_ms, ?5)",
            turso::params![
                tool_name,
                hour,
                success as i64,
                (!success) as i64,
                duration_ms as i64
            ],
        ).await?;

        // Also update daily aggregate
        let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
        conn.execute(
            "INSERT INTO tool_stats_daily (tool_name, day, call_count, success_count, failure_count, total_duration_ms, avg_duration_ms, p95_duration_ms)
             VALUES (?1, ?2, 1, ?3, ?4, ?5, ?5, ?5)
             ON CONFLICT(tool_name, day) DO UPDATE SET
                call_count = call_count + 1,
                success_count = success_count + ?3,
                failure_count = failure_count + ?4,
                total_duration_ms = total_duration_ms + ?5,
                avg_duration_ms = (total_duration_ms + ?5) / (call_count + 1),
                p95_duration_ms = MAX(p95_duration_ms, ?5)",
            turso::params![
                tool_name,
                day,
                success as i64,
                (!success) as i64,
                duration_ms as i64
            ],
        ).await?;

        Ok(())
    }

    /// Get hourly tool stats for a given time range (for graphs).
    /// Returns data for the last `hours` hours.
    pub async fn get_tool_stats_hourly(
        &self,
        tool_name: Option<&str>,
        hours: u32,
    ) -> Result<Vec<ToolStatsTimeBucket>, StorageError> {
        let conn = self.conn.lock().await;
        let since = chrono::Utc::now() - chrono::Duration::hours(i64::from(hours));
        let since_str = since.format("%Y-%m-%dT%H:00:00").to_string();

        let mut rows = if let Some(name) = tool_name {
            conn.query(
                "SELECT tool_name, hour, call_count, success_count, failure_count, 
                        total_duration_ms, avg_duration_ms, p95_duration_ms
                 FROM tool_stats_hourly
                 WHERE tool_name = ?1 AND hour >= ?2
                 ORDER BY hour ASC",
                turso::params![name, since_str],
            ).await?
        } else {
            conn.query(
                "SELECT 'all' as tool_name, hour, 
                        CAST(SUM(call_count) AS INTEGER),
                        CAST(SUM(success_count) AS INTEGER),
                        CAST(SUM(failure_count) AS INTEGER),
                        CAST(SUM(total_duration_ms) AS INTEGER),
                        CAST(AVG(avg_duration_ms) AS INTEGER),
                        CAST(MAX(p95_duration_ms) AS INTEGER)
                 FROM tool_stats_hourly
                 WHERE hour >= ?1
                 GROUP BY hour
                 ORDER BY hour ASC",
                turso::params![since_str],
            ).await?
        };
        let mut result = Vec::new();
        while let Some(row) = rows.next().await? {
            result.push(ToolStatsTimeBucket {
                tool_name: row.get::<String>(0)?,
                period: row.get::<String>(1)?,
                call_count: row.get::<i64>(2)? as u64,
                success_count: row.get::<i64>(3)? as u64,
                failure_count: row.get::<i64>(4)? as u64,
                total_duration_ms: row.get::<i64>(5)? as u64,
                avg_duration_ms: row.get::<i64>(6)? as u64,
                p95_duration_ms: row.get::<i64>(7)? as u64,
            });
        }
        Ok(result)
    }

    /// Get daily tool stats for a given time range (for long-term graphs).
    /// Returns data for the last `days` days.
    pub async fn get_tool_stats_daily(
        &self,
        tool_name: Option<&str>,
        days: u32,
    ) -> Result<Vec<ToolStatsTimeBucket>, StorageError> {
        let conn = self.conn.lock().await;
        let since = chrono::Utc::now() - chrono::Duration::days(i64::from(days));
        let since_str = since.format("%Y-%m-%d").to_string();

        let mut rows = if let Some(name) = tool_name {
            conn.query(
                "SELECT tool_name, day, call_count, success_count, failure_count,
                        total_duration_ms, avg_duration_ms, p95_duration_ms
                 FROM tool_stats_daily
                 WHERE tool_name = ?1 AND day >= ?2
                 ORDER BY day ASC",
                turso::params![name, since_str],
            ).await?
        } else {
            conn.query(
                "SELECT 'all' as tool_name, day,
                        CAST(SUM(call_count) AS INTEGER),
                        CAST(SUM(success_count) AS INTEGER),
                        CAST(SUM(failure_count) AS INTEGER),
                        CAST(SUM(total_duration_ms) AS INTEGER),
                        CAST(AVG(avg_duration_ms) AS INTEGER),
                        CAST(MAX(p95_duration_ms) AS INTEGER)
                 FROM tool_stats_daily
                 WHERE day >= ?1
                 GROUP BY day
                 ORDER BY day ASC",
                turso::params![since_str],
            ).await?
        };
        let mut result = Vec::new();
        while let Some(row) = rows.next().await? {
            result.push(ToolStatsTimeBucket {
                tool_name: row.get::<String>(0)?,
                period: row.get::<String>(1)?,
                call_count: row.get::<i64>(2)? as u64,
                success_count: row.get::<i64>(3)? as u64,
                failure_count: row.get::<i64>(4)? as u64,
                total_duration_ms: row.get::<i64>(5)? as u64,
                avg_duration_ms: row.get::<i64>(6)? as u64,
                p95_duration_ms: row.get::<i64>(7)? as u64,
            });
        }
        Ok(result)
    }

    /// Get recent tool call log entries (for detail views).
    pub async fn get_tool_call_log(
        &self,
        tool_name: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ToolCallLogEntry>, StorageError> {
        let conn = self.conn.lock().await;

        let mut rows = if let Some(name) = tool_name {
            conn.query(
                "SELECT tool_name, success, duration_ms, output_size, error_message, session_id, created_at
                 FROM tool_call_log
                 WHERE tool_name = ?1
                 ORDER BY created_at DESC
                 LIMIT ?2",
                turso::params![name, limit as i64],
            ).await?
        } else {
            conn.query(
                "SELECT tool_name, success, duration_ms, output_size, error_message, session_id, created_at
                 FROM tool_call_log
                 ORDER BY created_at DESC
                 LIMIT ?1",
                turso::params![limit as i64],
            ).await?
        };
        let mut result = Vec::new();
        while let Some(row) = rows.next().await? {
            result.push(ToolCallLogEntry {
                tool_name: row.get::<String>(0)?,
                success: row.get::<i64>(1)? != 0,
                duration_ms: row.get::<i64>(2)? as u64,
                output_size: row.get::<i64>(3)? as u64,
                error_message: {
                    let s: String = row.get::<String>(4)?;
                    if s.is_empty() { None } else { Some(s) }
                },
                session_id: {
                    let s: String = row.get::<String>(5)?;
                    if s.is_empty() { None } else { Some(s) }
                },
                created_at: row.get::<String>(6)?,
            });
        }
        Ok(result)
    }

    /// Prune old tool call logs (keep last N days).
    pub async fn prune_tool_call_log(&self, keep_days: u32) -> Result<u64, StorageError> {
        let conn = self.conn.lock().await;
        let cutoff = chrono::Utc::now() - chrono::Duration::days(i64::from(keep_days));
        let cutoff_str = cutoff.to_rfc3339();
        conn.execute(
            "DELETE FROM tool_call_log WHERE created_at < ?1",
            turso::params![cutoff_str],
        ).await?;
        // Return approximate count (turso doesn't give affected rows easily)
        Ok(0)
    }
}

// =========================================================================
// Checkpoints
// =========================================================================

impl Storage {
    /// Save a checkpoint for crash recovery.
    pub async fn save_checkpoint(&self, session_id: &str, data: &str) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO checkpoints (session_id, data, updated_at)
             VALUES (?1, ?2, datetime('now'))
             ON CONFLICT(session_id) DO UPDATE SET data = ?2, updated_at = datetime('now')",
            turso::params![session_id, data],
        ).await?;
        Ok(())
    }

    /// Load a checkpoint for a session.
    pub async fn load_checkpoint(&self, session_id: &str) -> Result<Option<String>, StorageError> {
        let conn = self.conn.lock().await;
        let mut rows = conn.query(
            "SELECT data FROM checkpoints WHERE session_id = ?1",
            turso::params![session_id],
        ).await?;
        if let Some(row) = rows.next().await? {
            Ok(Some(row.get::<String>(0)?))
        } else {
            Ok(None)
        }
    }

    /// Delete a checkpoint after successful completion.
    pub async fn delete_checkpoint(&self, session_id: &str) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        conn.execute(
            "DELETE FROM checkpoints WHERE session_id = ?1",
            turso::params![session_id],
        ).await?;
        Ok(())
    }

    /// List all checkpoint session IDs (for recovery at startup).
    pub async fn list_checkpoints(&self) -> Result<Vec<String>, StorageError> {
        let conn = self.conn.lock().await;
        let mut rows = conn.query(
            "SELECT session_id FROM checkpoints",
            (),
        ).await?;
        let mut ids = Vec::new();
        while let Some(row) = rows.next().await? {
            ids.push(row.get::<String>(0)?);
        }
        Ok(ids)
    }

    // =========================================================================
    // Tool Stats (aggregated — replaces tool-stats.json)
    // =========================================================================

    /// Save aggregated tool stats to the tool_stats table (upsert).
    pub async fn save_tool_stats_aggregated(&self, stats: &serde_json::Value) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        // The JSON is expected to be { "tools": { "tool_name": { stats... }, ... }, "sessions": N }
        if let Some(tools) = stats.get("tools").and_then(|v| v.as_object()) {
            for (tool_name, tool_data) in tools {
                let call_count = tool_data.get("call_count").and_then(|v| v.as_i64()).unwrap_or(0);
                let success_count = tool_data.get("success_count").and_then(|v| v.as_i64()).unwrap_or(0);
                let failure_count = tool_data.get("failure_count").and_then(|v| v.as_i64()).unwrap_or(0);
                let total_duration_ms = tool_data.get("total_duration_ms").and_then(|v| v.as_i64()).unwrap_or(0);
                let last_called_epoch_ms = tool_data.get("last_called_epoch_ms").and_then(|v| v.as_i64()).unwrap_or(0);
                let latencies = tool_data.get("latencies_ms").map(|v| v.to_string()).unwrap_or_else(|| "[]".to_string());
                let output_sizes = tool_data.get("output_sizes").map(|v| v.to_string()).unwrap_or_else(|| "[]".to_string());
                let errors = tool_data.get("errors").map(|v| v.to_string()).unwrap_or_else(|| "[]".to_string());

                conn.execute(
                    "INSERT INTO tool_stats (tool_name, call_count, success_count, failure_count, total_duration_ms, last_called_epoch_ms, latencies_ms_json, output_sizes_json, errors_json, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))
                     ON CONFLICT(tool_name) DO UPDATE SET
                        call_count = ?2, success_count = ?3, failure_count = ?4,
                        total_duration_ms = ?5, last_called_epoch_ms = ?6,
                        latencies_ms_json = ?7, output_sizes_json = ?8, errors_json = ?9,
                        updated_at = datetime('now')",
                    turso::params![
                        tool_name.as_str(),
                        call_count,
                        success_count,
                        failure_count,
                        total_duration_ms,
                        last_called_epoch_ms,
                        latencies.as_str(),
                        output_sizes.as_str(),
                        errors.as_str()
                    ],
                ).await?;
            }
        }
        Ok(())
    }

    /// Load aggregated tool stats from the tool_stats table (returns the JSON format ToolStatsTracker expects).
    pub async fn load_tool_stats_aggregated(&self) -> Result<serde_json::Value, StorageError> {
        let conn = self.conn.lock().await;
        let mut rows = conn.query(
            "SELECT tool_name, call_count, success_count, failure_count, total_duration_ms, last_called_epoch_ms, latencies_ms_json, output_sizes_json, errors_json
             FROM tool_stats",
            (),
        ).await?;

        let mut tools = serde_json::Map::new();
        let mut session_count: i64 = 0;
        while let Some(row) = rows.next().await? {
            let tool_name: String = row.get(0)?;
            let call_count: i64 = row.get(1)?;
            let success_count: i64 = row.get(2)?;
            let failure_count: i64 = row.get(3)?;
            let total_duration_ms: i64 = row.get(4)?;
            let last_called_epoch_ms: i64 = row.get(5)?;
            let latencies_str: String = row.get(6)?;
            let output_sizes_str: String = row.get(7)?;
            let errors_str: String = row.get(8)?;

            let latencies: serde_json::Value = serde_json::from_str(&latencies_str).unwrap_or(serde_json::json!([]));
            let output_sizes: serde_json::Value = serde_json::from_str(&output_sizes_str).unwrap_or(serde_json::json!([]));
            let errors: serde_json::Value = serde_json::from_str(&errors_str).unwrap_or(serde_json::json!([]));

            session_count += call_count;
            tools.insert(tool_name, serde_json::json!({
                "call_count": call_count,
                "success_count": success_count,
                "failure_count": failure_count,
                "total_duration_ms": total_duration_ms,
                "last_called_epoch_ms": last_called_epoch_ms,
                "latencies_ms": latencies,
                "output_sizes": output_sizes,
                "errors": errors,
            }));
        }

        Ok(serde_json::json!({
            "tools": tools,
            "sessions": session_count
        }))
    }

    // =========================================================================
    // Daemon Session Persistence (replaces sessions.json)
    // =========================================================================

    /// Create or update a daemon session in the database.
    /// This handles the daemon's Session struct format (different from GUI sessions).
    pub async fn upsert_daemon_session(
        &self,
        session_id: &str,
        name: Option<&str>,
        workspace_id: Option<&str>,
        created_at: &str,
        updated_at: &str,
        metadata: Option<&str>,
    ) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO sessions (session_id, channel, name, workspace_id, created_at, updated_at, metadata)
             VALUES (?1, 'gui', ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(session_id) DO UPDATE SET
                name = COALESCE(?2, name),
                workspace_id = COALESCE(?3, workspace_id),
                updated_at = ?5,
                metadata = COALESCE(?6, metadata)",
            turso::params![session_id, name, workspace_id, created_at, updated_at, metadata],
        ).await?;
        Ok(())
    }

    /// Add a daemon message to the messages table.
    /// Stores tool_calls, attachments, and reasoning in the metadata JSON field.
    pub async fn add_daemon_message(
        &self,
        session_id: &str,
        message_id: &str,
        role: &str,
        content: &str,
        created_at: &str,
        metadata: Option<&str>,
    ) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO messages (session_id, role, content, content_type, tool_use_id, created_at, metadata)
             VALUES (?1, ?2, ?3, 'text', ?4, ?5, ?6)",
            turso::params![session_id, role, content, message_id, created_at, metadata],
        ).await?;
        // Touch session
        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE session_id = ?2",
            turso::params![created_at, session_id],
        ).await?;
        Ok(())
    }

    /// Load all sessions from the database (regardless of channel).
    pub async fn list_daemon_sessions(&self) -> Result<Vec<Session>, StorageError> {
        let conn = self.conn.lock().await;
        let mut rows = conn.query(
            "SELECT id, session_id, channel, user_id, created_at, updated_at, metadata, workspace_id, name
             FROM sessions ORDER BY updated_at DESC",
            (),
        ).await?;
        let mut sessions = Vec::new();
        while let Some(row) = rows.next().await? {
            let metadata_str: Option<String> = row.get(6)?;
            sessions.push(Session {
                id: row.get(0)?,
                session_id: row.get(1)?,
                channel: row.get(2)?,
                user_id: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
                metadata: metadata_str.and_then(|s| serde_json::from_str(&s).ok()),
                workspace_id: row.get(7)?,
                name: row.get(8)?,
            });
        }
        Ok(sessions)
    }

    /// Load all messages for a daemon session, ordered by creation time.
    pub async fn load_daemon_messages(&self, session_id: &str) -> Result<Vec<Message>, StorageError> {
        let conn = self.conn.lock().await;
        let mut rows = conn.query(
            "SELECT id, session_id, role, content, content_type, tool_use_id, created_at, tokens_in, tokens_out, metadata
             FROM messages WHERE session_id = ?1 ORDER BY created_at ASC",
            turso::params![session_id],
        ).await?;
        let mut messages = Vec::new();
        while let Some(row) = rows.next().await? {
            let metadata_str: Option<String> = row.get(9)?;
            messages.push(Message {
                id: row.get(0)?,
                session_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                content_type: row.get(4)?,
                tool_use_id: row.get(5)?,
                created_at: row.get(6)?,
                tokens_in: row.get(7)?,
                tokens_out: row.get(8)?,
                metadata: metadata_str.and_then(|s| serde_json::from_str(&s).ok()),
            });
        }
        Ok(messages)
    }

    /// Delete all messages for a daemon session.
    pub async fn clear_daemon_session_messages(&self, session_id: &str) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        conn.execute(
            "DELETE FROM messages WHERE session_id = ?1",
            turso::params![session_id],
        ).await?;
        Ok(())
    }

    /// Delete a daemon session and its messages.
    pub async fn delete_daemon_session(&self, session_id: &str) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        conn.execute("DELETE FROM messages WHERE session_id = ?1", turso::params![session_id]).await?;
        conn.execute("DELETE FROM sessions WHERE session_id = ?1", turso::params![session_id]).await?;
        Ok(())
    }

    /// Update daemon session name.
    pub async fn rename_daemon_session(&self, session_id: &str, name: &str) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE sessions SET name = ?1, updated_at = datetime('now') WHERE session_id = ?2",
            turso::params![name, session_id],
        ).await?;
        Ok(())
    }

    /// Update daemon session workspace.
    pub async fn set_daemon_session_workspace(&self, session_id: &str, workspace_id: Option<&str>) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE sessions SET workspace_id = ?1, updated_at = datetime('now') WHERE session_id = ?2",
            turso::params![workspace_id, session_id],
        ).await?;
        Ok(())
    }
}

/// Time-bucketed tool statistics (hourly or daily).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolStatsTimeBucket {
    pub tool_name: String,
    pub period: String,
    pub call_count: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub total_duration_ms: u64,
    pub avg_duration_ms: u64,
    pub p95_duration_ms: u64,
}

/// A single tool call log entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolCallLogEntry {
    pub tool_name: String,
    pub success: bool,
    pub duration_ms: u64,
    pub output_size: u64,
    pub error_message: Option<String>,
    pub session_id: Option<String>,
    pub created_at: String,
}
