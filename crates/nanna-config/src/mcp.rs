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
//! spawned with the daemon's own environment plus `secret_env`: the *names* of
//! variables whose values live in the secure store, set with
//! `nanna mcp secret set <server> <VAR>`. Only that server's child gets them —
//! unlike exporting the token into the daemon, where every `exec` child would
//! inherit it too.
//!
//! ```toml
//! [[mcp.servers]]
//! name = "github"
//! command = "npx"
//! args = ["-y", "@modelcontextprotocol/server-github"]
//! secret_env = ["GITHUB_PERSONAL_ACCESS_TOKEN"]
//! ```

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
    /// Environment variables this server's process gets from the secure store
    /// (names only; see the module docs).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_env: Vec<String>,
}

/// Longest environment variable name accepted in `secret_env`.
///
/// Not a platform limit (there is none short of the environment block itself);
/// it bounds the secure-store key `mcp.<server>.<VAR>` to something every
/// keyring backend stores, and no real variable name comes close.
pub const MCP_SECRET_ENV_NAME_BYTES_MAX: usize = 128;

/// The secure-store key holding `var` for MCP server `server`. Pure.
#[must_use]
pub fn mcp_secret_key(server: &str, var: &str) -> String {
    format!("mcp.{}.{}", server.trim(), var)
}

/// Whether `var` is a portable environment variable name: ASCII letters,
/// digits and `_`, not starting with a digit. Pure.
///
/// Stricter than POSIX requires on purpose — `=` or NUL would corrupt the
/// child's environment, and anything outside this set is a typo in practice.
#[must_use]
pub fn is_env_var_name(var: &str) -> bool {
    !var.is_empty()
        && var.len() <= MCP_SECRET_ENV_NAME_BYTES_MAX
        && !var.as_bytes()[0].is_ascii_digit()
        && var.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

impl McpServerEntry {
    /// The `secret_env` values for this server, looked up by store key.
    /// Pure given `lookup`, which is the secure store in the daemon.
    ///
    /// All-or-nothing: a server started without a token it was configured to
    /// get fails later with an authentication error that names nothing, so a
    /// missing or malformed entry refuses the start and says which one and how
    /// to set it.
    ///
    /// # Errors
    ///
    /// One sentence naming the first bad name, repeated name, or missing value.
    pub fn resolve_secret_env(
        &self,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Vec<(String, String)>, String> {
        let name = self.name.trim();
        let mut resolved: Vec<(String, String)> = Vec::with_capacity(self.secret_env.len());
        for var in &self.secret_env {
            if !is_env_var_name(var) {
                return Err(format!(
                    "MCP server '{name}': secret_env entry '{var}' is not an environment variable name"
                ));
            }
            if resolved.iter().any(|(seen, _)| seen == var) {
                return Err(format!(
                    "MCP server '{name}': secret_env lists '{var}' twice"
                ));
            }
            match lookup(&mcp_secret_key(name, var)) {
                Some(value) if !value.trim().is_empty() => resolved.push((var.clone(), value)),
                _ => {
                    return Err(format!(
                        "MCP server '{name}' needs {var}, which is not in the secure store — \
                         run `nanna mcp secret set {name} {var}`"
                    ));
                }
            }
        }
        debug_assert_eq!(resolved.len(), self.secret_env.len());
        Ok(resolved)
    }
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
            secret_env: Vec::new(),
        }
    }

    #[test]
    fn secret_env_resolves_from_the_store_or_says_how_to_set_it() {
        let mut github = entry(" github ", "npx");
        github.secret_env = vec!["GITHUB_TOKEN".into(), "GH_HOST".into()];
        let store = |key: &str| match key {
            "mcp.github.GITHUB_TOKEN" => Some("ghp_secret".to_string()),
            "mcp.github.GH_HOST" => Some("github.com".to_string()),
            _ => None,
        };
        assert_eq!(
            github.resolve_secret_env(store),
            Ok(vec![
                ("GITHUB_TOKEN".to_string(), "ghp_secret".to_string()),
                ("GH_HOST".to_string(), "github.com".to_string()),
            ])
        );

        github.secret_env.push("MISSING".into());
        let missing = github.resolve_secret_env(store).expect_err("missing value");
        assert!(
            missing.contains("run `nanna mcp secret set github MISSING`"),
            "{missing}"
        );
        assert!(
            !missing.contains("ghp_secret"),
            "never echo a value: {missing}"
        );

        let blank = |_: &str| Some("  ".to_string());
        github.secret_env = vec!["GITHUB_TOKEN".into()];
        assert!(
            github.resolve_secret_env(blank).is_err(),
            "a blank value is missing"
        );

        github.secret_env = vec!["BAD=NAME".into()];
        assert!(github.resolve_secret_env(store).is_err());
        github.secret_env = vec!["GH_HOST".into(), "GH_HOST".into()];
        assert!(
            github
                .resolve_secret_env(store)
                .expect_err("dup")
                .contains("twice")
        );

        assert_eq!(
            entry("files", "npx").resolve_secret_env(|_| None),
            Ok(Vec::new())
        );
    }

    #[test]
    fn env_var_names_are_the_portable_set() {
        for good in ["GITHUB_TOKEN", "_X", "a1", "X"] {
            assert!(is_env_var_name(good), "{good}");
        }
        let long = "A".repeat(MCP_SECRET_ENV_NAME_BYTES_MAX + 1);
        for bad in ["", "1ABC", "A=B", "A B", "A\0", "ÄPFEL", long.as_str()] {
            assert!(!is_env_var_name(bad), "{bad}");
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
