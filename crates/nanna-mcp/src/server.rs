//! MCP Server implementation
//!
//! Exposes Nanna tools as an MCP server that external clients can connect to.
//! Supports stdio transport (for CLI tools) and HTTP/SSE (for web clients).

use crate::protocol::{RequestId, CallToolResult, ReadResourceResult, Prompt, Tool, Resource, JsonRpcRequest, JsonRpcResponse, JsonRpcError, InitializeParams, ClientCapabilities, ClientInfo, InitializeResult, ServerCapabilities, ToolsCapability, ResourcesCapability, PromptsCapability, LoggingCapability, ServerInfo, ListToolsResult, CallToolParams, ListResourcesResult, ReadResourceParams, ListPromptsResult, GetPromptParams, GetPromptResult};
#[cfg(any(test, feature = "tools-integration"))]
use crate::protocol::ToolContent;
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
    #[must_use]
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

        self.tools.write().await.insert(
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

        self.resources.write().await.insert(
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
        self.prompts.write().await.insert(name.clone(), prompt);
        info!(prompt = %name, "Registered MCP prompt");
    }

    /// Handle a JSON-RPC request
    pub async fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        debug!(method = %request.method, id = %request.id, "Handling MCP request");

        // A modern (2026-07-28) request names its revision in `_meta`; one we
        // do not speak is refused with the list we do, so the client can pick.
        let modern_version = request_version(request.params.as_ref());
        if let Some(version) = &modern_version
            && !crate::era::MODERN_PROTOCOL_VERSIONS.contains(&version.as_str())
        {
            return error_response(
                request.id,
                crate::protocol::error_codes::UNSUPPORTED_PROTOCOL_VERSION,
                "Unsupported protocol version".to_string(),
                Some(
                    serde_json::json!({ "supported": supported_versions(), "requested": version }),
                ),
            );
        }

        let result = match request.method.as_str() {
            "server/discover" => self.handle_discover().await,
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
            Ok(mut value) => {
                // Modern results say what they are; legacy clients never
                // expect the field.
                if modern_version.is_some() || request.method == "server/discover" {
                    mark_modern_result(&request.method, &mut value);
                }
                JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: Some(value),
                    error: None,
                }
            }
            Err(e) => {
                let code = error_code_for(&e);
                error_response(request.id, code, e.to_string(), None)
            }
        }
    }

    /// `server/discover` (2026-07-28): the revisions this server speaks, its
    /// capabilities and identity — everything `initialize` reports, with no
    /// session behind it.
    async fn handle_discover(&self) -> Result<Value> {
        let capabilities = self.capabilities().await;
        let mut result = serde_json::json!({
            "supportedVersions": supported_versions(),
            "capabilities": capabilities,
            "_meta": { crate::era::META_SERVER_INFO: {
                "name": self.config.name, "version": self.config.version } },
        });
        if let Some(instructions) = &self.config.instructions
            && let Some(object) = result.as_object_mut()
        {
            object.insert(
                "instructions".to_string(),
                Value::from(instructions.as_str()),
            );
        }
        debug_assert!(
            result["supportedVersions"]
                .as_array()
                .is_some_and(|v| !v.is_empty())
        );
        Ok(result)
    }

    /// What this server offers: the kinds it has anything registered for.
    async fn capabilities(&self) -> ServerCapabilities {
        let has_tools = !self.tools.read().await.is_empty();
        let has_resources = !self.resources.read().await.is_empty();
        let has_prompts = !self.prompts.read().await.is_empty();
        ServerCapabilities {
            tools: has_tools.then_some(ToolsCapability {
                list_changed: false,
            }),
            resources: has_resources.then_some(ResourcesCapability {
                subscribe: false,
                list_changed: false,
            }),
            prompts: has_prompts.then_some(PromptsCapability {
                list_changed: false,
            }),
            logging: Some(LoggingCapability {}),
            experimental: None,
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

        let result = InitializeResult {
            protocol_version: PROTOCOL_VERSION.to_string(),
            capabilities: self.capabilities().await,
            server_info: ServerInfo {
                name: self.config.name.clone(),
                version: Some(self.config.version.clone()),
            },
            instructions: self.config.instructions.clone(),
        };

        serde_json::to_value(result).map_err(Into::into)
    }

    async fn handle_list_tools(&self, _params: Option<Value>) -> Result<Value> {
        let tools = self.tools.read().await.values().map(|t| t.definition.clone()).collect();
        let result = ListToolsResult {
            tools,
            next_cursor: None,
        };
        serde_json::to_value(result).map_err(Into::into)
    }

    async fn handle_call_tool(&self, params: Option<Value>) -> Result<Value> {
        let params: CallToolParams = params
            .map(serde_json::from_value)
            .transpose()?
            .ok_or_else(|| McpError::Protocol("Missing params for tools/call".to_string()))?;

        // Only the handler is taken out of the registry; the call itself runs
        // without the lock, so a long-running tool does not hold off
        // registrations (or, behind a queued registration, other calls).
        let handler = self
            .tools
            .read()
            .await
            .get(&params.name)
            .map(|tool| Arc::clone(&tool.handler))
            .ok_or_else(|| McpError::ToolNotFound(params.name.clone()))?;

        let input = params
            .arguments
            .unwrap_or_else(|| Value::Object(serde_json::Map::default()));
        let result = handler(input).await?;

        serde_json::to_value(result).map_err(Into::into)
    }

    async fn handle_list_resources(&self, _params: Option<Value>) -> Result<Value> {
        let resources = self.resources.read().await.values().map(|r| r.definition.clone()).collect();
        let result = ListResourcesResult {
            resources,
            next_cursor: None,
        };
        serde_json::to_value(result).map_err(Into::into)
    }

    async fn handle_read_resource(&self, params: Option<Value>) -> Result<Value> {
        let params: ReadResourceParams = params
            .map(serde_json::from_value)
            .transpose()?
            .ok_or_else(|| McpError::Protocol("Missing params for resources/read".to_string()))?;

        // As in `handle_call_tool`: the read runs without the registry lock.
        let handler = self
            .resources
            .read()
            .await
            .get(&params.uri)
            .map(|resource| Arc::clone(&resource.handler))
            .ok_or_else(|| McpError::ResourceNotFound(params.uri.clone()))?;

        let result = handler(params.uri).await?;
        serde_json::to_value(result).map_err(Into::into)
    }

    async fn handle_list_prompts(&self, _params: Option<Value>) -> Result<Value> {
        let prompts = self.prompts.read().await.values().cloned().collect();
        let result = ListPromptsResult {
            prompts,
            next_cursor: None,
        };
        serde_json::to_value(result).map_err(Into::into)
    }

    async fn handle_get_prompt(&self, params: Option<Value>) -> Result<Value> {
        let params: GetPromptParams = params
            .map(serde_json::from_value)
            .transpose()?
            .ok_or_else(|| McpError::Protocol("Missing params for prompts/get".to_string()))?;

        let description = self
            .prompts
            .read()
            .await
            .get(&params.name)
            .map(|prompt| prompt.description.clone())
            .ok_or_else(|| McpError::Protocol(format!("Prompt not found: {}", params.name)))?;

        // For now, return empty messages - prompts would need template expansion
        let result = GetPromptResult {
            description,
            messages: vec![],
        };

        serde_json::to_value(result).map_err(Into::into)
    }

    /// Run the server on stdio (for CLI integration)
    ///
    /// The loop ends, with `Ok`, when stdin reaches EOF or can no longer be
    /// read; a line that is not a JSON-RPC request is logged and skipped.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::Serialization`] if a response cannot be serialised,
    /// or [`McpError::Io`] if writing or flushing the response to stdout fails.
    pub async fn run_stdio(self: Arc<Self>) -> Result<()> {
        info!("Starting MCP server on stdio");

        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let mut lines = BufReader::new(stdin).lines();

        while let Ok(Some(line)) = lines.next_line().await {
            debug!(line = %line, "Received request");

            let response_json = match classify_line(&line) {
                Incoming::Request(request) => {
                    serde_json::to_string(&self.handle_request(request).await)?
                }
                // Notifications (`notifications/initialized`, `cancelled`, …)
                // are one-way: nothing to answer, nothing wrong.
                Incoming::Notification(method) => {
                    debug!(method, "MCP notification");
                    continue;
                }
                // JSON-RPC requires an answer to a request it cannot read —
                // dropping it leaves the client waiting out its timeout.
                Incoming::Malformed(response) => {
                    warn!(error = %response["error"], "Malformed MCP request");
                    response.to_string()
                }
            };

            // Send response
            debug!(response = %response_json, "Sending response");

            stdout.write_all(response_json.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }

        info!("MCP server stdio loop ended");
        Ok(())
    }
}

/// Every revision this server answers: the modern ones statelessly, the
/// legacy one through `initialize`.
fn supported_versions() -> Vec<&'static str> {
    let mut versions = crate::era::MODERN_PROTOCOL_VERSIONS.to_vec();
    versions.push(PROTOCOL_VERSION);
    versions
}

