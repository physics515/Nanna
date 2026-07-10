//! Repository implementations using Turso

use crate::{CronJob, JobRun, Memory, Message, NewCronJob, NewJobRun, NewMemory, NewMessage, Session, StorageError, WorkspaceRecord};
use std::sync::Arc;
use tokio::sync::Mutex;
use turso::Connection;

/// Session repository
pub struct SessionRepository {
    conn: Arc<Mutex<Connection>>,
}

impl SessionRepository {
    pub const fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub async fn create(
        &self,
        session_id: &str,
        channel: &str,
        user_id: Option<&str>,
    ) -> Result<Session, StorageError> {
        self.create_with_workspace(session_id, channel, user_id, None, None).await
    }

    pub async fn create_with_workspace(
        &self,
        session_id: &str,
        channel: &str,
        user_id: Option<&str>,
        workspace_id: Option<&str>,
        name: Option<&str>,
    ) -> Result<Session, StorageError> {
        let conn = self.conn.lock().await;

        conn.execute(
            "INSERT INTO sessions (session_id, channel, user_id, workspace_id, name) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(session_id) DO UPDATE SET updated_at = datetime('now')",
            turso::params![session_id, channel, user_id, workspace_id, name],
        )
        .await?;

        drop(conn);
        self.get(session_id).await
    }

    pub async fn get(&self, session_id: &str) -> Result<Session, StorageError> {
        let conn = self.conn.lock().await;

        let mut rows = conn
            .query(
                "SELECT id, session_id, channel, user_id, created_at, updated_at, metadata, workspace_id, name 
                 FROM sessions WHERE session_id = ?1",
                turso::params![session_id],
            )
            .await?;

        if let Some(row) = rows.next().await? {
            let metadata_str: Option<String> = row.get(6)?;
            Ok(Session {
                id: row.get(0)?,
                session_id: row.get(1)?,
                channel: row.get(2)?,
                user_id: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
                metadata: metadata_str.and_then(|s| serde_json::from_str(&s).ok()),
                workspace_id: row.get(7)?,
                name: row.get(8)?,
            })
        } else {
            Err(StorageError::NotFound(format!("Session: {session_id}")))
        }
    }

    pub async fn list_recent(&self, limit: i64) -> Result<Vec<Session>, StorageError> {
        let conn = self.conn.lock().await;

        let mut rows = conn
            .query(
                "SELECT id, session_id, channel, user_id, created_at, updated_at, metadata, workspace_id, name 
                 FROM sessions ORDER BY updated_at DESC LIMIT ?1",
                turso::params![limit],
            )
            .await?;

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

    /// List sessions for a specific workspace (or global if workspace_id is None)
    pub async fn list_by_workspace(&self, workspace_id: Option<&str>, limit: i64) -> Result<Vec<Session>, StorageError> {
        let conn = self.conn.lock().await;

        let mut rows = match workspace_id {
            Some(ws_id) => {
                conn.query(
                    "SELECT id, session_id, channel, user_id, created_at, updated_at, metadata, workspace_id, name 
                     FROM sessions WHERE workspace_id = ?1 ORDER BY updated_at DESC LIMIT ?2",
                    turso::params![ws_id, limit],
                ).await?
            }
            None => {
                conn.query(
                    "SELECT id, session_id, channel, user_id, created_at, updated_at, metadata, workspace_id, name 
                     FROM sessions WHERE workspace_id IS NULL ORDER BY updated_at DESC LIMIT ?1",
                    turso::params![limit],
                ).await?
            }
        };

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

    /// Update session name
    pub async fn update_name(&self, session_id: &str, name: &str) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE sessions SET name = ?1, updated_at = datetime('now') WHERE session_id = ?2",
            turso::params![name, session_id],
        ).await?;
        Ok(())
    }

    /// Update session workspace
    pub async fn update_workspace(&self, session_id: &str, workspace_id: Option<&str>) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE sessions SET workspace_id = ?1, updated_at = datetime('now') WHERE session_id = ?2",
            turso::params![workspace_id, session_id],
        ).await?;
        Ok(())
    }
}

/// Message repository
pub struct MessageRepository {
    conn: Arc<Mutex<Connection>>,
}

