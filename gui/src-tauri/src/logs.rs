//! Daemon logging - retrieves recent logs from daemon output

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub target: String,
    pub message: String,
}

/// In-memory log buffer (shared across app state)
#[derive(Clone)]
pub struct LogBuffer {
    entries: Arc<RwLock<Vec<LogEntry>>>,
    max_entries: usize,
}

impl LogBuffer {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Arc::new(RwLock::new(Vec::new())),
            max_entries,
        }
    }

    /// Add a log entry
    pub async fn push(&self, entry: LogEntry) {
        let mut entries = self.entries.write().await;
        entries.push(entry);
        
        // Keep only the most recent entries
        if entries.len() > self.max_entries {
            entries.drain(0..entries.len() - self.max_entries);
        }
    }

    /// Get recent logs
    pub async fn get_recent(&self, limit: usize) -> Vec<LogEntry> {
        let entries = self.entries.read().await;
        entries.iter().rev().take(limit).cloned().collect::<Vec<_>>().into_iter().rev().collect()
    }

    /// Clear all logs
    pub async fn clear(&self) {
        self.entries.write().await.clear();
    }
}

/// Tauri command to get recent daemon logs
#[tauri::command]
pub async fn get_daemon_logs(limit: Option<usize>) -> Result<Vec<LogEntry>, String> {
    // For now, return empty vector
    // In a real implementation, this would query the daemon via WebSocket
    // or read from a log file that the daemon writes to
    debug!("get_daemon_logs called with limit: {:?}", limit);
    Ok(vec![])
}
