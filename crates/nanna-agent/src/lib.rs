#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! Agent system for Nanna
//!
//! Implements the agentic loop with tool calling, memory, and context management.

mod context;
mod loop_runner;
mod prompts;

pub use context::AgentContext;
pub use loop_runner::{Agent, AgentConfig, AgentResponse, RunOptions, StreamCallback, ToolCallRecord};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AgentError {
    #[error("LLM error: {0}")]
    Llm(#[from] nanna_llm::LlmError),
    #[error("Tool error: {0}")]
    Tool(#[from] nanna_tools::ToolError),
    #[error("Max iterations exceeded")]
    MaxIterations,
    #[error("Context too long")]
    ContextTooLong,
    #[error("Agent stopped")]
    Stopped,
}

/// Message content types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentContent {
    Text { text: String },
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { tool_use_id: String, content: String, is_error: bool },
}

impl AgentContent {
    pub fn text(content: impl Into<String>) -> Self {
        Self::Text { text: content.into() }
    }
}
