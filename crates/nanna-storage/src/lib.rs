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
use tokio::sync::RwLock;
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
    conn: Arc<RwLock<Connection>>,
}

impl Storage {
    /// Create a new storage instance
    pub async fn new(config: &StorageConfig) -> Result<Self, StorageError> {
        info!("Opening database: {}", config.path);
        let db = Builder::new_local(&config.path).build().await?;
        let conn = db.connect()?;
        let storage = Self {
            conn: Arc::new(RwLock::new(conn)),
        };

        storage.migrate().await?;
        Ok(storage)
    }

    /// Create an in-memory storage (for testing)
    pub async fn in_memory() -> Result<Self, StorageError> {
        let db = Builder::new_local(":memory:").build().await?;
        let conn = db.connect()?;
        let storage = Self {
            conn: Arc::new(RwLock::new(conn)),
        };
        storage.migrate().await?;
        Ok(storage)
    }

    /// Run database migrations
    async fn migrate(&self) -> Result<(), StorageError> {
        let conn = self.conn.write().await;

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
    pub const fn conn(&self) -> &Arc<RwLock<Connection>> {
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
