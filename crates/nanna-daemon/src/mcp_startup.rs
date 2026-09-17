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

/// What one configured MCP server is doing, for `system.status`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct McpServerState {
    pub name: String,
    /// `starting` | `started` | `failed` | `not_started`
    pub state: &'static str,
    /// Tools registered from it (started servers only).
    pub tools: usize,
    /// Why it failed or was not started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Per-server MCP state, shared between the boot task that writes it and the
/// control plane that reports it. Bounded by the configured server count.
pub type McpStatus = Arc<tokio::sync::RwLock<Vec<McpServerState>>>;

/// Record one server's outcome in place. Pure over the slice.
fn record_outcome(servers: &mut [McpServerState], name: &str, outcome: Result<usize, String>) {
    let Some(state) = servers.iter_mut().find(|s| s.name == name) else {
        debug_assert!(
            false,
            "an outcome for a server that was never recorded: {name}"
        );
        return;
    };
    match outcome {
        Ok(tools) => {
            state.state = "started";
            state.tools = tools;
        }
        Err(reason) => {
            state.state = "failed";
            state.detail = Some(reason);
        }
    }
}

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
pub async fn spawn_mcp_servers(
    config: &McpConfig,
    tools: Arc<ToolRegistry>,
    status: McpStatus,
    mut shutdown: broadcast::Receiver<()>,
) -> usize {
    let (start, skipped) = config.startable();
    for reason in &skipped {
        warn!("{reason}");
    }
    {
        let mut servers = status.write().await;
        servers.clear();
        servers.extend(start.iter().map(|entry| McpServerState {
            name: entry.name.trim().to_string(),
            state: "starting",
            tools: 0,
            detail: None,
        }));
        servers.extend(skipped.iter().map(|reason| McpServerState {
            name: String::new(),
            state: "not_started",
            tools: 0,
            detail: Some(reason.clone()),
        }));
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
            Ok(outcomes) => {
                let mut servers = status.write().await;
                for (name, outcome) in outcomes {
                    record_outcome(&mut servers, &name, outcome);
                }
                let registered: usize = servers.iter().map(|s| s.tools).sum();
                drop(servers);
                info!(
                    servers = count,
                    tools = registered,
                    "MCP tools registered (named mcp__<server>__<tool>)"
                );
            }
            Err(e) => {
                warn!("MCP tools could not be registered: {e}");
                for state in status
                    .write()
                    .await
                    .iter_mut()
                    .filter(|s| s.state == "starting")
                {
                    state.state = "failed";
                    state.detail = Some(e.to_string());
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn starting(name: &str) -> McpServerState {
        McpServerState {
            name: name.into(),
            state: "starting",
            tools: 0,
            detail: None,
        }
    }

    #[test]
    fn outcomes_land_on_the_server_they_belong_to() {
        let mut servers = vec![starting("files"), starting("git")];
        record_outcome(&mut servers, "git", Err("No such file or directory".into()));
        record_outcome(&mut servers, "files", Ok(3));
        assert_eq!(servers[0].state, "started");
        assert_eq!(servers[0].tools, 3);
        assert_eq!(servers[1].state, "failed");
        assert_eq!(
            servers[1].detail.as_deref(),
            Some("No such file or directory")
        );
    }

    #[tokio::test]
    async fn skipped_entries_are_reported_and_nothing_starts_without_servers() {
        let status = McpStatus::default();
        let config = McpConfig {
            servers: vec![nanna_config::McpServerEntry {
                name: "broken".into(),
                command: " ".into(),
                args: Vec::new(),
                enabled: true,
            }],
        };
        let (_tx, rx) = broadcast::channel(1);
        let started =
            spawn_mcp_servers(&config, Arc::new(ToolRegistry::new()), status.clone(), rx).await;
        assert_eq!(started, 0);
        let servers = status.read().await.clone();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].state, "not_started");
        assert!(
            servers[0]
                .detail
                .as_deref()
                .is_some_and(|d| d.contains("has no command"))
        );
    }
}
