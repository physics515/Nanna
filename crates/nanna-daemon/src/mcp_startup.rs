//! Starting the configured MCP servers (`[mcp]`) and registering their tools.
//!
//! `nanna-mcp` (client, transports, schema guard) and `nanna-agent`'s
//! `McpIntegration` were complete and constructed nowhere: no config section
//! existed and the daemon never called them, so Nanna could not use a single
//! MCP server. This is the missing boot step.
//!
//! **Started in the background.** A server launched through `npx -y` may
//! download a package on its first run; waiting for it would hold the daemon's
//! readiness hostage to the network. Tools register as each server finishes its
//! handshake, and a server that fails is logged by name and skipped — the rest
//! still start.

use std::sync::Arc;

use nanna_agent::mcp::{McpIntegration, McpServerConfig};
use nanna_config::McpConfig;
use nanna_tools::ToolRegistry;
use tokio::sync::broadcast;
use tracing::{info, warn};

/// Spawn every startable server from `config` and register its tools into
/// `tools`. Returns how many servers were handed to the background task.
///
/// The servers are shut down when `shutdown` fires, so their child processes
/// do not outlive a clean daemon exit.
///
/// # Panics
///
/// Never in practice: the assertion restates the bound
/// [`McpConfig::startable`] already enforces.
pub fn spawn_mcp_servers(
    config: &McpConfig,
    tools: Arc<ToolRegistry>,
    mut shutdown: broadcast::Receiver<()>,
) -> usize {
    let (start, skipped) = config.startable();
    for reason in &skipped {
        warn!("{reason}");
    }
    if start.is_empty() {
        return 0;
    }
    let mut integration = McpIntegration::new();
    for entry in &start {
        integration.add_server(
            McpServerConfig::new(entry.name.trim(), entry.command.trim()).args(entry.args.clone()),
        );
    }
    let count = start.len();
    assert!(
        count <= nanna_config::MCP_SERVERS_MAX,
        "startable() enforces the bound"
    );
    info!(servers = count, "Starting MCP servers in the background");
    tokio::spawn(async move {
        match integration.start_all(&tools).await {
            Ok(registered) => info!(
                servers = count,
                tools = registered,
                "MCP tools registered (named mcp__<server>__<tool>)"
            ),
            Err(e) => warn!("MCP tools could not be registered: {e}"),
        }
        // Keep the clients alive for the daemon's lifetime; close them on a
        // clean shutdown rather than leaving it to process exit.
        let _ = shutdown.recv().await;
        if let Err(e) = integration.shutdown().await {
            warn!("MCP servers did not shut down cleanly: {e}");
        }
    });
    count
}
