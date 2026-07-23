//! MCP Server implementation
//!
//! Exposes Nanna tools as an MCP server that external clients can connect to.
//! Supports stdio transport (for CLI tools) and HTTP/SSE (for web clients).

use crate::protocol::*;
use crate::{McpError, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Tool handler function type
pub type ToolHandler = Arc<
    dyn Fn(
            Value,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<CallToolResult>> + Send>>
        + Send
        + Sync,
>;

/// Resource handler function type  
pub type ResourceHandler = Arc<
    dyn Fn(
            String,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<ReadResourceResult>> + Send>,
        > + Send
        + Sync,
>;

/// MCP Server configuration
#[derive(Clone)]
pub struct McpServerConfig {
    /// Server name
    pub name: String,
    /// Server version
    pub version: String,
    /// Server instructions (shown to clients)
    pub instructions: Option<String>,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: "nanna".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            instructions: None,
        }
    }
}

/// MCP Server
pub struct McpServer {
    config: McpServerConfig,
    /// Registered tools
    tools: RwLock<HashMap<String, RegisteredTool>>,
    /// Registered resources
    resources: RwLock<HashMap<String, RegisteredResource>>,
    /// Registered prompts
    prompts: RwLock<HashMap<String, Prompt>>,
}

struct RegisteredTool {
    definition: Tool,
    handler: ToolHandler,
}

struct RegisteredResource {
    definition: Resource,
    handler: ResourceHandler,
}

impl McpServer {
    /// Create a new MCP server
    pub fn new(config: McpServerConfig) -> Self {
        Self {
            config,
            tools: RwLock::new(HashMap::new()),
            resources: RwLock::new(HashMap::new()),
            prompts: RwLock::new(HashMap::new()),
        }
    }

    /// Register a tool
    pub async fn register_tool<F, Fut>(&self, tool: Tool, handler: F)
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<CallToolResult>> + Send + 'static,
    {
        let name = tool.name.clone();
        let handler: ToolHandler = Arc::new(move |input| Box::pin(handler(input)));

        let mut tools = self.tools.write().await;
        tools.insert(
            name.clone(),
            RegisteredTool {
                definition: tool,
                handler,
            },
        );
        info!(tool = %name, "Registered MCP tool");
    }

    /// Register a resource
    pub async fn register_resource<F, Fut>(&self, resource: Resource, handler: F)
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<ReadResourceResult>> + Send + 'static,
    {
        let uri = resource.uri.clone();
        let handler: ResourceHandler = Arc::new(move |uri| Box::pin(handler(uri)));

        let mut resources = self.resources.write().await;
        resources.insert(
            uri.clone(),
            RegisteredResource {
                definition: resource,
                handler,
            },
        );
        info!(resource = %uri, "Registered MCP resource");
    }

    /// Register a prompt
    pub async fn register_prompt(&self, prompt: Prompt) {
        let name = prompt.name.clone();
        let mut prompts = self.prompts.write().await;
        prompts.insert(name.clone(), prompt);
        info!(prompt = %name, "Registered MCP prompt");
    }

    /// Handle a JSON-RPC request
    pub async fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        debug!(method = %request.method, id = %request.id, "Handling MCP request");

        let result = match request.method.as_str() {
            "initialize" => self.handle_initialize(request.params).await,
            "tools/list" => self.handle_list_tools(request.params).await,
            "tools/call" => self.handle_call_tool(request.params).await,
            "resources/list" => self.handle_list_resources(request.params).await,
            "resources/read" => self.handle_read_resource(request.params).await,
            "prompts/list" => self.handle_list_prompts(request.params).await,
            "prompts/get" => self.handle_get_prompt(request.params).await,
            "ping" => Ok(serde_json::json!({})),
            _ => Err(McpError::Protocol(format!(
                "Unknown method: {}",
                request.method
            ))),
        };

        match result {
            Ok(value) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: Some(value),
                error: None,
            },
            Err(e) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32000,
                    message: e.to_string(),
                    data: None,
                }),
            },
        }
    }

    async fn handle_initialize(&self, params: Option<Value>) -> Result<Value> {
        let _params: InitializeParams = params
            .map(serde_json::from_value)
            .transpose()?
            .unwrap_or_else(|| InitializeParams {
                protocol_version: PROTOCOL_VERSION.to_string(),
                capabilities: ClientCapabilities::default(),
                client_info: ClientInfo {
                    name: "unknown".to_string(),
                    version: "0.0.0".to_string(),
                },
            });

        let tools = self.tools.read().await;
        let resources = self.resources.read().await;
        let prompts = self.prompts.read().await;

        let result = InitializeResult {
            protocol_version: PROTOCOL_VERSION.to_string(),
            capabilities: ServerCapabilities {
                tools: if tools.is_empty() {
                    None
                } else {
                    Some(ToolsCapability {
                        list_changed: false,
                    })
                },
                resources: if resources.is_empty() {
                    None
                } else {
                    Some(ResourcesCapability {
                        subscribe: false,
                        list_changed: false,
                    })
                },
                prompts: if prompts.is_empty() {
                    None
                } else {
                    Some(PromptsCapability {
                        list_changed: false,
                    })
                },
                logging: Some(LoggingCapability {}),
                experimental: None,
            },
            server_info: ServerInfo {
                name: self.config.name.clone(),
                version: Some(self.config.version.clone()),
            },
            instructions: self.config.instructions.clone(),
        };

        serde_json::to_value(result).map_err(Into::into)
    }

    async fn handle_list_tools(&self, _params: Option<Value>) -> Result<Value> {
        let tools = self.tools.read().await;
        let result = ListToolsResult {
            tools: tools.values().map(|t| t.definition.clone()).collect(),
            next_cursor: None,
        };
        serde_json::to_value(result).map_err(Into::into)
    }

    async fn handle_call_tool(&self, params: Option<Value>) -> Result<Value> {
        let params: CallToolParams = params
            .map(serde_json::from_value)
            .transpose()?
            .ok_or_else(|| McpError::Protocol("Missing params for tools/call".to_string()))?;

        let tools = self.tools.read().await;
        let tool = tools
            .get(&params.name)
            .ok_or_else(|| McpError::ToolNotFound(params.name.clone()))?;

        let input = params
            .arguments
            .unwrap_or(Value::Object(Default::default()));
        let result = (tool.handler)(input).await?;

        serde_json::to_value(result).map_err(Into::into)
    }

    async fn handle_list_resources(&self, _params: Option<Value>) -> Result<Value> {
        let resources = self.resources.read().await;
        let result = ListResourcesResult {
            resources: resources.values().map(|r| r.definition.clone()).collect(),
            next_cursor: None,
        };
        serde_json::to_value(result).map_err(Into::into)
    }

    async fn handle_read_resource(&self, params: Option<Value>) -> Result<Value> {
        let params: ReadResourceParams = params
            .map(serde_json::from_value)
            .transpose()?
            .ok_or_else(|| McpError::Protocol("Missing params for resources/read".to_string()))?;

        let resources = self.resources.read().await;
        let resource = resources
            .get(&params.uri)
            .ok_or_else(|| McpError::ResourceNotFound(params.uri.clone()))?;

        let result = (resource.handler)(params.uri).await?;
        serde_json::to_value(result).map_err(Into::into)
    }

    async fn handle_list_prompts(&self, _params: Option<Value>) -> Result<Value> {
        let prompts = self.prompts.read().await;
        let result = ListPromptsResult {
            prompts: prompts.values().cloned().collect(),
            next_cursor: None,
        };
        serde_json::to_value(result).map_err(Into::into)
    }

    async fn handle_get_prompt(&self, params: Option<Value>) -> Result<Value> {
        let params: GetPromptParams = params
            .map(serde_json::from_value)
            .transpose()?
            .ok_or_else(|| McpError::Protocol("Missing params for prompts/get".to_string()))?;

        let prompts = self.prompts.read().await;
        let prompt = prompts
            .get(&params.name)
            .ok_or_else(|| McpError::Protocol(format!("Prompt not found: {}", params.name)))?;

        // For now, return empty messages - prompts would need template expansion
        let result = GetPromptResult {
            description: prompt.description.clone(),
            messages: vec![],
        };

        serde_json::to_value(result).map_err(Into::into)
    }

    /// Run the server on stdio (for CLI integration)
    pub async fn run_stdio(self: Arc<Self>) -> Result<()> {
        info!("Starting MCP server on stdio");

        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let mut lines = BufReader::new(stdin).lines();

        while let Ok(Some(line)) = lines.next_line().await {
            debug!(line = %line, "Received request");

            // Parse request
            let request: JsonRpcRequest = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(e) => {
                    warn!(error = %e, "Failed to parse request");
                    continue;
                }
            };

            // Handle request
            let response = self.handle_request(request).await;

            // Send response
            let response_json = serde_json::to_string(&response)?;
            debug!(response = %response_json, "Sending response");

            stdout.write_all(response_json.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }

        info!("MCP server stdio loop ended");
        Ok(())
    }
}

