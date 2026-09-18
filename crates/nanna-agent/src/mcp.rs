//! MCP server integration for the agent
//!
//! Provides utilities for spawning MCP servers and registering their tools
//! with the agent's tool registry.

#[cfg(feature = "mcp")]
use nanna_mcp::{
    AnyTransport, LegacySseTransport, McpClient, McpToolsManager, StdioTransport,
    StreamableHttpTransport,
};
use nanna_tools::ToolRegistry;
use tracing::{debug, error, info};

/// MCP server configuration
#[derive(Clone)]
pub struct McpServerConfig {
    /// Unique name for this server
    pub name: String,
    /// Command to run (e.g., "npx", "python", "node"); empty for a `url` server
    pub command: String,
    /// Arguments to pass to the command
    pub args: Vec<String>,
    /// Environment variables (values may be secrets — never logged)
    pub env: Vec<(String, String)>,
    /// Streamable HTTP endpoint; when set, `command`/`args`/`env` are unused
    pub url: Option<String>,
    /// Sent as `Authorization: Bearer` to a `url` server (never logged)
    pub bearer_token: Option<String>,
    /// Whether to auto-start on agent init
    pub auto_start: bool,
}

/// Redacts every value that may be a secret: `env` values and the token.
impl std::fmt::Debug for McpServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let env_names: Vec<&str> = self.env.iter().map(|(name, _)| name.as_str()).collect();
        f.debug_struct("McpServerConfig")
            .field("name", &self.name)
            .field("command", &self.command)
            .field("args", &self.args)
            .field("env", &env_names)
            .field("url", &self.url)
            .field(
                "bearer_token",
                &self.bearer_token.as_ref().map(|_| "<redacted>"),
            )
            .field("auto_start", &self.auto_start)
            .finish()
    }
}

impl McpServerConfig {
    /// Create a new MCP server config
    pub fn new(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
            args: Vec::new(),
            env: Vec::new(),
            url: None,
            bearer_token: None,
            auto_start: true,
        }
    }

    /// A Streamable HTTP server at `url`.
    pub fn http(name: impl Into<String>, url: impl Into<String>) -> Self {
        let mut config = Self::new(name, "");
        config.url = Some(url.into());
        config
    }

    /// The bearer token sent to a `url` server.
    #[must_use]
    pub fn bearer_token(mut self, token: Option<String>) -> Self {
        self.bearer_token = token;
        self
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
    manager: McpToolsManager<AnyTransport>,
    /// Server configurations
    configs: Vec<McpServerConfig>,
    /// Serves servers' elicitation requests; `None` declares none.
    elicitor: Option<std::sync::Arc<dyn nanna_mcp::Elicitor>>,
}

#[cfg(feature = "mcp")]
impl McpIntegration {
    /// Create a new MCP integration
    #[must_use]
    pub fn new() -> Self {
        Self {
            manager: McpToolsManager::new(),
            configs: Vec::new(),
            elicitor: None,
        }
    }

    /// Put servers' questions (MCP elicitation) to the user through
    /// `elicitor`. Set before [`Self::start_all`].
    pub fn set_elicitor(&mut self, elicitor: std::sync::Arc<dyn nanna_mcp::Elicitor>) {
        self.elicitor = Some(elicitor);
    }

    /// Add a server configuration
    pub fn add_server(&mut self, config: McpServerConfig) {
        self.configs.push(config);
    }

    /// Spawn all configured servers and register their tools.
    ///
    /// One server failing does not stop the others. The outcome of each
    /// auto-started server is returned in configuration order — `Ok(tools)` or
    /// the reason it did not start — so a caller can report per-server state
    /// instead of a single aggregate a failed server hides inside.
    ///
    /// # Errors
    ///
    /// Returns error only if registering the started servers' tools fails.
    pub async fn start_all(
        &self,
        registry: &ToolRegistry,
    ) -> Result<Vec<(String, Result<usize, String>)>, McpStartError> {
        let mut outcomes = Vec::with_capacity(self.configs.len());
        for config in &self.configs {
            if !config.auto_start {
                debug!(server = %config.name, "Skipping MCP server (auto_start=false)");
                continue;
            }
            let outcome = self.start_server(config).await.map_err(|e| {
                error!(server = %config.name, error = %e, "Failed to start MCP server");
                e.to_string()
            });
            outcomes.push((config.name.clone(), outcome));
        }
        debug_assert!(outcomes.len() <= self.configs.len());

        // Register all tools with the registry
        let registered = self.manager.register_with_registry(registry).await
            .map_err(|e| McpStartError::Registration(e.to_string()))?;

        info!(servers = self.configs.len(), tools = registered, "MCP integration started");
        Ok(outcomes)
    }

