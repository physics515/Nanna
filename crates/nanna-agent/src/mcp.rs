//! MCP server integration for the agent
//!
//! Provides utilities for spawning MCP servers and registering their tools
//! with the agent's tool registry.

#[cfg(feature = "mcp")]
use nanna_mcp::{McpClient, McpToolsManager, StdioTransport};
use nanna_tools::ToolRegistry;
use tracing::{debug, error, info};

/// MCP server configuration
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    /// Unique name for this server
    pub name: String,
    /// Command to run (e.g., "npx", "python", "node")
    pub command: String,
    /// Arguments to pass to the command
    pub args: Vec<String>,
    /// Environment variables
    pub env: Vec<(String, String)>,
    /// Whether to auto-start on agent init
    pub auto_start: bool,
}

impl McpServerConfig {
    /// Create a new MCP server config
    pub fn new(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
            args: Vec::new(),
            env: Vec::new(),
            auto_start: true,
        }
    }

    /// Add arguments
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// Add environment variables
    #[must_use]
    pub fn env<I, K, V>(mut self, env: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.env = env.into_iter().map(|(k, v)| (k.into(), v.into())).collect();
        self
    }

    /// Set auto-start
    #[must_use]
    pub const fn auto_start(mut self, auto_start: bool) -> Self {
        self.auto_start = auto_start;
        self
    }

    // Common MCP server configurations

    /// Filesystem server
    pub fn filesystem(name: impl Into<String>, paths: &[&str]) -> Self {
        Self::new(name, "npx")
            .args(
                ["-y", "@modelcontextprotocol/server-filesystem"]
                    .into_iter()
                    .chain(paths.iter().copied()),
            )
    }

    /// GitHub server
    pub fn github(name: impl Into<String>, token: impl Into<String>) -> Self {
        Self::new(name, "npx")
            .args(["-y", "@modelcontextprotocol/server-github"])
            .env([("GITHUB_PERSONAL_ACCESS_TOKEN", token.into())])
    }

    /// Brave Search server
    pub fn brave_search(name: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self::new(name, "npx")
            .args(["-y", "@modelcontextprotocol/server-brave-search"])
            .env([("BRAVE_API_KEY", api_key.into())])
    }

    /// Fetch server (web fetching)
    pub fn fetch(name: impl Into<String>) -> Self {
        Self::new(name, "npx").args(["-y", "@modelcontextprotocol/server-fetch"])
    }

    /// Memory server
    pub fn memory(name: impl Into<String>) -> Self {
        Self::new(name, "npx").args(["-y", "@modelcontextprotocol/server-memory"])
    }

    /// Puppeteer server
    pub fn puppeteer(name: impl Into<String>) -> Self {
        Self::new(name, "npx").args(["-y", "@modelcontextprotocol/server-puppeteer"])
    }

    /// Sequential thinking server
    pub fn sequential_thinking(name: impl Into<String>) -> Self {
        Self::new(name, "npx").args(["-y", "@modelcontextprotocol/server-sequential-thinking"])
    }

    /// Custom npx server
    pub fn npx(name: impl Into<String>, package: impl Into<String>) -> Self {
        Self::new(name, "npx").args(["-y", &package.into()])
    }
}

/// MCP integration manager for the agent
#[cfg(feature = "mcp")]
pub struct McpIntegration {
    /// Tool manager for MCP servers
    manager: McpToolsManager<StdioTransport>,
    /// Server configurations
    configs: Vec<McpServerConfig>,
}

#[cfg(feature = "mcp")]
impl McpIntegration {
    /// Create a new MCP integration
    #[must_use]
    pub fn new() -> Self {
        Self {
            manager: McpToolsManager::new(),
            configs: Vec::new(),
        }
    }

    /// Add a server configuration
    pub fn add_server(&mut self, config: McpServerConfig) {
        self.configs.push(config);
    }

    /// Spawn all configured servers and register their tools
    ///
    /// # Errors
    ///
    /// Returns error if any server fails to start
    pub async fn start_all(&self, registry: &ToolRegistry) -> Result<usize, McpStartError> {
        for config in &self.configs {
            if !config.auto_start {
                debug!(server = %config.name, "Skipping MCP server (auto_start=false)");
                continue;
            }

            if let Err(e) = self.start_server(config).await {
                error!(server = %config.name, error = %e, "Failed to start MCP server");
                // Continue with other servers
            }
        }

        // Register all tools with the registry
        let registered = self.manager.register_with_registry(registry).await
            .map_err(|e| McpStartError::Registration(e.to_string()))?;

        info!(servers = self.configs.len(), tools = registered, "MCP integration started");
        Ok(registered)
    }