/// Methods whose modern results MUST carry caching hints.
const CACHEABLE_METHODS: [&str; 6] = [
    "server/discover",
    "tools/list",
    "prompts/list",
    "resources/list",
    "resources/templates/list",
    "resources/read",
];

/// Tag a modern result: `resultType: "complete"`, plus the caching hints
/// the cacheable methods require. `ttlMs: 0` because what this server lists
/// can change at any moment (the daemon's registry does, as its own MCP
/// servers come and go); `private` because it is one user's machine. Pure.
fn mark_modern_result(method: &str, value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.insert("resultType".to_string(), Value::from("complete"));
    if CACHEABLE_METHODS.contains(&method) {
        object.insert("ttlMs".to_string(), Value::from(0));
        object.insert("cacheScope".to_string(), Value::from("private"));
    }
    debug_assert!(object.contains_key("resultType"));
}

/// The revision a modern request names in its `_meta`, if it names one. Pure.
fn request_version(params: Option<&Value>) -> Option<String> {
    params?
        .get("_meta")?
        .get(crate::era::META_PROTOCOL_VERSION)?
        .as_str()
        .map(str::to_string)
}

/// The JSON-RPC code for a handler failure: the standard codes the spec
/// assigns (unknown method, bad params) instead of the old catch-all
/// `-32000`, which the 2026-07-28 revision tells new code not to emit. Pure.
fn error_code_for(error: &McpError) -> i32 {
    match error {
        McpError::Protocol(message) if message.starts_with("Unknown method") => -32601,
        McpError::ToolNotFound(_) | McpError::ResourceNotFound(_) | McpError::Serialization(_) => {
            crate::protocol::error_codes::INVALID_PARAMS
        }
        McpError::Protocol(message) if message.starts_with("Missing params") => {
            crate::protocol::error_codes::INVALID_PARAMS
        }
        _ => -32603,
    }
}

