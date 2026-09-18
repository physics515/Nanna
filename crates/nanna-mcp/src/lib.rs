#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

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
//! let transport = StdioTransport::spawn("npx", &["-y", "@modelcontextprotocol/server-filesystem", "/tmp"])?;
//! let client = McpClient::new(transport);
//! client.initialize().await?; // probes the era: modern `server/discover` or legacy `initialize`
//!
//! let tools = client.list_tools().await?;
//! let result = client.call_tool("read_file", json!({"path": "/tmp/test.txt"})).await?;
//! ```

mod adapter;
mod client;
pub mod elicit;
pub mod era;
mod protocol;
mod schema_guard;
mod server;
#[cfg(feature = "http")]
pub mod streamable_http;
mod transport;

pub use adapter::*;
pub use client::{McpClient, McpClientBuilder};
pub use elicit::Elicitor;
pub use era::ProtocolEra;
pub use protocol::*;
pub use schema_guard::{
    MCP_SCHEMA_DEPTH_MAX, MCP_SCHEMA_NODES_MAX, SchemaViolation, validate_tool_schema,
};
pub use server::{McpServer, McpServerBuilder, McpServerConfig, ResourceHandler, ToolHandler};
/// Bridge that publishes a `nanna-tools` registry as MCP tools — the seam
/// `nanna mcp serve` uses to expose the local tool surface.
#[cfg(feature = "tools-integration")]
pub use server::tools_bridge;
pub use transport::*;
#[cfg(feature = "http")]
pub use streamable_http::StreamableHttpTransport;

/// The legacy (handshake) MCP revision; see [`era`] for the modern ones.
pub const PROTOCOL_VERSION: &str = era::LEGACY_PROTOCOL_VERSION;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum McpError {
    #[error("Transport error: {0}")]
    Transport(String),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("JSON-RPC error {code}: {message}")]
    JsonRpc {
        code: i32,
        message: String,
        data: Option<serde_json::Value>,
    },

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Timeout waiting for response")]
    Timeout,

    /// An HTTP answer that carried no JSON-RPC message (a proxy's page, an
    /// auth refusal, a legacy server's bare 404).
    #[error("HTTP {status}: {body}")]
    HttpStatus { status: u16, body: String },

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
