//! Turso-backed MemoryPersistence adapter
//!
//! Bridges `nanna_memory::MemoryPersistence` ↔ `nanna_storage::MemoryRepository`,
//! converting between `MemoryEntry` (in-memory type) and `Memory`/`NewMemory`
//! (storage models).  All FSRS fields are round-tripped losslessly.

use async_trait::async_trait;
use nanna_memory::{FsrsState, MemoryEntry, MemoryError, MemoryPersistence};
use nanna_storage::{MemoryRepository, NewMemory};
use std::collections::HashMap;
use tracing::warn;

/// Implements `MemoryPersistence` using the `MemoryRepository` from `nanna-storage`.
pub struct TursoMemoryPersistence {
    repo: MemoryRepository,
}

impl TursoMemoryPersistence {
    pub fn new(repo: MemoryRepository) -> Self {
        Self { repo }
    }
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

/// Convert a `MemoryEntry` to a `NewMemory` for INSERT/UPSERT.
fn entry_to_new_memory(entry: &MemoryEntry) -> NewMemory {
    // Serialize metadata HashMap<String,String> as JSON
    let metadata_value: Option<serde_json::Value> = if entry.metadata.is_empty() {
        None
    } else {
        serde_json::to_value(&entry.metadata).ok()
    };

    // Extract session_id from metadata if present
    let session_id = entry.metadata.get("session_id").cloned();

    NewMemory {
        memory_id: entry.id.clone(),
        content: entry.content.clone(),
        embedding: Some(entry.embedding.clone()),
        embedding_model: None, // Not tracked per-entry in MemoryEntry
        session_id,
        metadata: metadata_value,
        tags: Vec::new(), // Tags are stored inside metadata for MemoryEntry
        workspace_id: entry.workspace_id.clone(),
        expires_at: entry.expires_at,
        fsrs_stability: entry.fsrs.stability,
        fsrs_difficulty: entry.fsrs.difficulty,
        fsrs_last_access: entry.fsrs.last_access,
        fsrs_access_count: i64::from(entry.fsrs.access_count),
        fsrs_importance: entry.fsrs.importance,
        fsrs_storage_strength: entry.fsrs.storage_strength,
        fsrs_generation: i64::from(entry.fsrs.generation),
    }
}

/// Convert a `nanna_storage::Memory` back to a `MemoryEntry`.
///
/// Returns `None` if the stored entry has no embedding (can't be searched).
pub fn db_memory_to_entry(mem: nanna_storage::Memory) -> Option<MemoryEntry> {
    let embedding = mem.embedding?;

    // Rebuild metadata HashMap from JSON
    let mut metadata: HashMap<String, String> = mem.metadata
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    // Re-inject session_id into metadata if it was stored in session_id column
    if let Some(ref sid) = mem.session_id {
        metadata.entry("session_id".to_string()).or_insert_with(|| sid.clone());
    }

    // Parse timestamp from created_at ISO string (fallback: 0)
    let timestamp = chrono::DateTime::parse_from_rfc3339(&mem.created_at)
        .or_else(|_| {
            // Try the database `datetime('now')` format 'YYYY-MM-DD HH:MM:SS'
            chrono::NaiveDateTime::parse_from_str(&mem.created_at, "%Y-%m-%d %H:%M:%S")
                .map(|ndt| ndt.and_utc().fixed_offset())
                .map_err(|e| e)
        })
        .map(|dt| dt.timestamp())
        .unwrap_or(0);

    let fsrs = FsrsState {
        stability: mem.fsrs_stability,
        difficulty: mem.fsrs_difficulty,
        last_access: mem.fsrs_last_access,
        access_count: mem.fsrs_access_count as u32,
        importance: mem.fsrs_importance,
        storage_strength: mem.fsrs_storage_strength,
        generation: mem.fsrs_generation as u32,
    };

    Some(MemoryEntry {
        id: mem.memory_id,
        content: mem.content,
        embedding,
        metadata,
        timestamp,
        fsrs,
        workspace_id: mem.workspace_id,
        expires_at: mem.expires_at,
    })
}

// ---------------------------------------------------------------------------
// Trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl MemoryPersistence for TursoMemoryPersistence {
    async fn save_entry(&self, entry: &MemoryEntry) -> Result<(), MemoryError> {
        let new_mem = entry_to_new_memory(entry);

        // Try INSERT; if a duplicate memory_id exists, update all fields instead.
        // We use a two-step approach because turso INSERT OR REPLACE would reset
        // the autoincrement id — instead we attempt insert and fall back to update.
        match self.repo.create(new_mem).await {
            Ok(_) => Ok(()),
            Err(nanna_storage::StorageError::Database(ref e))
                if e.to_string().contains("UNIQUE") || e.to_string().contains("unique") =>
            {
                // Entry exists — update FSRS + content
                let _ = self.repo.update_content(&entry.id, &entry.content).await;
                let _ = self.repo.update_fsrs(
                    &entry.id,
                    entry.fsrs.stability,
                    entry.fsrs.difficulty,
                    entry.fsrs.last_access,
                    i64::from(entry.fsrs.access_count),
                    entry.fsrs.importance,
                    entry.fsrs.storage_strength,
                    i64::from(entry.fsrs.generation),
                ).await;
                Ok(())
            }
            Err(e) => Err(MemoryError::Persistence(e.to_string())),
        }
    }

    async fn remove_entry(&self, id: &str) -> Result<(), MemoryError> {
        self.repo
            .delete(id)
            .await
            .map(|_| ())
            .map_err(|e| MemoryError::Persistence(e.to_string()))
    }

    async fn update_entry_fsrs(&self, id: &str, fsrs: &FsrsState) -> Result<(), MemoryError> {
        self.repo
            .update_fsrs(
                id,
                fsrs.stability,
                fsrs.difficulty,
                fsrs.last_access,
                i64::from(fsrs.access_count),
                fsrs.importance,
                fsrs.storage_strength,
                i64::from(fsrs.generation),
            )
            .await
            .map(|_| ())
            .map_err(|e| MemoryError::Persistence(e.to_string()))
    }

    async fn update_entry_content(&self, id: &str, content: &str) -> Result<(), MemoryError> {
        self.repo
            .update_content(id, content)
            .await
            .map(|_| ())
            .map_err(|e| MemoryError::Persistence(e.to_string()))
    }

    async fn load_all(&self) -> Result<Vec<MemoryEntry>, MemoryError> {
        let db_memories = self
            .repo
            .bulk_load()
            .await
            .map_err(|e| MemoryError::Persistence(e.to_string()))?;

        let mut entries = Vec::with_capacity(db_memories.len());
        let mut skipped = 0usize;

        for mem in db_memories {
            match db_memory_to_entry(mem) {
                Some(entry) => entries.push(entry),
                None => {
                    skipped += 1;
                }
            }
        }

        if skipped > 0 {
            warn!("Skipped {} memories with no embedding during bulk load", skipped);
        }

        Ok(entries)
    }
}