    /// Start a single MCP server
    async fn start_server(&self, config: &McpServerConfig) -> Result<usize, McpStartError> {
        info!(server = %config.name, command = %config.command, "Starting MCP server");

        // Convert args and env for spawn
        let args: Vec<&str> = config.args.iter().map(String::as_str).collect();
        let env: Vec<(&str, &str)> = config
            .env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        // Spawn the MCP client
        let client = if env.is_empty() {
            McpClient::spawn(&config.command, &args).await
        } else {
            McpClient::spawn_with_env(&config.command, &args, &env).await
        }
        .map_err(|e| McpStartError::Spawn(config.name.clone(), e.to_string()))?;

        // Register with manager
        let tools = self
            .manager
            .register(&config.name, client)
            .await
            .map_err(|e| McpStartError::Registration(e.to_string()))?;

        info!(
            server = %config.name,
            tools = tools.len(),
            "MCP server started"
        );

        Ok(tools.len())
    }

    /// Get the tool manager
    #[must_use]
    pub fn manager(&self) -> &McpToolsManager<StdioTransport> {
        &self.manager
    }

    /// Refresh tools from all servers
    ///
    /// # Errors
    ///
    /// Returns error if refresh fails
    pub async fn refresh(&self) -> Result<(), McpStartError> {
        self.manager
            .refresh()
            .await
            .map_err(|e| McpStartError::Refresh(e.to_string()))
    }

    /// Shutdown all MCP servers
    ///
    /// # Errors
    ///
    /// Returns error if shutdown fails
    pub async fn shutdown(&self) -> Result<(), McpStartError> {
        self.manager
            .close_all()
            .await
            .map_err(|e| McpStartError::Shutdown(e.to_string()))
    }
}

#[cfg(feature = "mcp")]
impl Default for McpIntegration {
    fn default() -> Self {
        Self::new()
    }
}

/// Error starting MCP servers
#[derive(Debug, thiserror::Error)]
pub enum McpStartError {
    #[error("Failed to spawn server '{0}': {1}")]
    Spawn(String, String),
    #[error("Failed to register tools: {0}")]
    Registration(String),
    #[error("Failed to refresh tools: {0}")]
    Refresh(String),
    #[error("Failed to shutdown: {0}")]
    Shutdown(String),
}

/// Builder for MCP integration
#[cfg(feature = "mcp")]
pub struct McpIntegrationBuilder {
    configs: Vec<McpServerConfig>,
}

#[cfg(feature = "mcp")]
impl McpIntegrationBuilder {
    /// Create a new builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            configs: Vec::new(),
        }
    }

    /// Add a server
    #[must_use]
    pub fn server(mut self, config: McpServerConfig) -> Self {
        self.configs.push(config);
        self
    }

    /// Add filesystem access
    #[must_use]
    pub fn filesystem(self, paths: &[&str]) -> Self {
        self.server(McpServerConfig::filesystem("fs", paths))
    }

    /// Add GitHub access
    #[must_use]
    pub fn github(self, token: impl Into<String>) -> Self {
        self.server(McpServerConfig::github("github", token))
    }

    /// Add web search
    #[must_use]
    pub fn brave_search(self, api_key: impl Into<String>) -> Self {
        self.server(McpServerConfig::brave_search("search", api_key))
    }

    /// Add web fetching
    #[must_use]
    pub fn fetch(self) -> Self {
        self.server(McpServerConfig::fetch("fetch"))
    }

    /// Build the integration
    #[must_use]
    pub fn build(self) -> McpIntegration {
        let mut integration = McpIntegration::new();
        integration.configs = self.configs;
        integration
    }
}

#[cfg(feature = "mcp")]
impl Default for McpIntegrationBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_config() {
        let config = McpServerConfig::filesystem("fs", &["/tmp", "/home"]);
        assert_eq!(config.name, "fs");
        assert_eq!(config.command, "npx");
        assert!(config.args.contains(&"-y".to_string()));
    }

    #[test]
    fn test_builder() {
        let _integration = McpIntegrationBuilder::new()
            .filesystem(&["/tmp"])
            .fetch()
            .build();
    }
}
