//! Adapter for integrating MCP tools with nanna-tools
//!
//! Provides a bridge between MCP tool servers and the nanna-tools registry,
//! allowing MCP tools to be used seamlessly in the agent loop.

use crate::{McpClient, McpError, Tool as McpTool, ToolContent};
use crate::transport::Transport;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};

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

// ============================================================================
// nanna-tools Integration
// ============================================================================

#[cfg(feature = "tools-integration")]
mod tools_impl {
    use super::*;
    use async_trait::async_trait;
    use nanna_tools::{Tool, ToolDefinition, ToolError, ToolParameter, ToolResult, ParameterType};
    use serde_json::Value;

    /// Wrapper that adapts an MCP tool to the nanna-tools `Tool` trait
    pub struct McpToolWrapper<T: Transport + 'static> {
        /// The MCP client
        client: Arc<McpClient<T>>,
        /// The tool definition from MCP
        tool: McpTool,
        /// Prefix for the tool name (usually server name)
        prefix: String,
    }

    impl<T: Transport + 'static> McpToolWrapper<T> {
        /// Create a new wrapper for an MCP tool
        pub fn new(client: Arc<McpClient<T>>, tool: McpTool, prefix: impl Into<String>) -> Self {
            Self {
                client,
                tool,
                prefix: prefix.into(),
            }
        }

        /// Get the full tool name (prefix:name)
        #[must_use]
        pub fn full_name(&self) -> String {
            if self.prefix.is_empty() {
                self.tool.name.clone()
            } else {
                format!("{}:{}", self.prefix, self.tool.name)
            }
        }

        /// Convert MCP JSON Schema to nanna-tools parameters
        fn schema_to_parameters(schema: &Value) -> Vec<ToolParameter> {
            let mut params = Vec::new();

            let properties = schema
                .get("properties")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();

            let required: Vec<String> = schema
                .get("required")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();

            for (name, prop) in properties {
                let param_type = prop
                    .get("type")
                    .and_then(Value::as_str)
                    .map(|t| match t {
                        "string" => ParameterType::String,
                        "integer" => ParameterType::Integer,
                        "number" => ParameterType::Number,
                        "boolean" => ParameterType::Boolean,
                        "array" => ParameterType::Array,
                        "object" => ParameterType::Object,
                        _ => ParameterType::String,
                    })
                    .unwrap_or(ParameterType::String);

                let description = prop
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();

                let enum_values = prop.get("enum").and_then(Value::as_array).map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect()
                });

                let default = prop.get("default").cloned();

                params.push(ToolParameter {
                    name,
                    description,
                    param_type,
                    required: required.contains(&params.last().map_or(String::new(), |p: &ToolParameter| p.name.clone())),
                    default,
                    enum_values,
                });
            }

            // Fix required check (we need to check against the actual name)
            for param in &mut params {
                param.required = required.contains(&param.name);
            }

            params
        }
    }

    #[async_trait]
    impl<T: Transport + 'static> Tool for McpToolWrapper<T> {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: self.full_name(),
                description: self.tool.description.clone().unwrap_or_default(),
                parameters: Self::schema_to_parameters(&self.tool.input_schema),
                output_schema: None,
            }
        }

        async fn execute(
            &self,
            params: HashMap<String, Value>,
        ) -> Result<ToolResult, ToolError> {
            debug!(tool = %self.full_name(), "Executing MCP tool");

            // Convert HashMap to Value for MCP
            let arguments = if params.is_empty() {
                None
            } else {
                Some(Value::Object(params.into_iter().collect()))
            };

            // Call the MCP tool
            let result = self
                .client
                .call_tool(&self.tool.name, arguments)
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

            // Convert MCP content to string
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
                            resource
                                .blob
                                .as_ref()
                                .map(|b| format!("[Blob: {} bytes]", b.len()))
                        })
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");

            if result.is_error {
                Ok(ToolResult::error(content))
            } else {
                Ok(ToolResult::success(content))
            }
        }

        fn timeout_secs(&self) -> Option<u64> {
            Some(60) // MCP tools may be slower
        }
    }

    /// Manager for multiple MCP server connections with nanna-tools integration
    pub struct McpToolsManager<T: Transport + 'static> {
        /// Connected MCP clients by server name
        clients: RwLock<HashMap<String, Arc<McpClient<T>>>>,
        /// All available tool wrappers
        tool_wrappers: RwLock<Vec<Arc<McpToolWrapper<T>>>>,
    }

    impl<T: Transport + 'static> McpToolsManager<T> {
        /// Create a new MCP tools manager
        #[must_use]
        pub fn new() -> Self {
            Self {
                clients: RwLock::new(HashMap::new()),
                tool_wrappers: RwLock::new(Vec::new()),
            }
        }

        /// Register an MCP client and its tools
        ///
        /// # Errors
        ///
        /// Returns error if client is not initialized
        pub async fn register(
            &self,
            name: impl Into<String>,
            client: McpClient<T>,
        ) -> Result<Vec<Arc<McpToolWrapper<T>>>, McpError> {
            let name = name.into();
            let client = Arc::new(client);

            // Get tools from the server
            let tools = client.list_tools().await?;
            debug!(server = %name, count = tools.len(), "Registered MCP server tools");

            let mut wrappers = Vec::new();
            for tool in tools {
                let wrapper = Arc::new(McpToolWrapper::new(client.clone(), tool, &name));
                wrappers.push(wrapper);
            }

            // Store
            {
                let mut clients = self.clients.write().await;
                clients.insert(name, client);
            }
            {
                let mut tool_wrappers = self.tool_wrappers.write().await;
                tool_wrappers.extend(wrappers.iter().cloned());
            }

            Ok(wrappers)
        }

        /// Register all tools with a ToolRegistry
        ///
        /// # Errors
        ///
        /// Returns error if registration fails
        pub async fn register_with_registry(
            &self,
            registry: &nanna_tools::ToolRegistry,
        ) -> Result<usize, McpError> {
            let wrappers = self.tool_wrappers.read().await;
            let count = wrappers.len();

            for wrapper in wrappers.iter() {
                registry.register_boxed(wrapper.clone()).await;
            }

            Ok(count)
        }

        /// Get all tool wrappers
        pub async fn tools(&self) -> Vec<Arc<McpToolWrapper<T>>> {
            self.tool_wrappers.read().await.clone()
        }

        /// Refresh tools from all servers
        ///
        /// # Errors
        ///
        /// Returns error if refresh fails for any server
        pub async fn refresh(&self) -> Result<(), McpError> {
            let mut new_wrappers = Vec::new();

            let clients = self.clients.read().await;
            for (name, client) in clients.iter() {
                match client.refresh_tools().await {
                    Ok(tools) => {
                        for tool in tools {
                            let wrapper = Arc::new(McpToolWrapper::new(client.clone(), tool, name));
                            new_wrappers.push(wrapper);
                        }
                    }
                    Err(e) => {
                        warn!(server = %name, error = %e, "Failed to refresh tools");
                    }
                }
            }

            let mut tool_wrappers = self.tool_wrappers.write().await;
            *tool_wrappers = new_wrappers;

            Ok(())
        }

        /// Close all connections
        ///
        /// # Errors
        ///
        /// Returns error if any close fails
        pub async fn close_all(&self) -> Result<(), McpError> {
            let clients = self.clients.read().await;
            for client in clients.values() {
                client.close().await?;
            }
            Ok(())
        }
    }

    impl<T: Transport + 'static> Default for McpToolsManager<T> {
        fn default() -> Self {
            Self::new()
        }
    }
}

#[cfg(feature = "tools-integration")]
pub use tools_impl::{McpToolWrapper, McpToolsManager};

// ============================================================================
// Standalone adapter (no nanna-tools dependency)
// ============================================================================

/// Wrapper that adapts an MCP tool for standalone use
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

/// Manager for multiple MCP server connections (standalone, no nanna-tools)
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