impl MessageRepository {
    pub const fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub async fn create(&self, msg: NewMessage) -> Result<Message, StorageError> {
        let conn = self.conn.lock().await;

        let metadata_json = msg.metadata.as_ref().map(std::string::ToString::to_string);

        conn.execute(
            "INSERT INTO messages (session_id, role, content, content_type, tool_use_id, tokens_in, tokens_out, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            turso::params![
                msg.session_id.as_str(),
                msg.role.as_str(),
                msg.content.as_str(),
                msg.content_type.as_str(),
                msg.tool_use_id.as_deref(),
                msg.tokens_in,
                msg.tokens_out,
                metadata_json.as_deref(),
            ],
        )
        .await?;

        // Update session timestamp
        conn.execute(
            "UPDATE sessions SET updated_at = datetime('now') WHERE session_id = ?1",
            turso::params![msg.session_id.as_str()],
        )
        .await?;

        // Get the inserted message
        let mut rows = conn
            .query(
                "SELECT id, session_id, role, content, content_type, tool_use_id, created_at, tokens_in, tokens_out, metadata
                 FROM messages ORDER BY id DESC LIMIT 1",
                (),
            )
            .await?;

        if let Some(row) = rows.next().await? {
            let metadata_str: Option<String> = row.get(9)?;
            Ok(Message {
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
            })
        } else {
            Err(StorageError::NotFound("Message just created".to_string()))
        }
    }

    pub async fn get_by_session(
        &self,
        session_id: &str,
        limit: i64,
    ) -> Result<Vec<Message>, StorageError> {
        let conn = self.conn.lock().await;

        let mut rows = conn
            .query(
                "SELECT id, session_id, role, content, content_type, tool_use_id, created_at, tokens_in, tokens_out, metadata
                 FROM messages WHERE session_id = ?1 ORDER BY created_at ASC LIMIT ?2",
                turso::params![session_id, limit],
            )
            .await?;

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
}

/// Memory repository (for vector search)
pub struct MemoryRepository {
    conn: Arc<Mutex<Connection>>,
}

/// Helper to decode FSRS columns from row index 9..16
/// Expected column order after the base 9:
/// 9=workspace_id, 10=fsrs_stability, 11=fsrs_difficulty,
/// 12=fsrs_last_access, 13=fsrs_access_count, 14=fsrs_importance,
/// 15=fsrs_storage_strength, 16=fsrs_generation
fn decode_memory_row(
    row: &turso::Row,
    embedding: Option<Vec<f32>>,
    metadata_str: Option<String>,
    tags: Vec<String>,
) -> Result<Memory, turso::Error> {
    Ok(Memory {
        id: row.get(0)?,
        memory_id: row.get(1)?,
        content: row.get(2)?,
        embedding,
        embedding_model: row.get(4)?,
        session_id: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        metadata: metadata_str.and_then(|s| serde_json::from_str(&s).ok()),
        tags,
        workspace_id: row.get(9)?,
        fsrs_stability: row.get::<f64>(10)? as f32,
        fsrs_difficulty: row.get::<f64>(11)? as f32,
        fsrs_last_access: row.get(12)?,
        fsrs_access_count: row.get(13)?,
        fsrs_importance: row.get::<f64>(14)? as f32,
        fsrs_storage_strength: row.get::<f64>(15)? as f32,
        fsrs_generation: row.get(16)?,
    })
}

fn decode_embedding(bytes: Option<Vec<u8>>) -> Option<Vec<f32>> {
    bytes.map(|bytes| {
        bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect()
    })
}

impl MemoryRepository {
    pub const fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub async fn create(&self, mem: NewMemory) -> Result<Memory, StorageError> {
        let conn = self.conn.lock().await;

        // Serialize embedding as bytes
        let embedding_bytes: Option<Vec<u8>> = mem.embedding.as_ref().map(|e| {
            e.iter().flat_map(|f| f.to_le_bytes()).collect()
        });

        let metadata_json = mem.metadata.as_ref().map(std::string::ToString::to_string);
        let memory_id = mem.memory_id.clone();

        conn.execute(
            "INSERT INTO memories (memory_id, content, embedding, embedding_model, session_id, metadata,
                                   workspace_id,
                                   fsrs_stability, fsrs_difficulty, fsrs_last_access, fsrs_access_count,
                                   fsrs_importance, fsrs_storage_strength, fsrs_generation)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            turso::params![
                mem.memory_id.as_str(),
                mem.content.as_str(),
                embedding_bytes,
                mem.embedding_model.as_deref(),
                mem.session_id.as_deref(),
                metadata_json.as_deref(),
                mem.workspace_id.as_deref(),
                mem.fsrs_stability as f64,
                mem.fsrs_difficulty as f64,
                mem.fsrs_last_access,
                mem.fsrs_access_count,
                mem.fsrs_importance as f64,
                mem.fsrs_storage_strength as f64,
                mem.fsrs_generation,
            ],
        )
        .await?;

