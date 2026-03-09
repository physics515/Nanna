//! Task delegation tool - spawns sub-agents for independent sub-tasks
//!
//! Uses the `AgentSpawner` trait abstraction so it knows nothing about `nanna-agent`.
//! The concrete implementation lives in `nanna-daemon/src/server.rs`.

use crate::{AgentSpawner, Tool, ToolDefinition, ToolError, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Maximum iterations a sub-agent can run
const MAX_SUB_AGENT_ITERATIONS: usize = 25;
/// Default iterations if not specified
const DEFAULT_SUB_AGENT_ITERATIONS: usize = 10;
/// Timeout for sub-agent execution (5 minutes)
const SUB_AGENT_TIMEOUT_SECS: u64 = 300;

/// Tool for delegating independent sub-tasks to a fresh sub-agent.
///
/// The sub-agent gets its own isolated context (system prompt + workspace only),
/// full tool access (including recursive `task` calls), and only returns its
/// final text response plus usage metadata.
pub struct TaskTool {
    spawner: Arc<dyn AgentSpawner>,
}

impl TaskTool {
    pub fn new(spawner: Arc<dyn AgentSpawner>) -> Self {
        Self { spawner }
    }
}

#[async_trait]
impl Tool for TaskTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "task",
            "Delegate an independent sub-task to a fresh sub-agent. The sub-agent gets its own \
             context, full tool access, and returns a summary. Use this for: reading/analyzing \
             files that don't need to stay in your context, independent research tasks, any work \
             that can be done in isolation. The sub-agent cannot see your conversation history."
        )
        .string_param("prompt", "The task to perform. Be specific and self-contained.", true)
        .string_param("description", "Short description (3-5 words) for logging.", false)
        .int_param("max_iterations", "Maximum iterations (1-25, default 10).", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let prompt = params.get("prompt")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidParams("'prompt' parameter is required".to_string()))?;

        let description = params.get("description")
            .and_then(Value::as_str)
            .unwrap_or("sub-task");

        let max_iterations = params.get("max_iterations")
            .and_then(Value::as_u64)
            .map_or(DEFAULT_SUB_AGENT_ITERATIONS, |v| {
                (v as usize).min(MAX_SUB_AGENT_ITERATIONS).max(1)
            });

        match self.spawner.spawn(prompt, description, max_iterations).await {
            Ok(result) => {
                let metadata = format!(
                    "\n\n[Sub-agent used {} iterations, {} tool calls, {} input + {} output tokens]",
                    result.iterations, result.tool_calls, result.input_tokens, result.output_tokens
                );
                Ok(ToolResult::success(format!("{}{}", result.text, metadata)))
            }
            Err(e) => Ok(ToolResult::error(format!("Sub-agent failed: {e}"))),
        }
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(SUB_AGENT_TIMEOUT_SECS)
    }
}
