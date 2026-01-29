#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! Tool system for Nanna
//!
//! Provides a framework for defining and executing tools that the LLM can invoke.

mod builtin;
mod registry;
mod schema;

pub use builtin::*;
pub use registry::ToolRegistry;
pub use schema::{ToolDefinition, ToolParameter, ToolResult};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;

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
}

/// Trait for implementing tools
#[async_trait]
pub trait Tool: Send + Sync {
    /// Get the tool definition (name, description, parameters)
    fn definition(&self) -> ToolDefinition;

    /// Execute the tool with the given parameters
    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError>;

    /// Check if the tool requires elevated permissions
    fn requires_elevation(&self) -> bool {
        false
    }

    /// Get the tool's timeout in seconds (None = no timeout)
    fn timeout_secs(&self) -> Option<u64> {
        Some(30)
    }
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
