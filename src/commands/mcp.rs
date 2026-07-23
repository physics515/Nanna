//! `nanna mcp` — expose Nanna's tools to an external MCP client.
//!
//! Nanna already ships an MCP *server* (`nanna-mcp::McpServer`), but nothing
//! started it, so the whole subsystem was reachable only from Rust. This module
//! is the entry point: it loads the filesystem JS/TS skills, applies the user's
//! `[tools]` policy, and serves them over stdio JSON-RPC — the transport every
//! MCP client (Claude Code, Claude Desktop, editors) speaks.
//!
//! **stdout is the protocol.** The caller must send logs to stderr; a stray
//! `println!` or a stdout-writing tracing layer corrupts the JSON-RPC stream and
//! the client disconnects with a parse error. `main` handles this by installing
//! a stderr writer for this command.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use nanna_config::Config;
use nanna_mcp::{McpServer, McpServerConfig, tools_bridge};
use nanna_tools::{ToolPolicy, ToolRegistry};
use tracing::{info, warn};

/// Serve Nanna's tools over stdio JSON-RPC until the client closes the stream.
///
/// `tools_dir_override` wins over the config's `[tools] tools_dir`, which in
/// turn is resolved by the shared `resolve_tools_dir` (so `NANNA_TOOLS_DIR` and
/// the dev-tree fallback keep working).
///
/// # Errors
///
/// Returns an error if no tools directory can be resolved, or if the stdio loop
/// fails to read or write.
pub async fn serve(
    config: &Config,
    tools_dir_override: Option<PathBuf>,
) -> anyhow::Result<()> {
    let tools_dir = tools_dir_override
        .or_else(|| {
            nanna_tools::skills::defaults::resolve_tools_dir(config.tools.tools_dir.as_deref())
        })
        .context(
            "no tools directory found — set [tools] tools_dir in config.toml or NANNA_TOOLS_DIR",
        )?;
    anyhow::ensure!(
        tools_dir.is_dir(),
        "tools directory {} does not exist",
        tools_dir.display()
    );

    let registry = Arc::new(ToolRegistry::new());
    let loaded = registry.load_skills(&tools_dir).await;
    info!("Loaded {loaded} tools from {}", tools_dir.display());
    if loaded == 0 {
        warn!("No tools loaded — the MCP client will see an empty tool list");
    }

    // Apply the user's [tools] enabled/disabled policy BEFORE advertising
    // anything. `definitions()` filters denied tools out of the listing and
    // `execute()` re-checks after alias/fuzzy resolution, so a disabled tool is
    // neither offered to nor invocable by the connecting client.
    let policy = ToolPolicy::from_config_lists(Some(&config.tools.enabled), &config.tools.disabled);
    if !policy.is_unrestricted() {
        info!("Applying [tools] policy to the MCP surface");
        registry.set_policy(policy).await;
    }

    let server = Arc::new(McpServer::new(McpServerConfig {
        name: "nanna".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        instructions: Some(
            "Nanna's local tool surface: filesystem, shell, web, and code tools \
             executed on this machine."
                .to_string(),
        ),
    }));

    let exposed = tools_bridge::register_tools_from_registry(&server, Arc::clone(&registry))
        .await
        .context("failed to register tools with the MCP server")?;
    info!("Exposing {exposed} tools over MCP stdio");
    debug_assert!(
        loaded > 0 || exposed == 0,
        "an empty registry must not advertise any tool"
    );

    server.run_stdio().await.context("MCP stdio loop failed")?;
    info!("MCP client disconnected");
    Ok(())
}