/// Protocol version
pub const PROTOCOL_VERSION: &str = "2024-11-05";

// ============================================================================
// Builder for easy server setup
// ============================================================================

/// Builder for creating MCP servers
pub struct McpServerBuilder {
    config: McpServerConfig,
    tools: Vec<(Tool, ToolHandler)>,
}

impl McpServerBuilder {
    /// Create a new builder
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            config: McpServerConfig {
                name: name.into(),
                ..Default::default()
            },
            tools: Vec::new(),
        }
    }

    /// Set server version
    #[must_use]
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.config.version = version.into();
        self
    }

    /// Set server instructions
    #[must_use]
    pub fn instructions(mut self, instructions: impl Into<String>) -> Self {
        self.config.instructions = Some(instructions.into());
        self
    }

    /// Add a tool with a handler
    #[must_use]
    pub fn tool<F, Fut>(mut self, tool: Tool, handler: F) -> Self
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<CallToolResult>> + Send + 'static,
    {
        let handler: ToolHandler = Arc::new(move |input| Box::pin(handler(input)));
        self.tools.push((tool, handler));
        self
    }

    /// Build the server
    pub async fn build(self) -> McpServer {
        let server = McpServer::new(self.config);

        for (tool, handler) in self.tools {
            let name = tool.name.clone();
            let mut tools = server.tools.write().await;
            tools.insert(
                name,
                RegisteredTool {
                    definition: tool,
                    handler,
                },
            );
        }

        server
    }
}

