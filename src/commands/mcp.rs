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

/// Why `server`/`var` cannot name a stored MCP secret, if they cannot. Pure.
fn secret_target_problem(server: &str, var: &str) -> Option<String> {
    if server.trim().is_empty() {
        return Some("the server name is empty".to_string());
    }
    if !nanna_config::mcp::is_env_var_name(var) {
        return Some(format!(
            "'{var}' is not an environment variable name (letters, digits and _, not starting with a digit)"
        ));
    }
    None
}

/// A note when the stored secret is not wired to a configured server yet. Pure.
fn secret_wiring_note(config: &Config, server: &str, var: &str) -> Option<String> {
    let server = server.trim();
    match config
        .mcp
        .servers
        .iter()
        .find(|entry| entry.name.trim() == server)
    {
        None => Some(format!(
            "note: no [[mcp.servers]] entry is named '{server}' yet; add one with secret_env = [\"{var}\"]"
        )),
        Some(entry) if !entry.secret_env.iter().any(|name| name == var) => Some(format!(
            "note: '{server}' does not list {var} in secret_env, so it will not receive it"
        )),
        Some(_) => None,
    }
}

/// `nanna mcp secret set <server> <VAR>`: read the value without echoing it
/// (or from stdin when piped) and store it in the secure store.
///
/// # Errors
///
/// Returns an error for a bad name, an empty value, or a store failure.
pub fn secret_set(config: &Config, server: &str, var: &str) -> anyhow::Result<()> {
    use std::io::{BufRead, IsTerminal};
    if let Some(problem) = secret_target_problem(server, var) {
        anyhow::bail!("{problem}");
    }
    let value = if std::io::stdin().is_terminal() {
        dialoguer::Password::new()
            .with_prompt(format!("{var} for MCP server '{}'", server.trim()))
            .interact()?
    } else {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        line.trim_end_matches(['\r', '\n']).to_string()
    };
    anyhow::ensure!(
        !value.trim().is_empty(),
        "the value is empty; nothing was stored"
    );
    nanna_config::credentials::SecureStore::new()
        .set(&nanna_config::mcp_secret_key(server, var), &value)
        .context("could not write the secure store")?;
    eprintln!("Stored {var} for MCP server '{}'.", server.trim());
    if let Some(note) = secret_wiring_note(config, server, var) {
        eprintln!("{note}");
    }
    eprintln!("The daemon reads it when it starts the server (restart the daemon to apply).");
    Ok(())
}

/// `nanna mcp secret delete <server> <VAR>`.
///
/// # Errors
///
/// Returns an error for a bad name or a store failure.
pub fn secret_delete(server: &str, var: &str) -> anyhow::Result<()> {
    if let Some(problem) = secret_target_problem(server, var) {
        anyhow::bail!("{problem}");
    }
    nanna_config::credentials::SecureStore::new()
        .delete(&nanna_config::mcp_secret_key(server, var))
        .context("could not remove it from the secure store")?;
    eprintln!("Removed {var} for MCP server '{}'.", server.trim());
    Ok(())
}

#[cfg(test)]
mod secret_tests {
    use super::*;

    #[test]
    fn secret_targets_are_checked_before_the_store_is_touched() {
        assert_eq!(secret_target_problem("github", "GITHUB_TOKEN"), None);
        assert!(secret_target_problem("  ", "GITHUB_TOKEN").is_some());
        assert!(secret_target_problem("github", "GITHUB-TOKEN").is_some());
    }

    #[test]
    fn a_secret_nobody_will_receive_is_pointed_out() {
        let mut config = Config::default();
        assert!(
            secret_wiring_note(&config, "github", "GITHUB_TOKEN")
                .is_some_and(|note| note.contains("no [[mcp.servers]] entry"))
        );
        config.mcp.servers.push(nanna_config::McpServerEntry {
            name: "github".into(),
            command: "npx".into(),
            args: Vec::new(),
            enabled: true,
            secret_env: Vec::new(),
        });
        assert!(
            secret_wiring_note(&config, " github ", "GITHUB_TOKEN")
                .is_some_and(|note| note.contains("does not list GITHUB_TOKEN"))
        );
        config.mcp.servers[0].secret_env.push("GITHUB_TOKEN".into());
        assert_eq!(secret_wiring_note(&config, "github", "GITHUB_TOKEN"), None);
    }
}
