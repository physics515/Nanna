//! Scripted tool definitions

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// A user-authored tool written in JavaScript/TypeScript
#[derive(Debug, Clone)]
pub struct ScriptedTool {
    /// Unique tool name
    pub name: String,
    /// Source code (JS or TS)
    pub source: String,
    /// Whether the source is TypeScript
    pub is_typescript: bool,
    /// Source file path (for error messages)
    pub source_path: Option<PathBuf>,
    /// Permissions granted to this tool
    pub permissions: ToolPermissions,
    /// Execution timeout in milliseconds
    pub timeout_ms: u64,
}

impl ScriptedTool {
    /// Create a new scripted tool from source code
    pub fn new(name: impl Into<String>, source: impl Into<String>) -> Self {
        let source = source.into();
        let name = name.into();
        let is_typescript = name.ends_with(".ts") || source.contains(": string") || source.contains(": number");
        
        Self {
            name,
            source,
            is_typescript,
            source_path: None,
            permissions: ToolPermissions::default(),
            timeout_ms: 30_000,
        }
    }

    /// Create from a file path
    pub fn from_file(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        let source = std::fs::read_to_string(&path)?;
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unnamed".to_string());
        let is_typescript = path.extension().map_or(false, |e| e == "ts" || e == "tsx");

        Ok(Self {
            name,
            source,
            is_typescript,
            source_path: Some(path),
            permissions: ToolPermissions::default(),
            timeout_ms: 30_000,
        })
    }

    /// Set permissions
    #[must_use]
    pub fn with_permissions(mut self, permissions: ToolPermissions) -> Self {
        self.permissions = permissions;
        self
    }

    /// Set timeout
    #[must_use]
    pub const fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    /// Mark as TypeScript
    #[must_use]
    pub const fn typescript(mut self, is_ts: bool) -> Self {
        self.is_typescript = is_ts;
        self
    }
}

/// Permissions for scripted tools
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolPermissions {
    /// Allowed network hosts (empty = none)
    pub net: Vec<String>,
    /// Allowed read paths (empty = none)
    pub read: Vec<PathBuf>,
    /// Allowed write paths (empty = none)
    pub write: Vec<PathBuf>,
    /// Allow environment variable access
    pub env: bool,
    /// Allow subprocess execution
    pub run: bool,
}

impl ToolPermissions {
    /// No permissions (fully sandboxed)
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Allow network access to specific hosts
    #[must_use]
    pub fn with_net(mut self, hosts: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.net = hosts.into_iter().map(Into::into).collect();
        self
    }

    /// Allow reading from specific paths
    #[must_use]
    pub fn with_read(mut self, paths: impl IntoIterator<Item = impl Into<PathBuf>>) -> Self {
        self.read = paths.into_iter().map(Into::into).collect();
        self
    }

    /// Allow writing to specific paths
    #[must_use]
    pub fn with_write(mut self, paths: impl IntoIterator<Item = impl Into<PathBuf>>) -> Self {
        self.write = paths.into_iter().map(Into::into).collect();
        self
    }

    /// Allow environment variable access
    #[must_use]
    pub const fn with_env(mut self) -> Self {
        self.env = true;
        self
    }

    /// Check if network access to a host is allowed
    pub fn allows_net(&self, host: &str) -> bool {
        self.net.iter().any(|h| h == "*" || h == host || host.ends_with(&format!(".{h}")))
    }

    /// Check if reading a path is allowed
    pub fn allows_read(&self, path: &std::path::Path) -> bool {
        self.read.iter().any(|p| path.starts_with(p))
    }

    /// Check if writing a path is allowed
    pub fn allows_write(&self, path: &std::path::Path) -> bool {
        self.write.iter().any(|p| path.starts_with(p))
    }
}

/// Where a tool's output should be routed after execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OutputTarget {
    /// Large results are chunked and stored in memory; a stub replaces them in context (default).
    #[default]
    Memory,
    /// Results always stay in context. Large results are summarized (or truncated as fallback).
    Context,
}

/// Tool manifest extracted from source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolManifest {
    /// Tool name
    pub name: String,
    /// Tool description
    pub description: Option<String>,
    /// Input parameter schema (JSON Schema)
    pub parameters: Option<Value>,
    /// Where tool output should be routed
    #[serde(default)]
    pub output: OutputTarget,
}

/// Extract manifest from tool source (looks for default export)
pub fn extract_manifest(source: &str) -> Option<ToolManifest> {
    // Simple regex-free extraction for common patterns
    // export default { name: "...", description: "...", ... }
    
    let name = extract_string_field(source, "name")?;
    let description = extract_string_field(source, "description");
    let output = match extract_string_field(source, "output").as_deref() {
        Some("context") => OutputTarget::Context,
        _ => OutputTarget::Memory,
    };

    Some(ToolManifest {
        name,
        description,
        parameters: None, // TODO: Parse parameters schema
        output,
    })
}

fn extract_string_field(source: &str, field: &str) -> Option<String> {
    // Match: name: "value" or name: 'value'
    let patterns = [
        format!(r#"{field}: ""#),
        format!(r#"{field}: '"#),
        format!(r#"{field}:""#),
        format!(r#"{field}:'"#),
    ];

    for pattern in &patterns {
        if let Some(start) = source.find(pattern) {
            let quote = if pattern.ends_with('"') { '"' } else { '\'' };
            let value_start = start + pattern.len();
            if let Some(end) = source[value_start..].find(quote) {
                return Some(source[value_start..value_start + end].to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_manifest() {
        let source = r#"
            export default {
                name: "greet",
                description: "Greet someone by name",
                execute({ name }) {
                    return `Hello, ${name}!`;
                }
            }
        "#;

        let manifest = extract_manifest(source).unwrap();
        assert_eq!(manifest.name, "greet");
        assert_eq!(manifest.description, Some("Greet someone by name".to_string()));
    }

    #[test]
    fn test_extract_manifest_with_output_context() {
        let source = r#"
            export default {
                name: "recall",
                description: "Search memory",
                output: "context",
                execute({ query }) {
                    return "results";
                }
            }
        "#;

        let manifest = extract_manifest(source).unwrap();
        assert_eq!(manifest.name, "recall");
        assert_eq!(manifest.output, OutputTarget::Context);
    }

    #[test]
    fn test_extract_manifest_output_defaults_to_memory() {
        let source = r#"
            export default {
                name: "exec",
                description: "Run a command",
                execute({ command }) {
                    return "done";
                }
            }
        "#;

        let manifest = extract_manifest(source).unwrap();
        assert_eq!(manifest.output, OutputTarget::Memory);
    }

    #[test]
    fn test_permissions() {
        let perms = ToolPermissions::none()
            .with_net(["api.example.com", "*.github.com"])
            .with_read(["/tmp"]);

        assert!(perms.allows_net("api.example.com"));
        assert!(!perms.allows_net("evil.com"));
        assert!(perms.allows_read(std::path::Path::new("/tmp/file.txt")));
        assert!(!perms.allows_read(std::path::Path::new("/etc/passwd")));
    }
}