    /// Spawn a `command` server or connect to a `url` one, and run the
    /// dual-era handshake either way.
    async fn connect(
        config: &McpServerConfig,
        elicitor: Option<std::sync::Arc<dyn nanna_mcp::Elicitor>>,
    ) -> Result<McpClient<AnyTransport>, nanna_mcp::McpError> {
        let with_elicitor = |client: McpClient<AnyTransport>| match &elicitor {
            Some(elicitor) => client.with_elicitor(std::sync::Arc::clone(elicitor)),
            None => client,
        };
        let Some(url) = &config.url else {
            let args: Vec<&str> = config.args.iter().map(String::as_str).collect();
            let env: Vec<(&str, &str)> = config
                .env
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            let transport = AnyTransport::Stdio(StdioTransport::spawn_with_env(
                &config.command,
                &args,
                &env,
            )?);
            let client = with_elicitor(McpClient::new(transport));
            client.initialize().await?;
            return Ok(client);
        };
        let token = config.bearer_token.clone();
        let transport =
            AnyTransport::Http(Box::new(StreamableHttpTransport::new(url, token.clone())?));
        let client = with_elicitor(McpClient::new(transport));
        match client.initialize().await {
            Ok(_) => Ok(client),
            // No Streamable HTTP endpoint here, and no modern error body
            // either: the binding's cue to try the deprecated HTTP+SSE
            // transport, which is legacy-only (so no era probe).
            Err(nanna_mcp::McpError::HttpStatus {
                status: 400 | 404 | 405,
                ..
            }) => {
                info!(%url, "No Streamable HTTP endpoint; trying the 2024 HTTP+SSE transport");
                let legacy = LegacySseTransport::connect(url, token).await?;
                let client = with_elicitor(McpClient::new(AnyTransport::Sse(Box::new(legacy))));
                client.initialize_legacy().await?;
                Ok(client)
            }
            Err(e) => Err(e),
        }
    }

    /// Start a single MCP server
    async fn start_server(&self, config: &McpServerConfig) -> Result<usize, McpStartError> {
        if let Some(url) = &config.url {
            info!(server = %config.name, %url, "Connecting to MCP server");
        } else {
            info!(server = %config.name, command = %config.command, "Starting MCP server");
        }

        let client = Self::connect(config, self.elicitor.clone())
            .await
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

    /// Keep `registry` in step with every server's tool list until `stop`
    /// resolves (see [`McpToolsManager::watch_list_changes`]).
    pub async fn watch(
        &self,
        registry: &ToolRegistry,
        stop: impl std::future::Future<Output = ()>,
    ) {
        self.manager.watch_list_changes(registry, stop).await;
    }

    /// Get the tool manager
    #[must_use]
    pub const fn manager(&self) -> &McpToolsManager<AnyTransport> {
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
    pub const fn new() -> Self {
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
    #[test]
    fn debug_never_prints_a_secret() {
        let config = super::McpServerConfig::http("notion", "https://x/mcp")
            .bearer_token(Some("sk-live-123".into()));
        let stdio = super::McpServerConfig::new("gh", "npx").env([("GITHUB_TOKEN", "ghp_456")]);
        let printed = format!("{config:?} {stdio:?}");
        assert!(!printed.contains("sk-live-123"), "{printed}");
        assert!(!printed.contains("ghp_456"), "{printed}");
        assert!(
            printed.contains("GITHUB_TOKEN"),
            "names stay visible: {printed}"
        );
        assert!(printed.contains("<redacted>"), "{printed}");
    }

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
