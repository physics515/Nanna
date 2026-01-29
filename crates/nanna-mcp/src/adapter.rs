//! Adapter for integrating MCP tools with nanna-tools
//!
//! Provides a bridge between MCP tool servers and the nanna-tools registry,
//! allowing MCP tools to be used seamlessly in the agent loop.

use crate::{McpClient, McpError, Tool as McpTool, ToolContent};
use crate::transport::Transport;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, warn};

/// Wrapper that adapts an MCP tool to the nanna-tools interface
pub struct McpToolAdapter<T: Transport + 'static> {
    /// The MCP client
    client: Arc<McpClient<T>>,
    /// The tool definition from MCP
    tool: McpTool,
}

impl<T: Transport + 'static> McpToolAdapter<T> {
    /// Create a new adapter for an MCP tool
    pub fn new(client: Arc<McpClient<T>>, tool: McpTool) -> Self {
        Self { client, tool }
    }

    /// Get the tool name
    #[must_use]
    pub fn name(&self) -> &str {
        &self.tool.name
    }

    /// Get the tool description
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.tool.description.as_deref()
    }

    /// Get the input schema
    #[must_use]
    pub fn input_schema(&self) -> &serde_json::Value {
        &self.tool.input_schema
    }

    /// Execute the tool
    ///
    /// # Errors
    ///
    /// Returns error if tool execution fails
    pub async fn execute(
        &self,
        arguments: Option<serde_json::Value>,
    ) -> Result<McpToolResult, McpError> {
        debug!(tool = %self.tool.name, "Executing MCP tool");

        let result = self.client.call_tool(&self.tool.name, arguments).await?;

        // Convert MCP content to string result
        let content = result
            .content
            .iter()
            .filter_map(|c| match c {
                ToolContent::Text { text } => Some(text.clone()),
                ToolContent::Image { data, mime_type } => {
                    Some(format!("[Image: {mime_type}, {} bytes]", data.len()))
                }
                ToolContent::Resource { resource } => {
                    resource.text.clone().or_else(|| {
                        resource.blob.as_ref().map(|b| format!("[Blob: {} bytes]", b.len()))
                    })
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(McpToolResult {
            content,
            is_error: result.is_error,
            raw: result.content,
        })
    }
}

/// Result from executing an MCP tool
#[derive(Debug, Clone)]
pub struct McpToolResult {
    /// Text content from the tool
    pub content: String,
    /// Whether the tool reported an error
    pub is_error: bool,
    /// Raw content blocks from MCP
    pub raw: Vec<ToolContent>,
}

/// Manager for multiple MCP server connections
pub struct McpManager<T: Transport + 'static> {
    /// Connected MCP clients by server name
    clients: HashMap<String, Arc<McpClient<T>>>,
    /// All available tools across all servers
    tools: HashMap<String, McpToolAdapter<T>>,
}

impl<T: Transport + 'static> McpManager<T> {
    /// Create a new MCP manager
    #[must_use]
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
            tools: HashMap::new(),
        }
    }

    /// Register an MCP client
    ///
    /// # Errors
    ///
    /// Returns error if client is not initialized
    pub async fn register(
        &mut self,
        name: impl Into<String>,
        client: McpClient<T>,
    ) -> Result<(), McpError> {
        let name = name.into();
        let client = Arc::new(client);

        // Get tools from the server
        let tools = client.list_tools().await?;
        debug!(server = %name, count = tools.len(), "Registered MCP server tools");

        for tool in tools {
            let tool_name = format!("{}:{}", name, tool.name);
            let adapter = McpToolAdapter::new(client.clone(), tool);
            self.tools.insert(tool_name, adapter);
        }

        self.clients.insert(name, client);
        Ok(())
    }

    /// Get all available tools
    #[must_use]
    pub fn tools(&self) -> impl Iterator<Item = (&str, &McpToolAdapter<T>)> {
        self.tools.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Get a specific tool by name
    #[must_use]
    pub fn get_tool(&self, name: &str) -> Option<&McpToolAdapter<T>> {
        self.tools.get(name)
    }

    /// Execute a tool by name
    ///
    /// # Errors
    ///
    /// Returns error if tool not found or execution fails
    pub async fn execute(
        &self,
        name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<McpToolResult, McpError> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| McpError::ToolNotFound(name.to_string()))?;

        tool.execute(arguments).await
    }

    /// Refresh tools from all servers
    ///
    /// # Errors
    ///
    /// Returns error if refresh fails for any server
    pub async fn refresh(&mut self) -> Result<(), McpError> {
        self.tools.clear();

        for (name, client) in &self.clients {
            match client.refresh_tools().await {
                Ok(tools) => {
                    for tool in tools {
                        let tool_name = format!("{name}:{}", tool.name);
                        let adapter = McpToolAdapter::new(client.clone(), tool);
                        self.tools.insert(tool_name, adapter);
                    }
                }
                Err(e) => {
                    warn!(server = %name, error = %e, "Failed to refresh tools");
                }
            }
        }

        Ok(())
    }

    /// Close all connections
    ///
    /// # Errors
    ///
    /// Returns error if any close fails
    pub async fn close_all(&self) -> Result<(), McpError> {
        for client in self.clients.values() {
            client.close().await?;
        }
        Ok(())
    }
}

impl<T: Transport + 'static> Default for McpManager<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert an MCP tool to Anthropic tool format for LLM function calling
#[must_use]
pub fn to_anthropic_format(tool: &McpTool) -> serde_json::Value {
    serde_json::json!({
        "name": tool.name,
        "description": tool.description.as_deref().unwrap_or(""),
        "input_schema": tool.input_schema
    })
}

/// Convert an MCP tool to OpenAI tool format
#[must_use]
pub fn to_openai_format(tool: &McpTool) -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description.as_deref().unwrap_or(""),
            "parameters": tool.input_schema
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_format_conversion() {
        let tool = McpTool {
            name: "read_file".to_string(),
            description: Some("Read a file from disk".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file"
                    }
                },
                "required": ["path"]
            }),
        };

        let anthropic = to_anthropic_format(&tool);
        assert_eq!(anthropic["name"], "read_file");
        assert!(anthropic["input_schema"]["properties"]["path"].is_object());

        let openai = to_openai_format(&tool);
        assert_eq!(openai["type"], "function");
        assert_eq!(openai["function"]["name"], "read_file");
    }
}