        // Add tags
        for tag in &mem.tags {
            conn.execute(
                "INSERT INTO memory_tags (memory_id, tag) VALUES (?1, ?2)",
                turso::params![mem.memory_id.as_str(), tag.as_str()],
            )
            .await?;
        }

        drop(conn);
        self.get(&memory_id).await
    }

    pub async fn get(&self, memory_id: &str) -> Result<Memory, StorageError> {
        let conn = self.conn.lock().await;

        let mut rows = conn
            .query(
                "SELECT id, memory_id, content, embedding, embedding_model, session_id, created_at, updated_at, metadata,
                        workspace_id,
                        fsrs_stability, fsrs_difficulty, fsrs_last_access, fsrs_access_count,
                        fsrs_importance, fsrs_storage_strength, fsrs_generation
                 FROM memories WHERE memory_id = ?1",
                turso::params![memory_id],
            )
            .await?;

        if let Some(row) = rows.next().await? {
            let embedding_bytes: Option<Vec<u8>> = row.get(3)?;
            let embedding = decode_embedding(embedding_bytes);
            let metadata_str: Option<String> = row.get(8)?;

            // Get tags
            let mut tag_rows = conn
                .query(
                    "SELECT tag FROM memory_tags WHERE memory_id = ?1",
                    turso::params![memory_id],
                )
                .await?;

            let mut tags = Vec::new();
            while let Some(tag_row) = tag_rows.next().await? {
                tags.push(tag_row.get(0)?);
            }

            Ok(decode_memory_row(&row, embedding, metadata_str, tags)?)
        } else {
            Err(StorageError::NotFound(format!("Memory: {memory_id}")))
        }
    }

    pub async fn list_all(&self, limit: i64) -> Result<Vec<Memory>, StorageError> {
        let conn = self.conn.lock().await;

        let mut rows = conn
            .query(
                "SELECT id, memory_id, content, embedding, embedding_model, session_id, created_at, updated_at, metadata,
                        workspace_id,
                        fsrs_stability, fsrs_difficulty, fsrs_last_access, fsrs_access_count,
                        fsrs_importance, fsrs_storage_strength, fsrs_generation
                 FROM memories ORDER BY updated_at DESC LIMIT ?1",
                turso::params![limit],
            )
            .await?;

        let mut memories = Vec::new();
        while let Some(row) = rows.next().await? {
            let embedding_bytes: Option<Vec<u8>> = row.get(3)?;
            let embedding = decode_embedding(embedding_bytes);
            let metadata_str: Option<String> = row.get(8)?;
            memories.push(decode_memory_row(&row, embedding, metadata_str, Vec::new())?);
        }

        Ok(memories)
    }

    /// Load ALL entries efficiently (no limit) — for populating the in-memory cache on startup.
    pub async fn bulk_load(&self) -> Result<Vec<Memory>, StorageError> {
        let conn = self.conn.lock().await;

        let mut rows = conn
            .query(
                "SELECT id, memory_id, content, embedding, embedding_model, session_id, created_at, updated_at, metadata,
                        workspace_id,
                        fsrs_stability, fsrs_difficulty, fsrs_last_access, fsrs_access_count,
                        fsrs_importance, fsrs_storage_strength, fsrs_generation
                 FROM memories ORDER BY id ASC",
                (),
            )
            .await?;

        let mut memories = Vec::new();
        while let Some(row) = rows.next().await? {
            let embedding_bytes: Option<Vec<u8>> = row.get(3)?;
            let embedding = decode_embedding(embedding_bytes);
            let metadata_str: Option<String> = row.get(8)?;
            memories.push(decode_memory_row(&row, embedding, metadata_str, Vec::new())?);
        }

        Ok(memories)
    }

    /// Count total memories in the database.
    pub async fn count(&self) -> Result<i64, StorageError> {
        let conn = self.conn.lock().await;
        let mut rows = conn
            .query("SELECT COUNT(*) FROM memories", ())
            .await?;
        if let Some(row) = rows.next().await? {
            Ok(row.get(0)?)
        } else {
            Ok(0)
        }
    }

    /// Update FSRS state for a memory entry identified by memory_id.
    pub async fn update_fsrs(
        &self,
        memory_id: &str,
        stability: f32,
        difficulty: f32,
        last_access: i64,
        access_count: i64,
        importance: f32,
        storage_strength: f32,
        generation: i64,
    ) -> Result<bool, StorageError> {
        let conn = self.conn.lock().await;
        let result = conn
            .execute(
                "UPDATE memories SET
                    fsrs_stability = ?1, fsrs_difficulty = ?2, fsrs_last_access = ?3,
                    fsrs_access_count = ?4, fsrs_importance = ?5,
                    fsrs_storage_strength = ?6, fsrs_generation = ?7,
                    updated_at = datetime('now')
                 WHERE memory_id = ?8",
                turso::params![
                    stability as f64,
                    difficulty as f64,
                    last_access,
                    access_count,
                    importance as f64,
                    storage_strength as f64,
                    generation,
                    memory_id,
                ],
            )
            .await?;
        Ok(result > 0)
    }

    /// Update content text for a memory entry.
    pub async fn update_content(&self, memory_id: &str, content: &str) -> Result<bool, StorageError> {
        let conn = self.conn.lock().await;
        let result = conn
            .execute(
                "UPDATE memories SET content = ?1, updated_at = datetime('now') WHERE memory_id = ?2",
                turso::params![content, memory_id],
            )
            .await?;
        Ok(result > 0)
    }

    /// Delete multiple memories by their memory_ids.
    pub async fn bulk_delete(&self, ids: &[&str]) -> Result<u64, StorageError> {
        if ids.is_empty() {
            return Ok(0);
        }
        let conn = self.conn.lock().await;
        let mut deleted = 0u64;
        for id in ids {
            // Delete tags first
            conn.execute(
                "DELETE FROM memory_tags WHERE memory_id = ?1",
                turso::params![*id],
            ).await?;
            let n = conn
                .execute(
                    "DELETE FROM memories WHERE memory_id = ?1",
                    turso::params![*id],
                )
                .await?;
            deleted += n as u64;
        }
        Ok(deleted)
    }

    pub async fn delete(&self, memory_id: &str) -> Result<bool, StorageError> {
        let conn = self.conn.lock().await;

        // Delete tags first
        conn.execute(
            "DELETE FROM memory_tags WHERE memory_id = ?1",
            turso::params![memory_id],
        )
        .await?;

        // Delete memory
        let result = conn
            .execute(
                "DELETE FROM memories WHERE memory_id = ?1",
                turso::params![memory_id],
            )
            .await?;

        Ok(result > 0)
    }
}

