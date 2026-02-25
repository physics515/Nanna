//! In-memory circular log buffer for recent daemon logs
//!
//! Captures log events via a tracing Layer and makes them available to the GUI.
//! Uses std::sync::Mutex (not tokio) so it works before the Tokio runtime starts.

use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

/// A single log entry
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub target: String,
    pub message: String,
}

/// Circular buffer for recent logs (last N entries)
#[derive(Clone)]
pub struct LogBuffer {
    entries: Arc<Mutex<Vec<LogEntry>>>,
    max_entries: usize,
}

impl LogBuffer {
    /// Create a new log buffer with max capacity
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Arc::new(Mutex::new(Vec::with_capacity(max_entries))),
            max_entries,
        }
    }

    /// Add a log entry
    pub fn push(&self, entry: LogEntry) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.push(entry);

        // Keep only the last N entries
        if entries.len() > self.max_entries {
            let excess = entries.len() - self.max_entries;
            entries.drain(0..excess);
        }
    }

    /// Get all entries
    pub fn get_all(&self) -> Vec<LogEntry> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Get last N entries
    pub fn get_recent(&self, limit: usize) -> Vec<LogEntry> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
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
    pub fn clear(&self) {
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

// =============================================================================
// Tracing Layer for capturing logs into the buffer
// =============================================================================

/// Visitor that extracts the message field from tracing events
#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        }
    }
}

/// Tracing layer that captures events into a LogBuffer
pub struct LogBufferLayer {
    buffer: LogBuffer,
}

impl LogBufferLayer {
    pub fn new(buffer: LogBuffer) -> Self {
        Self { buffer }
    }
}

impl<S: tracing::Subscriber> Layer<S> for LogBufferLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let level = metadata.level().as_str().to_lowercase();
        let target = metadata.target().to_string();

        // Extract message from event fields
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let entry = LogEntry {
            timestamp: chrono::Local::now()
                .format("%Y-%m-%d %H:%M:%S%.3f")
                .to_string(),
            level,
            target,
            message: visitor.message,
        };

        self.buffer.push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_buffer() {
        let buffer = LogBuffer::new(10);

        for i in 0..15 {
            buffer.push(LogEntry {
                timestamp: format!("2024-01-01 12:00:{:02}", i),
                level: "info".to_string(),
                target: "test".to_string(),
                message: format!("Message {}", i),
            });
        }

        let entries = buffer.get_all();
        assert_eq!(entries.len(), 10);
        assert_eq!(entries[0].message, "Message 5");
        assert_eq!(entries[9].message, "Message 14");
    }

    #[test]
    fn test_get_recent() {
        let buffer = LogBuffer::new(100);

        for i in 0..20 {
            buffer.push(LogEntry {
                timestamp: format!("2024-01-01 12:00:{:02}", i),
                level: "info".to_string(),
                target: "test".to_string(),
                message: format!("Message {}", i),
            });
        }

        let recent = buffer.get_recent(5);
        assert_eq!(recent.len(), 5);
        assert_eq!(recent[0].message, "Message 15");
        assert_eq!(recent[4].message, "Message 19");
    }
}