fn error_response(
    id: RequestId,
    code: i32,
    message: String,
    data: Option<Value>,
) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message,
            data,
        }),
    }
}

/// One line from an MCP client, by JSON-RPC shape.
#[derive(Debug)]
enum Incoming {
    Request(JsonRpcRequest),
    Notification(String),
    /// The error answer for a line that is not a readable request.
    Malformed(Value),
}

/// Classify one line: a request, a notification (a `method` and no `id`),
/// or something to answer with a parse (`-32700`) or invalid-request
/// (`-32600`) error under whatever id could be read. Pure.
fn classify_line(line: &str) -> Incoming {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Incoming::Malformed(malformed(&Value::Null, -32700, "Parse error"));
    };
    let id = value.get("id").filter(|id| !id.is_null()).cloned();
    let method = value.get("method").and_then(Value::as_str);
    match (id, method) {
        (None, Some(method)) => Incoming::Notification(method.to_string()),
        (id, _) => serde_json::from_value::<JsonRpcRequest>(value).map_or_else(
            |e| {
                // Echo the id only if it is one (string or number).
                let id = id
                    .filter(|id| serde_json::from_value::<RequestId>(id.clone()).is_ok())
                    .unwrap_or(Value::Null);
                Incoming::Malformed(malformed(&id, -32600, &format!("Invalid request: {e}")))
            },
            Incoming::Request,
        ),
    }
}