/// Config repository (key-value store)
pub struct ConfigRepository {
    conn: Arc<Mutex<Connection>>,
}

impl ConfigRepository {
    pub const fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub async fn get(&self, key: &str) -> Result<Option<String>, StorageError> {
        let conn = self.conn.lock().await;

        let mut rows = conn
            .query("SELECT value FROM config WHERE key = ?1", turso::params![key])
            .await?;

        if let Some(row) = rows.next().await? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub async fn set(&self, key: &str, value: &str) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;

        conn.execute(
            "INSERT INTO config (key, value, updated_at) VALUES (?1, ?2, datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = datetime('now')",
            turso::params![key, value],
        )
        .await?;

        Ok(())
    }

    pub async fn delete(&self, key: &str) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        conn.execute("DELETE FROM config WHERE key = ?1", turso::params![key])
            .await?;
        Ok(())
    }

    pub async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Option<T>, StorageError> {
        if let Some(value) = self.get(key).await? {
            Ok(Some(serde_json::from_str(&value)?))
        } else {
            Ok(None)
        }
    }

    pub async fn set_json<T: serde::Serialize>(&self, key: &str, value: &T) -> Result<(), StorageError> {
        let json = serde_json::to_string(value)?;
        self.set(key, &json).await
    }
}

