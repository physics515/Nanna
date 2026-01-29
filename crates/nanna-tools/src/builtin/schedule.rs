//! Scheduling tools for autonomous behavior

use crate::{Tool, ToolDefinition, ToolError, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::info;

/// Scheduled reminder/task
#[derive(Debug, Clone)]
pub struct ScheduledReminder {
    pub id: String,
    pub message: String,
    pub delay_secs: u64,
    pub created_at: i64,
    pub triggered: bool,
}

/// Shared scheduler state
pub type SchedulerState = Arc<RwLock<ReminderStore>>;

/// Simple reminder store
#[derive(Default)]
pub struct ReminderStore {
    reminders: Vec<ScheduledReminder>,
}

impl ReminderStore {
    pub fn add(&mut self, message: String, delay_secs: u64) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.reminders.push(ScheduledReminder {
            id: id.clone(),
            message,
            delay_secs,
            created_at: chrono_timestamp(),
            triggered: false,
        });
        id
    }

    pub fn list(&self) -> Vec<ScheduledReminder> {
        self.reminders.iter().filter(|r| !r.triggered).cloned().collect()
    }

    pub fn cancel(&mut self, id: &str) -> bool {
        if let Some(r) = self.reminders.iter_mut().find(|r| r.id == id) {
            r.triggered = true;
            true
        } else {
            false
        }
    }

    pub fn get_due(&mut self) -> Vec<ScheduledReminder> {
        let now = chrono_timestamp();
        let mut due = Vec::new();

        for reminder in &mut self.reminders {
            if !reminder.triggered {
                let trigger_at = reminder.created_at + reminder.delay_secs as i64;
                if now >= trigger_at {
                    reminder.triggered = true;
                    due.push(reminder.clone());
                }
            }
        }

        due
    }
}

fn chrono_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Tool to set a reminder
pub struct RemindTool {
    state: SchedulerState,
}

impl RemindTool {
    pub fn new(state: SchedulerState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Tool for RemindTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "remind",
            "Set a reminder for later. The reminder will trigger after the specified delay.",
        )
        .string_param("message", "What to remind about", true)
        .integer_param("minutes", "Minutes from now (default: 5)", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let message = params
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("message is required".to_string()))?;

        let minutes = params
            .get("minutes")
            .and_then(|v| v.as_u64())
            .unwrap_or(5);

        let delay_secs = minutes * 60;

        let mut state = self.state.write().await;
        let id = state.add(message.to_string(), delay_secs);

        info!("Reminder set: '{}' in {} minutes (id: {})", message, minutes, &id[..8]);

        Ok(ToolResult::success(format!(
            "Reminder set for {} minutes from now: {}",
            minutes, message
        )))
    }
}

/// Tool to list pending reminders
pub struct ListRemindersTool {
    state: SchedulerState,
}

impl ListRemindersTool {
    pub fn new(state: SchedulerState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Tool for ListRemindersTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "list_reminders",
            "List all pending reminders.",
        )
    }

    async fn execute(&self, _params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let state = self.state.read().await;
        let reminders = state.list();

        if reminders.is_empty() {
            Ok(ToolResult::success("No pending reminders."))
        } else {
            let now = chrono_timestamp();
            let output = reminders
                .iter()
                .map(|r| {
                    let trigger_at = r.created_at + r.delay_secs as i64;
                    let remaining = (trigger_at - now).max(0);
                    format!("[{}] in {}m: {}", &r.id[..8], remaining / 60, r.message)
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(ToolResult::success(output))
        }
    }
}

/// Tool to cancel a reminder
pub struct CancelReminderTool {
    state: SchedulerState,
}

impl CancelReminderTool {
    pub fn new(state: SchedulerState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Tool for CancelReminderTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "cancel_reminder",
            "Cancel a pending reminder by ID.",
        )
        .string_param("id", "Reminder ID (first 8 chars is enough)", true)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let id_prefix = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("id is required".to_string()))?;

        let mut state = self.state.write().await;
        
        // Find by prefix
        let full_id = state.reminders
            .iter()
            .find(|r| r.id.starts_with(id_prefix))
            .map(|r| r.id.clone());

        if let Some(id) = full_id {
            if state.cancel(&id) {
                Ok(ToolResult::success(format!("Reminder {} cancelled.", &id[..8])))
            } else {
                Ok(ToolResult::success("Reminder not found."))
            }
        } else {
            Ok(ToolResult::success("Reminder not found."))
        }
    }
}
