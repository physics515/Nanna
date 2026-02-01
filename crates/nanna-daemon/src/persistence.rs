//! Session and state persistence
//!
//! Handles saving and loading daemon state to disk.

use crate::session::{Session, SessionId, SessionMessage};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{debug, error, info, warn};

/// Serializable session state for persistence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSession {
    pub id: SessionId,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub messages: Vec<SessionMessage>,
    pub metadata: HashMap<String, serde_json::Value>,
}

impl From<&Session> for PersistedSession {
    fn from(session: &Session) -> Self {
        Self {
            id: session.id.clone(),
            name: session.name.clone(),
            created_at: session.created_at,
            updated_at: session.updated_at,
            messages: session.messages.clone(),
            metadata: session.metadata.clone(),
        }
    }
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
        }
    }
}

/// Complete daemon state for persistence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedState {
    pub version: u32,
    pub saved_at: DateTime<Utc>,
    pub sessions: Vec<PersistedSession>,
    pub default_session_id: Option<SessionId>,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            version: 1,
            saved_at: Utc::now(),
            sessions: Vec::new(),
            default_session_id: None,
        }
    }
}

/// Persistence manager for daemon state
pub struct PersistenceManager {
    data_dir: PathBuf,
}

impl PersistenceManager {
    /// Create a new persistence manager
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }
    
    /// Get the path to the sessions file
    fn sessions_path(&self) -> PathBuf {
        self.data_dir.join("sessions.json")
    }
    
    /// Get the path to the sessions backup file
    fn sessions_backup_path(&self) -> PathBuf {
        self.data_dir.join("sessions.backup.json")
    }
    
    /// Ensure the data directory exists
    pub async fn ensure_dir(&self) -> Result<(), std::io::Error> {
        fs::create_dir_all(&self.data_dir).await
    }
    
    /// Save sessions to disk
    pub async fn save_sessions(&self, sessions: &[Session], default_id: Option<&str>) -> Result<(), String> {
        self.ensure_dir().await.map_err(|e| e.to_string())?;
        
        let state = PersistedState {
            version: 1,
            saved_at: Utc::now(),
            sessions: sessions.iter().map(PersistedSession::from).collect(),
            default_session_id: default_id.map(String::from),
        };
        
        let json = serde_json::to_string_pretty(&state)
            .map_err(|e| format!("Failed to serialize sessions: {}", e))?;
        
        let path = self.sessions_path();
        let backup_path = self.sessions_backup_path();
        
        // Backup existing file
        if path.exists() {
            if let Err(e) = fs::copy(&path, &backup_path).await {
                warn!("Failed to backup sessions file: {}", e);
            }
        }
        
        // Write new file
        fs::write(&path, json).await
            .map_err(|e| format!("Failed to write sessions file: {}", e))?;
        
        debug!("Saved {} sessions to {:?}", sessions.len(), path);
        Ok(())
    }
    
    /// Load sessions from disk
    pub async fn load_sessions(&self) -> Result<(Vec<Session>, Option<SessionId>), String> {
        let path = self.sessions_path();
        
        if !path.exists() {
            debug!("No sessions file found at {:?}", path);
            return Ok((Vec::new(), None));
        }
        
        let json = fs::read_to_string(&path).await
            .map_err(|e| format!("Failed to read sessions file: {}", e))?;
        
        let state: PersistedState = serde_json::from_str(&json)
            .map_err(|e| {
                // Try backup
                warn!("Failed to parse sessions file, trying backup: {}", e);
                e.to_string()
            })?;
        
        let sessions: Vec<Session> = state.sessions.into_iter().map(Session::from).collect();
        
        info!("Loaded {} sessions from {:?}", sessions.len(), path);
        Ok((sessions, state.default_session_id))
    }
    
    /// Try to load from backup if main file fails
    pub async fn load_sessions_with_fallback(&self) -> Result<(Vec<Session>, Option<SessionId>), String> {
        match self.load_sessions().await {
            Ok(result) => Ok(result),
            Err(e) => {
                warn!("Failed to load sessions, trying backup: {}", e);
                
                let backup_path = self.sessions_backup_path();
                if !backup_path.exists() {
                    return Err(format!("No backup file found: {}", e));
                }
                
                let json = fs::read_to_string(&backup_path).await
                    .map_err(|e| format!("Failed to read backup: {}", e))?;
                
                let state: PersistedState = serde_json::from_str(&json)
                    .map_err(|e| format!("Failed to parse backup: {}", e))?;
                
                let sessions: Vec<Session> = state.sessions.into_iter().map(Session::from).collect();
                
                info!("Recovered {} sessions from backup", sessions.len());
                Ok((sessions, state.default_session_id))
            }
        }
    }
    
    /// Auto-save sessions periodically (call this from a background task)
    pub async fn auto_save_loop(
        &self,
        sessions: std::sync::Arc<tokio::sync::RwLock<HashMap<SessionId, Session>>>,
        default_session: std::sync::Arc<tokio::sync::RwLock<Option<SessionId>>>,
        interval: std::time::Duration,
        mut shutdown: tokio::sync::broadcast::Receiver<()>,
    ) {
        let mut interval = tokio::time::interval(interval);
        
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let sessions_guard = sessions.read().await;
                    let sessions_vec: Vec<Session> = sessions_guard.values().cloned().collect();
                    let default_id = default_session.read().await.clone();
                    drop(sessions_guard);
                    
                    if let Err(e) = self.save_sessions(&sessions_vec, default_id.as_deref()).await {
                        error!("Auto-save failed: {}", e);
                    }
                }
                _ = shutdown.recv() => {
                    info!("Auto-save loop shutting down");
                    
                    // Final save on shutdown
                    let sessions_guard = sessions.read().await;
                    let sessions_vec: Vec<Session> = sessions_guard.values().cloned().collect();
                    let default_id = default_session.read().await.clone();
                    drop(sessions_guard);
                    
                    if let Err(e) = self.save_sessions(&sessions_vec, default_id.as_deref()).await {
                        error!("Final save failed: {}", e);
                    } else {
                        info!("Final save completed");
                    }
                    
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    #[tokio::test]
    async fn test_session_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let manager = PersistenceManager::new(temp_dir.path());
        
        // Create test sessions
        let mut session1 = Session::new(Some("Test Session 1".to_string()));
        session1.add_message(crate::session::MessageRole::User, "Hello");
        session1.add_message(crate::session::MessageRole::Assistant, "Hi there!");
        
        let session2 = Session::new(Some("Test Session 2".to_string()));
        
        let sessions = vec![session1.clone(), session2.clone()];
        
        // Save
        manager.save_sessions(&sessions, Some(&session1.id)).await.unwrap();
        
        // Load
        let (loaded, default_id) = manager.load_sessions().await.unwrap();
        
        assert_eq!(loaded.len(), 2);
        assert_eq!(default_id.as_deref(), Some(session1.id.as_str()));
        
        // Verify first session
        let loaded1 = loaded.iter().find(|s| s.id == session1.id).unwrap();
        assert_eq!(loaded1.name, session1.name);
        assert_eq!(loaded1.messages.len(), 2);
    }
}