/// Cron job repository
pub struct CronJobRepository {
    conn: Arc<Mutex<Connection>>,
}

impl CronJobRepository {
    pub const fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub async fn create(&self, job: NewCronJob) -> Result<CronJob, StorageError> {
        let conn = self.conn.lock().await;

        let task_json = job.task.to_string();
        let metadata_json = job.metadata.as_ref().map(std::string::ToString::to_string);

        conn.execute(
            "INSERT INTO cron_jobs (job_id, schedule, task, enabled, next_run, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(job_id) DO UPDATE SET 
                schedule = excluded.schedule,
                task = excluded.task,
                enabled = excluded.enabled,
                next_run = excluded.next_run,
                metadata = excluded.metadata",
            turso::params![
                job.job_id.as_str(),
                job.schedule.as_str(),
                task_json.as_str(),
                job.enabled,
                job.next_run.as_deref(),
                metadata_json.as_deref(),
            ],
        )
        .await?;

        drop(conn);
        self.get(&job.job_id).await
    }

    pub async fn get(&self, job_id: &str) -> Result<CronJob, StorageError> {
        let conn = self.conn.lock().await;

        let mut rows = conn
            .query(
                "SELECT id, job_id, schedule, task, enabled, last_run, next_run, created_at, metadata
                 FROM cron_jobs WHERE job_id = ?1",
                turso::params![job_id],
            )
            .await?;

        if let Some(row) = rows.next().await? {
            let task_str: String = row.get(3)?;
            let metadata_str: Option<String> = row.get(8)?;
            let enabled: i64 = row.get(4)?;

            Ok(CronJob {
                id: row.get(0)?,
                job_id: row.get(1)?,
                schedule: row.get(2)?,
                task: serde_json::from_str(&task_str).unwrap_or_default(),
                enabled: enabled != 0,
                last_run: row.get(5)?,
                next_run: row.get(6)?,
                created_at: row.get(7)?,
                metadata: metadata_str.and_then(|s| serde_json::from_str(&s).ok()),
            })
        } else {
            Err(StorageError::NotFound(format!("CronJob: {job_id}")))
        }
    }

    pub async fn list_enabled(&self) -> Result<Vec<CronJob>, StorageError> {
        let conn = self.conn.lock().await;

        let mut rows = conn
            .query(
                "SELECT id, job_id, schedule, task, enabled, last_run, next_run, created_at, metadata
                 FROM cron_jobs WHERE enabled = 1 ORDER BY next_run ASC",
                (),
            )
            .await?;

        let mut jobs = Vec::new();
        while let Some(row) = rows.next().await? {
            let task_str: String = row.get(3)?;
            let metadata_str: Option<String> = row.get(8)?;
            let enabled: i64 = row.get(4)?;

            jobs.push(CronJob {
                id: row.get(0)?,
                job_id: row.get(1)?,
                schedule: row.get(2)?,
                task: serde_json::from_str(&task_str).unwrap_or_default(),
                enabled: enabled != 0,
                last_run: row.get(5)?,
                next_run: row.get(6)?,
                created_at: row.get(7)?,
                metadata: metadata_str.and_then(|s| serde_json::from_str(&s).ok()),
            });
        }

        Ok(jobs)
    }

    pub async fn list_all(&self) -> Result<Vec<CronJob>, StorageError> {
        let conn = self.conn.lock().await;

        let mut rows = conn
            .query(
                "SELECT id, job_id, schedule, task, enabled, last_run, next_run, created_at, metadata
                 FROM cron_jobs ORDER BY created_at DESC",
                (),
            )
            .await?;

        let mut jobs = Vec::new();
        while let Some(row) = rows.next().await? {
            let task_str: String = row.get(3)?;
            let metadata_str: Option<String> = row.get(8)?;
            let enabled: i64 = row.get(4)?;

            jobs.push(CronJob {
                id: row.get(0)?,
                job_id: row.get(1)?,
                schedule: row.get(2)?,
                task: serde_json::from_str(&task_str).unwrap_or_default(),
                enabled: enabled != 0,
                last_run: row.get(5)?,
                next_run: row.get(6)?,
                created_at: row.get(7)?,
                metadata: metadata_str.and_then(|s| serde_json::from_str(&s).ok()),
            });
        }

        Ok(jobs)
    }

