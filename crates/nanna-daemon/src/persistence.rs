//! Legacy persistence helpers
//!
//! Session and workspace persistence has been migrated to SQLite (nanna-storage).
//! This module is kept for backward compatibility during the transition period
//! — it can migrate old sessions.json data to the database on first run.

use crate::session::Session;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::fs;
use tracing::{info, warn};
use chrono::{DateTime, Utc};

/// Serializable session state (legacy JSON format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSession {
    pub id: String,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub messages: Vec<crate::session::SessionMessage>,
    pub metadata: HashMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
}

impl From<PersistedSession> for Session {
    fn from(persisted: PersistedSession) -> Self {
        Session {
            id: persisted.id,
            name: persisted.name,
            created_at: persisted.created_at,
            updated_at: persisted.updated_at,
            messages: persisted.messages,
            subscribers: Default::default(),
            owner: None,
            metadata: persisted.metadata,
            workspace_id: persisted.workspace_id,
        }
    }
}

/// Legacy persisted state format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedState {
    pub version: u32,
    pub saved_at: DateTime<Utc>,
    pub sessions: Vec<PersistedSession>,
    pub default_session_id: Option<String>,
}

/// Persistence manager — now only handles one-time migration from JSON to DB.
pub struct PersistenceManager {
    data_dir: PathBuf,
}

impl PersistenceManager {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    /// Attempt to load sessions from the legacy sessions.json file.
    /// Returns the sessions if the file exists and is valid, or None otherwise.
    /// After a successful migration, the file should be renamed to .migrated.
    pub async fn load_legacy_sessions(&self) -> Option<(Vec<Session>, Option<String>)> {
        let path = self.data_dir.join("sessions.json");
        if !path.exists() {
            return None;
        }

        let json = match fs::read_to_string(&path).await {
            Ok(j) => j,
            Err(e) => {
                warn!("Failed to read legacy sessions.json: {}", e);
                return None;
            }
        };

        let state: PersistedState = match serde_json::from_str(&json) {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to parse legacy sessions.json: {}", e);
                // Try backup
                let backup_path = self.data_dir.join("sessions.backup.json");
                if backup_path.exists() {
                    if let Ok(backup_json) = fs::read_to_string(&backup_path).await {
                        match serde_json::from_str::<PersistedState>(&backup_json) {
                            Ok(s) => {
                                info!("Recovered {} sessions from backup", s.sessions.len());
                                s
                            }
                            Err(_) => return None,
                        }
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            }
        };

        if state.sessions.is_empty() {
            return None;
        }

        let sessions: Vec<Session> = state.sessions.into_iter().map(Session::from).collect();
        info!("Found {} sessions in legacy sessions.json for migration", sessions.len());
        Some((sessions, state.default_session_id))
    }

    /// Mark the legacy sessions.json as migrated (rename it).
    pub async fn mark_sessions_migrated(&self) {
        let path = self.data_dir.join("sessions.json");
        let migrated_path = self.data_dir.join("sessions.json.migrated");
        if path.exists() {
            if let Err(e) = fs::rename(&path, &migrated_path).await {
                warn!("Failed to rename sessions.json to .migrated: {}", e);
            } else {
                info!("Renamed sessions.json → sessions.json.migrated");
            }
        }
        // Also rename backup
        let backup = self.data_dir.join("sessions.backup.json");
        if backup.exists() {
            let _ = fs::rename(&backup, self.data_dir.join("sessions.backup.json.migrated")).await;
        }
    }

    /// Load legacy tool stats from tool-stats.json (one-time migration).
    pub async fn load_legacy_tool_stats(&self) -> Option<serde_json::Value> {
        let path = self.data_dir.join("tool-stats.json");
        if !path.exists() {
            return None;
        }
        let json = fs::read_to_string(&path).await.ok()?;
        let data: serde_json::Value = serde_json::from_str(&json).ok()?;
        info!("Found legacy tool-stats.json for migration");
        Some(data)
    }

    /// Mark the legacy tool-stats.json as migrated.
    pub async fn mark_tool_stats_migrated(&self) {
        let path = self.data_dir.join("tool-stats.json");
        let migrated = self.data_dir.join("tool-stats.json.migrated");
        if path.exists() {
            if let Err(e) = fs::rename(&path, &migrated).await {
                warn!("Failed to rename tool-stats.json to .migrated: {}", e);
            } else {
                info!("Renamed tool-stats.json → tool-stats.json.migrated");
            }
        }
    }
}