impl Default for McpServerBuilder {
    fn default() -> Self {
        Self::new("nanna")
    }
}

// ============================================================================
// Helper to convert nanna-tools to MCP tools
// ============================================================================

#[cfg(feature = "tools-integration")]
pub mod tools_bridge {
    use super::*;
    use nanna_tools::{ToolCall, ToolRegistry};
    use std::collections::HashMap as StdHashMap;

    /// Register all tools from a ToolRegistry with the MCP server.
    ///
    /// `registry.definitions()` already has the registry's [`ToolPolicy`]
    /// applied, so a tool denied by `[tools] disabled` is never advertised to
    /// the connecting client — and `registry.execute` re-checks the policy after
    /// alias/fuzzy resolution, so it could not be invoked even if a client
    /// guessed the name.
    ///
    /// [`ToolPolicy`]: nanna_tools::ToolPolicy
    ///
    /// # Errors
    ///
    /// Returns [`McpError`] if a tool definition cannot be converted.
    pub async fn register_tools_from_registry(
        server: &McpServer,
        registry: Arc<ToolRegistry>,
    ) -> Result<usize> {
        let definitions = registry.definitions().await;
        let mut count = 0;

        for def in definitions {
            let tool = Tool {
                name: def.name.clone(),
                description: Some(def.description.clone()),
                input_schema: def.to_anthropic_format()["input_schema"].clone(),
            };

            let registry_clone = registry.clone();
            let tool_name = def.name.clone();

            server
                .register_tool(tool, move |input: Value| {
                    let registry = registry_clone.clone();
                    let name = tool_name.clone();
                    async move {
                        // Convert Value to HashMap
                        let params: StdHashMap<String, Value> = input
                            .as_object()
                            .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                            .unwrap_or_default();

                        let call = ToolCall {
                            id: uuid::Uuid::new_v4().to_string(),
                            name,
                            parameters: params,
                        };

                        let response = registry.execute(call).await;

                        let is_error = !response.result.success;
                        let content = if response.result.success {
                            vec![ToolContent::Text {
                                text: response.result.content,
                            }]
                        } else {
                            vec![ToolContent::Text {
                                text: response.result.error.unwrap_or_default(),
                            }]
                        };

                        // Mirror of the client-side mapping: a tool's structured
                        // `data` is exactly what `structuredContent` carries, and
                        // only a successful call has a result to report.
                        let structured_content = if is_error { None } else { response.result.data };

                        Ok(CallToolResult {
                            content,
                            is_error,
                            structured_content,
                        })
                    }
                })
                .await;

            count += 1;
        }

        info!(count, "Registered tools from ToolRegistry");
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_server_creation() {
        let server = McpServer::new(McpServerConfig::default());
        assert!(server.tools.read().await.is_empty());
    }

    #[tokio::test]
    async fn test_register_tool() {
        let server = McpServer::new(McpServerConfig::default());

        let tool = Tool {
            name: "test_tool".to_string(),
            description: Some("A test tool".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        };

        server
            .register_tool(tool, |_| async {
                Ok(CallToolResult {
                    content: vec![ToolContent::Text {
                        text: "success".to_string(),
                    }],
                    is_error: false,
                    structured_content: None,
                })
            })
            .await;

        let tools = server.tools.read().await;
        assert!(tools.contains_key("test_tool"));
    }

    #[tokio::test]
    async fn test_handle_initialize() {
        let server = McpServer::new(McpServerConfig {
            name: "test-server".to_string(),
            version: "1.0.0".to_string(),
            instructions: None,
        });

        let request = JsonRpcRequest::new(
            1,
            "initialize",
            Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "test-client",
                    "version": "1.0.0"
                }
            })),
        );

        let response = server.handle_request(request).await;
        assert!(response.error.is_none());
        assert!(response.result.is_some());
    }
}