    pub async fn update_last_run(&self, job_id: &str, last_run: &str, next_run: Option<&str>) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;

        conn.execute(
            "UPDATE cron_jobs SET last_run = ?1, next_run = ?2 WHERE job_id = ?3",
            turso::params![last_run, next_run, job_id],
        )
        .await?;

        Ok(())
    }

    pub async fn set_enabled(&self, job_id: &str, enabled: bool) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;

        conn.execute(
            "UPDATE cron_jobs SET enabled = ?1 WHERE job_id = ?2",
            turso::params![enabled, job_id],
        )
        .await?;

        Ok(())
    }

    pub async fn delete(&self, job_id: &str) -> Result<bool, StorageError> {
        let conn = self.conn.lock().await;

        let result = conn
            .execute(
                "DELETE FROM cron_jobs WHERE job_id = ?1",
                turso::params![job_id],
            )
            .await?;

        Ok(result > 0)
    }
}

/// Job run history repository
pub struct JobRunRepository {
    conn: Arc<Mutex<Connection>>,
}

impl JobRunRepository {
    pub const fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Record a new job run
    pub async fn create(&self, run: NewJobRun) -> Result<JobRun, StorageError> {
        let conn = self.conn.lock().await;

        conn.execute(
            "INSERT INTO job_runs (job_id, started_at, finished_at, success, output, error, duration_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            turso::params![
                run.job_id.as_str(),
                run.started_at.as_str(),
                run.finished_at.as_deref(),
                run.success,
                run.output.as_deref(),
                run.error.as_deref(),
                run.duration_ms,
            ],
        )
        .await?;

        // Get the inserted run
        let mut rows = conn
            .query(
                "SELECT id, job_id, started_at, finished_at, success, output, error, duration_ms
                 FROM job_runs ORDER BY id DESC LIMIT 1",
                (),
            )
            .await?;

        if let Some(row) = rows.next().await? {
            let success: i64 = row.get(4)?;
            Ok(JobRun {
                id: row.get(0)?,
                job_id: row.get(1)?,
                started_at: row.get(2)?,
                finished_at: row.get(3)?,
                success: success != 0,
                output: row.get(5)?,
                error: row.get(6)?,
                duration_ms: row.get(7)?,
            })
        } else {
            Err(StorageError::NotFound("JobRun just created".to_string()))
        }
    }

    /// Get runs for a specific job
    pub async fn list_by_job(&self, job_id: &str, limit: i64) -> Result<Vec<JobRun>, StorageError> {
        let conn = self.conn.lock().await;

        let mut rows = conn
            .query(
                "SELECT id, job_id, started_at, finished_at, success, output, error, duration_ms
                 FROM job_runs WHERE job_id = ?1 ORDER BY started_at DESC LIMIT ?2",
                turso::params![job_id, limit],
            )
            .await?;

        let mut runs = Vec::new();
        while let Some(row) = rows.next().await? {
            let success: i64 = row.get(4)?;
            runs.push(JobRun {
                id: row.get(0)?,
                job_id: row.get(1)?,
                started_at: row.get(2)?,
                finished_at: row.get(3)?,
                success: success != 0,
                output: row.get(5)?,
                error: row.get(6)?,
                duration_ms: row.get(7)?,
            });
        }

        Ok(runs)
    }

    /// Get recent runs across all jobs
    pub async fn list_recent(&self, limit: i64) -> Result<Vec<JobRun>, StorageError> {
        let conn = self.conn.lock().await;

        let mut rows = conn
            .query(
                "SELECT id, job_id, started_at, finished_at, success, output, error, duration_ms
                 FROM job_runs ORDER BY started_at DESC LIMIT ?1",
                turso::params![limit],
            )
            .await?;

        let mut runs = Vec::new();
        while let Some(row) = rows.next().await? {
            let success: i64 = row.get(4)?;
            runs.push(JobRun {
                id: row.get(0)?,
                job_id: row.get(1)?,
                started_at: row.get(2)?,
                finished_at: row.get(3)?,
                success: success != 0,
                output: row.get(5)?,
                error: row.get(6)?,
                duration_ms: row.get(7)?,
            });
        }

        Ok(runs)
    }

    /// Delete old runs for a job (keep only the most recent N)
    pub async fn cleanup(&self, job_id: &str, keep: i64) -> Result<i64, StorageError> {
        let conn = self.conn.lock().await;

        let result = conn
            .execute(
                "DELETE FROM job_runs WHERE job_id = ?1 AND id NOT IN (
                    SELECT id FROM job_runs WHERE job_id = ?1 ORDER BY started_at DESC LIMIT ?2
                )",
                turso::params![job_id, keep],
            )
            .await?;

        Ok(result as i64)
    }

    /// Delete all runs for a job
    pub async fn delete_by_job(&self, job_id: &str) -> Result<i64, StorageError> {
        let conn = self.conn.lock().await;

        let result = conn
            .execute(
                "DELETE FROM job_runs WHERE job_id = ?1",
                turso::params![job_id],
            )
            .await?;

        Ok(result as i64)
    }
}

