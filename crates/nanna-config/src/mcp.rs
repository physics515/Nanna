//! MCP (Model Context Protocol) servers the daemon starts at boot (`[mcp]`).
//!
//! ```toml
//! [[mcp.servers]]
//! name = "files"
//! command = "npx"
//! args = ["-y", "@modelcontextprotocol/server-filesystem", "/home/me/notes"]
//! ```
//!
//! Stdio servers only for now. **There is deliberately no `env` table:**
//! secrets do not live in `config.toml` (they moved to the OS keyring in P1),
//! and the tokens MCP servers want are exactly that kind of value. A server is
//! spawned with the daemon's own environment, so a token it needs is supplied
//! by starting the daemon with it set.

use serde::{Deserialize, Serialize};

/// Most MCP servers started at once.
///
/// Derived from what each one costs: a stdio server is a child process — the
/// common `npx`-launched ones are a Node runtime at roughly 50–100 MB resident —
/// and every tool it offers adds a definition the model may be shown. Sixteen
/// is ~1.5 GB of RAM on a machine that also holds a local model, well past
/// what a personal daemon needs, and the refusal names the extras rather than
/// starting them.
pub const MCP_SERVERS_MAX: usize = 16;

/// The `[mcp]` section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    /// Servers to start, in order.
    pub servers: Vec<McpServerEntry>,
}

/// One stdio MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerEntry {
    /// Unique name. Tools appear to the model as `mcp__{name}__{tool}`.
    pub name: String,
    /// Executable, resolved against `PATH` (e.g. `npx`, `uvx`, `python3`).
    pub command: String,
    /// Arguments passed to `command`.
    #[serde(default)]
    pub args: Vec<String>,
    /// `false` keeps the entry without starting it.
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

const fn enabled_by_default() -> bool {
    true
}

impl McpConfig {
    /// The servers to start, plus one sentence per entry that will not be.
    /// Pure.
    ///
    /// Disabled entries are skipped silently — that is what `enabled = false`
    /// asks for. Everything else that is skipped is named: a blank name or
    /// command, a name already used (tool names are derived from it, so two
    /// servers with one name would shadow each other's tools), and anything
    /// past [`MCP_SERVERS_MAX`].
    #[must_use]
    pub fn startable(&self) -> (Vec<&McpServerEntry>, Vec<String>) {
        let mut start: Vec<&McpServerEntry> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();
        for (index, entry) in self.servers.iter().enumerate() {
            if !entry.enabled {
                continue;
            }
            let name = entry.name.trim();
            if name.is_empty() {
                skipped.push(format!("mcp.servers[{index}] has no name"));
            } else if entry.command.trim().is_empty() {
                skipped.push(format!("MCP server '{name}' has no command"));
            } else if start.iter().any(|kept| kept.name.trim() == name) {
                skipped.push(format!(
                    "MCP server name '{name}' is used twice; the second is not started"
                ));
            } else if start.len() == MCP_SERVERS_MAX {
                skipped.push(format!(
                    "MCP server '{name}' is past the limit of {MCP_SERVERS_MAX} servers and is not started"
                ));
            } else {
                start.push(entry);
            }
        }
        debug_assert!(start.len() <= MCP_SERVERS_MAX);
        debug_assert!(start.len() + skipped.len() <= self.servers.len());
        (start, skipped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, command: &str) -> McpServerEntry {
        McpServerEntry {
            name: name.to_string(),
            command: command.to_string(),
            args: Vec::new(),
            enabled: true,
        }
    }

    #[test]
    fn a_config_file_section_parses_with_defaults() {
        let config: McpConfig = toml::from_str(
            r#"
            [[servers]]
            name = "files"
            command = "npx"
            args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]

            [[servers]]
            name = "off"
            command = "uvx"
            enabled = false
            "#,
        )
        .expect("parses");
        assert_eq!(config.servers.len(), 2);
        assert!(config.servers[0].enabled, "enabled unless said otherwise");
        assert_eq!(config.servers[0].args.len(), 3);
        assert!(!config.servers[1].enabled);
        assert_eq!(McpConfig::default().servers.len(), 0);
    }

    #[test]
    fn only_well_formed_unique_enabled_servers_start() {
        let mut disabled = entry("quiet", "npx");
        disabled.enabled = false;
        let config = McpConfig {
            servers: vec![
                entry("files", "npx"),
                entry(" ", "npx"),
                entry("nocmd", "  "),
                entry("files", "uvx"),
                disabled,
                entry("git", "uvx"),
            ],
        };
        let (start, skipped) = config.startable();
        let names: Vec<&str> = start.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["files", "git"]);
        assert_eq!(skipped.len(), 3, "{skipped:?}");
        assert!(skipped[0].contains("has no name"));
        assert!(skipped[1].contains("'nocmd' has no command"));
        assert!(skipped[2].contains("'files' is used twice"));
    }

    #[test]
    fn servers_past_the_limit_are_named_not_started() {
        let config = McpConfig {
            servers: (0..=MCP_SERVERS_MAX)
                .map(|i| entry(&format!("s{i}"), "npx"))
                .collect(),
        };
        let (start, skipped) = config.startable();
        assert_eq!(start.len(), MCP_SERVERS_MAX);
        assert_eq!(
            skipped,
            [format!(
                "MCP server 's{MCP_SERVERS_MAX}' is past the limit of 16 servers and is not started"
            )]
        );
    }
}
