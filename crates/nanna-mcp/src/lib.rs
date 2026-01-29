#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]
#![allow(clippy::module_name_repetitions)]

//! MCP (Model Context Protocol) client for Nanna
//!
//! Implements the client side of Anthropic's Model Context Protocol,
//! enabling connection to external tool servers via stdio or HTTP.
//!
//! # Features
//!
//! - `stdio` (default): Connect to MCP servers via stdin/stdout
//! - `http`: Connect to MCP servers via HTTP/SSE
//!
//! # Example
//!
//! ```ignore
//! use nanna_mcp::{McpClient, StdioTransport};
//!
//! let transport = StdioTransport::spawn("npx", &["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]).await?;
//! let client = McpClient::new(transport);
//! client.initialize().await?;
//!
//! let tools = client.list_tools().await?;
//! let result = client.call_tool("read_file", json!({"path": "/tmp/test.txt"})).await?;
//! ```

mod protocol;
mod transport;
mod client;
mod adapter;
mod server;

pub use protocol::*;
pub use transport::*;
pub use client::{McpClient, McpClientBuilder};
pub use adapter::*;
pub use server::{McpServer, McpServerBuilder, McpServerConfig, ToolHandler, ResourceHandler};

/// MCP protocol version
pub const PROTOCOL_VERSION: &str = "2024-11-05";

use thiserror::Error;

#[derive(Error, Debug)]
pub enum McpError {
    #[error("Transport error: {0}")]
    Transport(String),
    
    #[error("Protocol error: {0}")]
    Protocol(String),
    
    #[error("JSON-RPC error {code}: {message}")]
    JsonRpc { code: i32, message: String, data: Option<serde_json::Value> },
    
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    
    #[error("Connection closed")]
    ConnectionClosed,
    
    #[error("Timeout waiting for response")]
    Timeout,
    
    #[error("Server not initialized")]
    NotInitialized,
    
    #[error("Tool not found: {0}")]
    ToolNotFound(String),
    
    #[error("Resource not found: {0}")]
    ResourceNotFound(String),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, McpError>;
