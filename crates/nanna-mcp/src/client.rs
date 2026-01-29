//! High-level MCP client implementation

use crate::{
    protocol::{ServerInfo, ServerCapabilities, Tool, Resource, Prompt, RequestId, JsonRpcRequest, InitializeResult, InitializeParams, ClientCapabilities, RootsCapability, ClientInfo, JsonRpcNotification, ListToolsResult, CallToolResult, CallToolParams, ListResourcesResult, ReadResourceResult, ReadResourceParams, ListPromptsResult, GetPromptResult, GetPromptParams}, transport::Transport, McpError, Result,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// MCP protocol version
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// MCP client for connecting to tool servers
pub struct McpClient<T: Transport> {
    transport: Arc<T>,
    /// Request ID counter
    id_counter: AtomicI64,
    /// Server info (set after initialization)
    server_info: RwLock<Option<ServerInfo>>,
    /// Server capabilities
    capabilities: RwLock<Option<ServerCapabilities>>,
    /// Cached tools
    tools: RwLock<Vec<Tool>>,
    /// Cached resources
    resources: RwLock<Vec<Resource>>,
    /// Cached prompts
    prompts: RwLock<Vec<Prompt>>,
    /// Whether client is initialized
    initialized: RwLock<bool>,
}

impl<T: Transport> McpClient<T> {
    /// Create a new MCP client with the given transport
    pub fn new(transport: T) -> Self {
        Self {
            transport: Arc::new(transport),
            id_counter: AtomicI64::new(1),
            server_info: RwLock::new(None),
            capabilities: RwLock::new(None),
            tools: RwLock::new(Vec::new()),
            resources: RwLock::new(Vec::new()),
            prompts: RwLock::new(Vec::new()),
            initialized: RwLock::new(false),
        }
    }

    /// Generate next request ID
    fn next_id(&self) -> RequestId {
        RequestId::Number(self.id_counter.fetch_add(1, Ordering::SeqCst))
    }

    /// Send a request and parse the result
    async fn request<R>(&self, method: &str, params: Option<serde_json::Value>) -> Result<R>
    where
        R: serde::de::DeserializeOwned,
    {
        let request = JsonRpcRequest::new(self.next_id(), method, params);
        let response = self.transport.request(request).await?;

        if let Some(error) = response.error {
            return Err(McpError::JsonRpc {
                code: error.code,
                message: error.message,
                data: error.data,
            });
        }

        let result = response
            .result
            .ok_or_else(|| McpError::Protocol("Missing result in response".into()))?;

        serde_json::from_value(result).map_err(Into::into)
    }

    /// Initialize the connection with the server
    ///
    /// Must be called before using any other methods.
    ///
    /// # Errors
    ///
    /// Returns error if initialization fails or server rejects the connection
    pub async fn initialize(&self) -> Result<InitializeResult> {
        let params = InitializeParams {
            protocol_version: PROTOCOL_VERSION.to_string(),
            capabilities: ClientCapabilities {
                roots: Some(RootsCapability { list_changed: true }),
                sampling: None,
                experimental: None,
            },
            client_info: ClientInfo {
                name: "nanna".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };

        let result: InitializeResult = self
            .request("initialize", Some(serde_json::to_value(&params)?))
            .await?;

        info!(
            server = %result.server_info.name,
            version = result.server_info.version.as_deref().unwrap_or("unknown"),
            "Connected to MCP server"
        );

        // Store server info and capabilities
        {
            let mut info = self.server_info.write().await;
            *info = Some(result.server_info.clone());
        }
        {
            let mut caps = self.capabilities.write().await;
            *caps = Some(result.capabilities.clone());
        }

        // Send initialized notification
        self.transport
            .notify(JsonRpcNotification::new("notifications/initialized", None))
            .await?;

        {
            let mut init = self.initialized.write().await;
            *init = true;
        }

        // Pre-fetch tools, resources, and prompts if supported
        if result.capabilities.tools.is_some()
            && let Ok(tools_result) = self.list_tools_internal().await {
                let mut tools = self.tools.write().await;
                *tools = tools_result.tools;
            }

        if result.capabilities.resources.is_some()
            && let Ok(resources_result) = self.list_resources_internal().await {
                let mut resources = self.resources.write().await;
                *resources = resources_result.resources;
            }

        if result.capabilities.prompts.is_some()
            && let Ok(prompts_result) = self.list_prompts_internal().await {
                let mut prompts = self.prompts.write().await;
                *prompts = prompts_result.prompts;
            }

        Ok(result)
    }

    /// Check if client is initialized
    async fn ensure_initialized(&self) -> Result<()> {
        let init = self.initialized.read().await;
        if !*init {
            return Err(McpError::NotInitialized);
        }
        Ok(())
    }

    // ========================================================================
    // Tools
    // ========================================================================

    /// List available tools (internal, no init check)
    async fn list_tools_internal(&self) -> Result<ListToolsResult> {
        self.request("tools/list", None).await
    }

    /// List available tools
    ///
    /// # Errors
    ///
    /// Returns error if not initialized or request fails
    pub async fn list_tools(&self) -> Result<Vec<Tool>> {
        self.ensure_initialized().await?;
        let tools = self.tools.read().await;
        Ok(tools.clone())
    }

    /// Refresh the tools cache
    ///
    /// # Errors
    ///
    /// Returns error if not initialized or request fails
    pub async fn refresh_tools(&self) -> Result<Vec<Tool>> {
        self.ensure_initialized().await?;
        let result = self.list_tools_internal().await?;
        let mut tools = self.tools.write().await;
        *tools = result.tools.clone();
        Ok(result.tools)
    }

    /// Call a tool by name
    ///
    /// # Errors
    ///
    /// Returns error if tool not found, not initialized, or execution fails
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<CallToolResult> {
        self.ensure_initialized().await?;

        debug!(tool = name, "Calling MCP tool");

        let params = CallToolParams {
            name: name.to_string(),
            arguments,
        };

        self.request("tools/call", Some(serde_json::to_value(&params)?))
            .await
    }

    /// Get a tool by name
    pub async fn get_tool(&self, name: &str) -> Option<Tool> {
        let tools = self.tools.read().await;
        tools.iter().find(|t| t.name == name).cloned()
    }

    // ========================================================================
    // Resources
    // ========================================================================

    /// List available resources (internal)
    async fn list_resources_internal(&self) -> Result<ListResourcesResult> {
        self.request("resources/list", None).await
    }

    /// List available resources
    ///
    /// # Errors
    ///
    /// Returns error if not initialized or request fails
    pub async fn list_resources(&self) -> Result<Vec<Resource>> {
        self.ensure_initialized().await?;
        let resources = self.resources.read().await;
        Ok(resources.clone())
    }

    /// Refresh the resources cache
    ///
    /// # Errors
    ///
    /// Returns error if not initialized or request fails
    pub async fn refresh_resources(&self) -> Result<Vec<Resource>> {
        self.ensure_initialized().await?;
        let result = self.list_resources_internal().await?;
        let mut resources = self.resources.write().await;
        *resources = result.resources.clone();
        Ok(result.resources)
    }

    /// Read a resource by URI
    ///
    /// # Errors
    ///
    /// Returns error if resource not found, not initialized, or read fails
    pub async fn read_resource(&self, uri: &str) -> Result<ReadResourceResult> {
        self.ensure_initialized().await?;

        debug!(uri, "Reading MCP resource");

        let params = ReadResourceParams {
            uri: uri.to_string(),
        };

        self.request("resources/read", Some(serde_json::to_value(&params)?))
            .await
    }

    // ========================================================================
    // Prompts
    // ========================================================================

    /// List available prompts (internal)
    async fn list_prompts_internal(&self) -> Result<ListPromptsResult> {
        self.request("prompts/list", None).await
    }

    /// List available prompts
    ///
    /// # Errors
    ///
    /// Returns error if not initialized or request fails
    pub async fn list_prompts(&self) -> Result<Vec<Prompt>> {
        self.ensure_initialized().await?;
        let prompts = self.prompts.read().await;
        Ok(prompts.clone())
    }

    /// Refresh the prompts cache
    ///
    /// # Errors
    ///
    /// Returns error if not initialized or request fails
    pub async fn refresh_prompts(&self) -> Result<Vec<Prompt>> {
        self.ensure_initialized().await?;
        let result = self.list_prompts_internal().await?;
        let mut prompts = self.prompts.write().await;
        *prompts = result.prompts.clone();
        Ok(result.prompts)
    }

    /// Get a prompt by name with arguments
    ///
    /// # Errors
    ///
    /// Returns error if prompt not found, not initialized, or retrieval fails
    pub async fn get_prompt(
        &self,
        name: &str,
        arguments: Option<HashMap<String, String>>,
    ) -> Result<GetPromptResult> {
        self.ensure_initialized().await?;

        debug!(prompt = name, "Getting MCP prompt");

        let params = GetPromptParams {
            name: name.to_string(),
            arguments,
        };

        self.request("prompts/get", Some(serde_json::to_value(&params)?))
            .await
    }

    // ========================================================================
    // Server Info
    // ========================================================================

    /// Get server info
    pub async fn server_info(&self) -> Option<ServerInfo> {
        let info = self.server_info.read().await;
        info.clone()
    }

    /// Get server capabilities
    pub async fn capabilities(&self) -> Option<ServerCapabilities> {
        let caps = self.capabilities.read().await;
        caps.clone()
    }

    /// Check if server supports tools
    pub async fn supports_tools(&self) -> bool {
        let caps = self.capabilities.read().await;
        caps.as_ref().is_some_and(|c| c.tools.is_some())
    }

    /// Check if server supports resources
    pub async fn supports_resources(&self) -> bool {
        let caps = self.capabilities.read().await;
        caps.as_ref().is_some_and(|c| c.resources.is_some())
    }

    /// Check if server supports prompts
    pub async fn supports_prompts(&self) -> bool {
        let caps = self.capabilities.read().await;
        caps.as_ref().is_some_and(|c| c.prompts.is_some())
    }

    // ========================================================================
    // Lifecycle
    // ========================================================================

    /// Close the connection
    ///
    /// # Errors
    ///
    /// Returns error if close fails
    pub async fn close(&self) -> Result<()> {
        self.transport.close().await
    }
}

/// Builder for creating MCP clients with configuration
pub struct McpClientBuilder {
    /// Custom client name
    client_name: Option<String>,
    /// Custom client version
    client_version: Option<String>,
}

impl Default for McpClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl McpClientBuilder {
    /// Create a new builder
    #[must_use]
    pub const fn new() -> Self {
        Self {
            client_name: None,
            client_version: None,
        }
    }

    /// Set custom client name
    #[must_use]
    pub fn client_name(mut self, name: impl Into<String>) -> Self {
        self.client_name = Some(name.into());
        self
    }

    /// Set custom client version
    #[must_use]
    pub fn client_version(mut self, version: impl Into<String>) -> Self {
        self.client_version = Some(version.into());
        self
    }

    /// Build the client with the given transport
    pub fn build<T: Transport>(self, transport: T) -> McpClient<T> {
        McpClient::new(transport)
    }
}

// ============================================================================
// Convenience functions for spawning common servers
// ============================================================================

#[cfg(feature = "stdio")]
impl McpClient<crate::StdioTransport> {
    /// Spawn an MCP server and create a connected client
    ///
    /// # Errors
    ///
    /// Returns error if spawn or initialization fails
    pub async fn spawn(program: &str, args: &[&str]) -> Result<Self> {
        let transport = crate::StdioTransport::spawn(program, args).await?;
        let client = Self::new(transport);
        client.initialize().await?;
        Ok(client)
    }

    /// Spawn with environment variables
    ///
    /// # Errors
    ///
    /// Returns error if spawn or initialization fails
    pub async fn spawn_with_env(
        program: &str,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Result<Self> {
        let transport = crate::StdioTransport::spawn_with_env(program, args, env).await?;
        let client = Self::new(transport);
        client.initialize().await?;
        Ok(client)
    }
}

#[cfg(feature = "http")]
impl McpClient<crate::HttpTransport> {
    /// Connect to an HTTP MCP server
    ///
    /// # Errors
    ///
    /// Returns error if connection or initialization fails
    pub async fn connect(url: impl Into<String>) -> Result<Self> {
        let transport = crate::HttpTransport::connect(url).await?;
        let client = Self::new(transport);
        client.initialize().await?;
        Ok(client)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protocol_version() {
        assert!(!PROTOCOL_VERSION.is_empty());
    }
}