/// A JSON-RPC error response; `id` is `null` when the request's could not
/// be read, as the spec requires. Pure.
fn malformed(id: &Value, code: i32, message: &str) -> Value {
    debug_assert!(id.is_null() || id.is_string() || id.is_number());
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
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
    use super::{info, McpServer, Arc, Result, Tool, Value, ToolContent, CallToolResult};
    use nanna_tools::{ToolCall, ToolRegistry};
    use std::collections::HashMap as StdHashMap;

    /// Register all tools from a `ToolRegistry` with the MCP server.
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
    /// Returns [`McpError`](crate::McpError) if a tool definition cannot be converted.
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
                            .map_or_default(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect());

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

    fn modern(method: &str, params: Value) -> JsonRpcRequest {
        let mut params = params;
        params["_meta"] = serde_json::json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {}
        });
        JsonRpcRequest::new(1_i64, method, Some(params))
    }

    #[tokio::test]
    async fn the_server_answers_modern_clients_statelessly() {
        let server = McpServer::new(McpServerConfig::default());
        let discover = server
            .handle_request(modern("server/discover", serde_json::json!({})))
            .await;
        let result = discover.result.expect("discover answers");
        assert_eq!(
            result["supportedVersions"],
            serde_json::json!(["2026-07-28", "2024-11-05"])
        );
        assert_eq!(result["resultType"], "complete");
        assert_eq!(result["ttlMs"], 0, "cacheable results carry caching hints");
        assert_eq!(result["cacheScope"], "private");
        assert_eq!(
            result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            "nanna"
        );

        // No initialize needed; a list is tagged too, a call only as complete.
        let list = server
            .handle_request(modern("tools/list", serde_json::json!({})))
            .await;
        assert_eq!(list.result.as_ref().expect("list")["ttlMs"], 0);
        let call = server
            .handle_request(modern(
                "tools/call",
                serde_json::json!({ "name": "absent" }),
            ))
            .await;
        assert_eq!(call.error.expect("unknown tool").code, -32602);
    }

    #[tokio::test]
    async fn an_unknown_revision_or_method_gets_the_spec_codes() {
        let server = McpServer::new(McpServerConfig::default());
        let mut future = modern("tools/list", serde_json::json!({}));
        future.params.as_mut().expect("params")["_meta"]["io.modelcontextprotocol/protocolVersion"] =
            Value::from("2099-01-01");
        let refused = server.handle_request(future).await.error.expect("refused");
        assert_eq!(refused.code, -32022);
        assert_eq!(refused.data.expect("data")["supported"][0], "2026-07-28");

        let unknown = server
            .handle_request(modern("teleport", serde_json::json!({})))
            .await;
        assert_eq!(unknown.error.expect("unknown").code, -32601);

        // Legacy traffic is untouched: no resultType on a legacy answer.
        let legacy = server
            .handle_request(JsonRpcRequest::new(2_i64, "tools/list", None))
            .await;
        assert!(
            legacy
                .result
                .expect("legacy list")
                .get("resultType")
                .is_none()
        );
    }

    #[test]
    fn notifications_are_accepted_and_bad_requests_answered() {
        assert!(matches!(
            classify_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            Incoming::Notification(m) if m == "notifications/initialized"
        ));
        assert!(matches!(
            classify_line(r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#),
            Incoming::Request(_)
        ));
        let Incoming::Malformed(parse) = classify_line("not json") else {
            panic!("unreadable text must be answered");
        };
        assert_eq!(parse["error"]["code"], -32700);
        assert!(parse["id"].is_null(), "an unreadable id is null");
        let Incoming::Malformed(invalid) =
            classify_line(r#"{"jsonrpc":"2.0","id":"x","params":{}}"#)
        else {
            panic!("a request without a method must be answered");
        };
        assert_eq!(invalid["error"]["code"], -32600);
        assert_eq!(
            invalid["id"], "x",
            "the id is echoed so the client can match it"
        );
    }

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

        assert!(server.tools.read().await.contains_key("test_tool"));
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
