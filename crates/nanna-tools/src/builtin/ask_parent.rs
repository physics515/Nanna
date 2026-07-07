//! Ask-parent tool — lets sub-agents ask their parent agent questions
//!
//! Uses the `ParentChannel` trait abstraction so it knows nothing about session management.
//! The concrete implementation lives in `nanna-daemon/src/server.rs`.
//!
//! The tool reads the current session ID from the shared ToolRegistry at execution time,
//! so a single instance can be registered globally and work for any sub-agent.

use crate::{ParentChannel, Tool, ToolDefinition, ToolError, ToolRegistry, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Default timeout for waiting on parent response (2 minutes)
const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Tool that allows a sub-agent to ask its parent agent a question and wait for a reply.
///
/// The sub-agent's execution pauses while waiting. The parent receives the question
/// as an event, processes it, and sends a reply back through the mailbox system.
///
/// Reads the current session ID from the ToolRegistry at execution time so it works
/// correctly even though the registry is shared across sessions.
pub struct AskParentTool {
    channel: Arc<dyn ParentChannel>,
    registry: Arc<ToolRegistry>,
}

impl AskParentTool {
    pub fn new(channel: Arc<dyn ParentChannel>, registry: Arc<ToolRegistry>) -> Self {
        Self { channel, registry }
    }
}

#[async_trait]
impl Tool for AskParentTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "ask_parent",
            "Ask the parent agent a question and wait for a reply. Use this when you need \
             clarification, additional context, approval, or information that only the parent \
             agent (or the human) would have. Your execution pauses until the parent responds."
        )
        .string_param("question", "The question or request for the parent agent.", true)
        .int_param("timeout_secs", "How long to wait for a reply (default 120s).", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let question = params.get("question")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidParams("'question' parameter is required".to_string()))?;

        let timeout = params.get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_SECS);

        // Read current session ID from the registry (set by the daemon before each run)
        let session_id = self.registry.session_id().await
            .ok_or_else(|| ToolError::ExecutionFailed(
                "ask_parent is only available to sub-agents (no session ID set)".to_string()
            ))?;

        match self.channel.ask_parent(&session_id, question, timeout).await {
            Ok(reply) => Ok(ToolResult::success(reply)),
            Err(e) => Ok(ToolResult::error(format!("Failed to get parent response: {e}"))),
        }
    }

    // No timeout on the tool itself — the ask_parent call has its own internal timeout
    fn timeout_secs(&self) -> Option<u64> {
        None
    }
}
