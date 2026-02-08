//! In-memory circular log buffer for recent daemon logs
//!
//! Captures info/error level logs and makes them available to the GUI.

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing_subscriber::fmt::MakeWriter;

/// A single log entry
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

/// Circular buffer for recent logs (last N entries)
#[derive(Clone)]
pub struct LogBuffer {
    entries: Arc<RwLock<Vec<LogEntry>>>,
    max_entries: usize,
}

impl LogBuffer {
    /// Create a new log buffer with max capacity
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Arc::new(RwLock::new(Vec::with_capacity(max_entries))),
            max_entries,
        }
    }

    /// Add a log entry
    pub async fn push(&self, entry: LogEntry) {
        let mut entries = self.entries.write().await;
        entries.push(entry);

        // Keep only the last N entries
        if entries.len() > self.max_entries {
            let excess = entries.len() - self.max_entries;
            entries.drain(0..excess);
        }
    }

    /// Get all entries
    pub async fn get_all(&self) -> Vec<LogEntry> {
        self.entries.read().await.clone()
    }

    /// Get last N entries
    pub async fn get_recent(&self, limit: usize) -> Vec<LogEntry> {
        let entries = self.entries.read().await;
        entries
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    /// Clear all entries
    pub async fn clear(&self) {
        self.entries.write().await.clear();
    }
}

/// Writer that captures logs to the buffer
pub struct LogBufferWriter {
    buffer: LogBuffer,
    level: String,
}

impl LogBufferWriter {
    pub fn new(buffer: LogBuffer, level: String) -> Self {
        Self { buffer, level }
    }
}

impl<'a> MakeWriter<'a> for LogBufferWriter {
    type Writer = LogBufferWriterInner;

    fn make_writer(&'a self) -> Self::Writer {
        LogBufferWriterInner {
            buffer: self.buffer.clone(),
            level: self.level.clone(),
            message: Vec::new(),
        }
    }
}

pub struct LogBufferWriterInner {
    buffer: LogBuffer,
    level: String,
    message: Vec<u8>,
}

impl std::io::Write for LogBufferWriterInner {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.message.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if !self.message.is_empty() {
            let msg = String::from_utf8_lossy(&self.message).to_string();
            let entry = LogEntry {
                timestamp: chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
                level: self.level.clone(),
                message: msg.trim().to_string(),
            };

            // Spawn async task to push to buffer
            let buffer = self.buffer.clone();
            tokio::spawn(async move {
                buffer.push(entry).await;
            });

            self.message.clear();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_log_buffer() {
        let buffer = LogBuffer::new(10);

        for i in 0..15 {
            buffer
                .push(LogEntry {
                    timestamp: format!("2024-01-01 12:00:{:02}", i),
                    level: "info".to_string(),
                    message: format!("Message {}", i),
                })
                .await;
        }

        let entries = buffer.get_all().await;
        assert_eq!(entries.len(), 10);
        assert_eq!(entries[0].message, "Message 5");
        assert_eq!(entries[9].message, "Message 14");
    }

    #[tokio::test]
    async fn test_get_recent() {
        let buffer = LogBuffer::new(100);

        for i in 0..20 {
            buffer
                .push(LogEntry {
                    timestamp: format!("2024-01-01 12:00:{:02}", i),
                    level: "info".to_string(),
                    message: format!("Message {}", i),
                })
                .await;
        }

        let recent = buffer.get_recent(5).await;
        assert_eq!(recent.len(), 5);
        assert_eq!(recent[0].message, "Message 15");
        assert_eq!(recent[4].message, "Message 19");
    }
}
