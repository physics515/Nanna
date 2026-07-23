#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! Tool system for Nanna
//!
//! Provides a framework for defining and executing tools that the LLM can invoke.
//!
//! # User-Authored Skills
//!
//! The `skills` module supports two tiers of tool authoring:
//! - **Scripted (Boa/Deno)**: JS/TS tools with sandboxing (requires `scripting` feature)
//! - **Executable (Manifest)**: Python/shell/binary via `tool.yaml`
//!
//! See [`skills`] module for details.

mod builtin;
mod output;
mod policy;
mod registry;
mod schema;
pub mod search;
pub mod skills;

pub use builtin::*;
pub use output::{format_tool_output, schemas as output_schemas, wants_json_output};
pub use policy::{DenyReason, ToolPolicy};
pub use registry::ToolRegistry;
pub use search::{SearchDoc, ToolSearchHit};
pub use schema::{ParameterType, ToolDefinition, ToolParameter, ToolResult};
pub use skills::{DiscoveredSkill, SkillSource, discover_skills, load_skill, load_skills_from_dir};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;

/// Where a tool's output should be routed after execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OutputTarget {
    /// Large results are chunked and stored in memory; a stub replaces them in context (default).
    #[default]
    Memory,
    /// Results always stay in context. Large results are summarized (or truncated as fallback).
    Context,
}

#[derive(Error, Debug)]
pub enum ToolError {
    #[error("Tool not found: {0}")]
    NotFound(String),
    #[error("Invalid parameters: {0}")]
    InvalidParams(String),
    #[error("Execution failed: {0}")]
    ExecutionFailed(String),
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    #[error("Timeout")]
    Timeout,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Tool invocation request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub parameters: HashMap<String, Value>,
}

/// Tool invocation response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResponse {
    pub id: String,
    pub name: String,
    pub result: ToolResult,
    /// Where this tool's output should be routed
    #[serde(default)]
    pub output_target: OutputTarget,
}

/// Trait for implementing tools
#[async_trait]
pub trait Tool: Send + Sync {
    /// Get the tool definition (name, description, parameters)
    fn definition(&self) -> ToolDefinition;

    /// Execute the tool with the given parameters
    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError>;

    /// Where tool output should be routed (memory stub vs inline context)
    fn output_target(&self) -> OutputTarget {
        OutputTarget::Memory
    }

    /// Check if the tool requires elevated permissions
    fn requires_elevation(&self) -> bool {
        false
    }

    /// Get the tool's timeout in seconds (None = no timeout)
    fn timeout_secs(&self) -> Option<u64> {
        Some(30)
    }
}

/// Result from spawning a sub-agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnResult {
    /// Final text response from the sub-agent
    pub text: String,
    /// Number of iterations the sub-agent used
    pub iterations: usize,
    /// Number of tool calls made
    pub tool_calls: usize,
    /// Input tokens consumed
    pub input_tokens: u32,
    /// Output tokens consumed
    pub output_tokens: u32,
    /// Model used for the sub-agent
    pub model: String,
}

/// Trait for spawning sub-agents. Implemented in the daemon where Agent is available.
/// This follows the same adapter pattern as `MemoryServiceAdapter`.
#[async_trait]
pub trait AgentSpawner: Send + Sync {
    /// Spawn a sub-agent with the given prompt and constraints.
    /// `max_iterations`: None = unlimited (agent stops when done).
    async fn spawn(
        &self,
        prompt: &str,
        description: &str,
        max_iterations: Option<usize>,
    ) -> Result<SpawnResult, String>;
}

/// Trait for sub-agent ↔ parent communication. Implemented in the daemon.
/// Allows sub-agents to ask questions to their parent and receive answers.
#[async_trait]
pub trait ParentChannel: Send + Sync {
    /// Send a question to the parent agent and wait for a reply.
    /// Returns the parent's response, or an error on timeout/failure.
    async fn ask_parent(
        &self,
        sub_session_id: &str,
        question: &str,
        timeout_secs: u64,
    ) -> Result<String, String>;
}

/// Helper for creating tool results
impl ToolResult {
    pub fn success(content: impl Into<String>) -> Self {
        Self {
            success: true,
            content: content.into(),
            error: None,
            data: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            success: false,
            content: String::new(),
            error: Some(message.into()),
            data: None,
        }
    }

    #[must_use]
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}