// ═══ Workspace Repository ═══

pub struct WorkspaceRepository {
    conn: Arc<Mutex<Connection>>,
}

impl WorkspaceRepository {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// List all registered workspaces
    pub async fn list(&self) -> Result<Vec<WorkspaceRecord>, StorageError> {
        let conn = self.conn.lock().await;
        let mut rows = conn
            .query(
                "SELECT id, name, path, active, created_at, last_accessed FROM workspaces ORDER BY last_accessed DESC",
                turso::params![],
            )
            .await?;

        let mut workspaces = Vec::new();
        while let Some(row) = rows.next().await? {
            workspaces.push(WorkspaceRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                active: row.get::<i64>(3)? != 0,
                created_at: row.get(4)?,
                last_accessed: row.get(5)?,
            });
        }
        Ok(workspaces)
    }

    /// Get a workspace by ID
    pub async fn get(&self, id: &str) -> Result<Option<WorkspaceRecord>, StorageError> {
        let conn = self.conn.lock().await;
        let id = id.to_string();
        let mut rows = conn
            .query(
                "SELECT id, name, path, active, created_at, last_accessed FROM workspaces WHERE id = ?1",
                turso::params![id],
            )
            .await?;

        if let Some(row) = rows.next().await? {
            Ok(Some(WorkspaceRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                active: row.get::<i64>(3)? != 0,
                created_at: row.get(4)?,
                last_accessed: row.get(5)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Insert or update a workspace (upsert by path)
    pub async fn upsert(&self, record: &WorkspaceRecord) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        let id = record.id.clone();
        let name = record.name.clone();
        let path = record.path.clone();
        let active = record.active as i64;
        conn.execute(
            "INSERT INTO workspaces (id, name, path, active, last_accessed)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               path = excluded.path,
               active = excluded.active,
               last_accessed = datetime('now')",
            turso::params![id, name, path, active],
        )
        .await?;
        Ok(())
    }

    /// Remove a workspace by ID
    pub async fn delete(&self, id: &str) -> Result<bool, StorageError> {
        let conn = self.conn.lock().await;
        let id = id.to_string();
        let affected = conn
            .execute(
                "DELETE FROM workspaces WHERE id = ?1",
                turso::params![id],
            )
            .await?;
        Ok(affected > 0)
    }

    /// Clear the active flag on all workspaces
    pub async fn clear_active(&self) -> Result<(), StorageError> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE workspaces SET active = 0",
            turso::params![],
        )
        .await?;
        Ok(())
    }

    /// Set a workspace as active (clears others first)
    pub async fn set_active(&self, id: &str) -> Result<bool, StorageError> {
        let conn = self.conn.lock().await;
        // Clear all
        conn.execute("UPDATE workspaces SET active = 0", turso::params![])
            .await?;
        // Set this one
        let id = id.to_string();
        let affected = conn
            .execute(
                "UPDATE workspaces SET active = 1, last_accessed = datetime('now') WHERE id = ?1",
                turso::params![id],
            )
            .await?;
        Ok(affected > 0)
    }
}
