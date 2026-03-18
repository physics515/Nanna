//! Bridge between Nanna and scripted tools
//!
//! Exposes controlled Nanna functionality to JavaScript code.

use crate::{Result, ScriptError, tool::ToolPermissions};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

/// A service function callable from scripts via `Nanna.service(name, params)`.
///
/// Each closure receives a JSON `Value` of params and returns a JSON `Value` result.
pub type ServiceFn = Arc<
    dyn Fn(Value) -> Pin<Box<dyn Future<Output = std::result::Result<Value, String>> + Send>>
        + Send
        + Sync,
>;

/// Bridge providing Nanna capabilities to scripts
#[derive(Clone)]
pub struct NannaBridge {
    pub permissions: ToolPermissions,
    http_client: reqwest::Client,
    logs: Arc<RwLock<Vec<LogEntry>>>,
    /// Tool definitions for `Nanna.listTools()` — set by the script engine when available
    tool_definitions: Option<Value>,
    /// Service functions callable via `Nanna.service(name, params)`
    services: Arc<HashMap<String, ServiceFn>>,
    /// Default working directory for exec commands (overrides home dir fallback)
    default_workdir: Option<PathBuf>,
}

impl NannaBridge {
    /// Create a new bridge with the given permissions
    pub fn new(permissions: ToolPermissions) -> Self {
        Self {
            permissions,
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            logs: Arc::new(RwLock::new(Vec::new())),
            tool_definitions: None,
            services: Arc::new(HashMap::new()),
            default_workdir: None,
        }
    }

    /// Set the default working directory for exec commands.
    /// When set, this overrides the home directory fallback.
    #[must_use]
    pub fn with_default_workdir(mut self, workdir: impl Into<PathBuf>) -> Self {
        self.default_workdir = Some(workdir.into());
        self
    }

    /// Set tool definitions for `Nanna.listTools()`
    #[must_use]
    pub fn with_tool_definitions(mut self, defs: Value) -> Self {
        self.tool_definitions = Some(defs);
        self
    }

    /// Resolve a path: expand `~` to home directory, resolve relative paths against
    /// the default working directory (if set) or home dir.
    /// Works on Windows, macOS, Linux, Android, and iOS.
    /// Falls back to the raw path if no base directory can be determined.
    fn resolve_path_with_workdir(path: &str, default_workdir: Option<&Path>) -> PathBuf {
        let path = path.trim();
        
        // Expand ~ or ~/ (but not ~username which we don't support)
        if path == "~" || path.starts_with("~/") || path.starts_with("~\\") {
            if let Some(home) = Self::home_dir() {
                let rest = path.strip_prefix("~/")
                    .or_else(|| path.strip_prefix("~\\"))
                    .unwrap_or("");
                return home.join(rest);
            }
        }
        
        let p = Path::new(path);
        if p.is_relative() && !path.is_empty() {
            // Resolve relative to workspace working directory, then home dir
            if let Some(wd) = default_workdir {
                return wd.join(p);
            }
            if let Some(home) = Self::home_dir() {
                return home.join(p);
            }
        }
        p.to_path_buf()
    }

    /// Resolve a path using the instance's default working directory.
    fn resolve_path(&self, path: &str) -> PathBuf {
        Self::resolve_path_with_workdir(path, self.default_workdir.as_deref())
    }

    /// Get the user's home directory, cross-platform.
    /// Uses `directories` crate which supports Windows, macOS, Linux.
    /// On Android/iOS, falls back to current directory.
    fn home_dir() -> Option<PathBuf> {
        directories::BaseDirs::new()
            .map(|d| d.home_dir().to_path_buf())
            .or_else(|| std::env::current_dir().ok())
    }

    /// Set service functions for `Nanna.service()`
    #[must_use]
    pub fn with_services(mut self, services: HashMap<String, ServiceFn>) -> Self {
        self.services = Arc::new(services);
        self
    }

    /// Get stored tool definitions (for `Nanna.listTools()`)
    pub fn list_tools(&self) -> Option<&Value> {
        self.tool_definitions.as_ref()
    }

    /// Call a registered service by name
    pub async fn call_service(&self, name: &str, params: Value) -> std::result::Result<Value, String> {
        let service = self.services.get(name)
            .ok_or_else(|| format!("Service not found: {name}"))?;
        service(params).await
    }

    /// Execute a shell command (if permitted)
    pub async fn exec(&self, command: &str, workdir: Option<&str>) -> Result<ExecResponse> {
        tracing::debug!(target: "bridge", "exec called: command={}, run_permission={}", command, self.permissions.run);
        
        if !self.permissions.run {
            tracing::warn!(target: "bridge", "Shell execution denied - run permission is false");
            return Err(ScriptError::Permission(
                "Shell execution not permitted. Tool needs 'run' permission.".to_string(),
            ));
        }

        // On mobile platforms, shell exec is not available
        if cfg!(target_os = "android") || cfg!(target_os = "ios") {
            return Err(ScriptError::Permission(
                "Shell execution is not available on mobile platforms".to_string(),
            ));
        }

        // Determine shell based on OS
        let (shell, shell_arg) = if cfg!(windows) {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };

        let mut cmd = tokio::process::Command::new(shell);
        cmd.arg(shell_arg).arg(command);

        if let Some(wd) = workdir {
            cmd.current_dir(self.resolve_path(wd));
        } else if let Some(ref wd) = self.default_workdir {
            cmd.current_dir(wd);
        } else if let Some(home) = Self::home_dir() {
            cmd.current_dir(home);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            cmd.output(),
        )
        .await
        .map_err(|_| ScriptError::Timeout(30_000))?
        .map_err(|e| ScriptError::Bridge(format!("Failed to execute command: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        Ok(ExecResponse {
            success: output.status.success(),
            code: output.status.code(),
            stdout,
            stderr,
        })
    }

    /// Get the current platform (windows, darwin, linux)
    #[must_use]
    pub fn platform() -> &'static str {
        if cfg!(windows) {
            "win32"
        } else if cfg!(target_os = "macos") {
            "darwin"
        } else if cfg!(target_os = "android") {
            "android"
        } else if cfg!(target_os = "ios") {
            "ios"
        } else {
            "linux"
        }
    }

    /// Fetch a URL (if permitted)
    pub async fn fetch(&self, url: &str, options: Option<FetchOptions>) -> Result<FetchResponse> {
        // Parse URL to check host
        let parsed = url::Url::parse(url)
            .map_err(|e| ScriptError::Bridge(format!("Invalid URL: {e}")))?;
        
        let host = parsed.host_str().unwrap_or("");
        
        if !self.permissions.allows_net(host) {
            return Err(ScriptError::Permission(format!(
                "Network access to '{host}' not permitted"
            )));
        }

        let options = options.unwrap_or_default();
        
        let mut request = match options.method.as_deref().unwrap_or("GET") {
            "GET" => self.http_client.get(url),
            "POST" => self.http_client.post(url),
            "PUT" => self.http_client.put(url),
            "DELETE" => self.http_client.delete(url),
            "PATCH" => self.http_client.patch(url),
            m => return Err(ScriptError::Bridge(format!("Unsupported method: {m}"))),
        };

        // Add headers
        if let Some(headers) = options.headers {
            for (key, value) in headers {
                request = request.header(&key, &value);
            }
        }

        // Add body
        if let Some(body) = options.body {
            request = request.body(body);
        }

        let response = request
            .send()
            .await
            .map_err(|e| ScriptError::Bridge(format!("Fetch failed: {e}")))?;

        let status = response.status().as_u16();
        let headers: std::collections::HashMap<String, String> = response
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
            .collect();
        
        let body = response
            .text()
            .await
            .map_err(|e| ScriptError::Bridge(format!("Failed to read body: {e}")))?;

        Ok(FetchResponse {
            status,
            headers,
            body,
        })
    }

    /// Read a file (if permitted)
    pub async fn read_file(&self, path: &str) -> Result<String> {
        let path = self.resolve_path(path);
        
        if !self.permissions.allows_read(&path) {
            return Err(ScriptError::Permission(format!(
                "Read access to '{}' not permitted",
                path.display()
            )));
        }

        tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| ScriptError::Bridge(format!("Failed to read '{}': {e}", path.display())))
    }

    /// Write a file (if permitted)
    pub async fn write_file(&self, path: &str, content: &str) -> Result<()> {
        let path = self.resolve_path(path);
        
        if !self.permissions.allows_write(&path) {
            return Err(ScriptError::Permission(format!(
                "Write access to '{}' not permitted",
                path.display()
            )));
        }

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
        }

        tokio::fs::write(&path, content)
            .await
            .map_err(|e| ScriptError::Bridge(format!("Failed to write '{}': {e}", path.display())))
    }

    /// Log a message
    pub async fn log(&self, level: LogLevel, message: String) {
        let entry = LogEntry {
            level,
            message,
            timestamp: chrono::Utc::now(),
        };

        // Also emit to tracing
        match entry.level {
            LogLevel::Debug => tracing::debug!(target: "script", "{}", entry.message),
            LogLevel::Info => tracing::info!(target: "script", "{}", entry.message),
            LogLevel::Warn => tracing::warn!(target: "script", "{}", entry.message),
            LogLevel::Error => tracing::error!(target: "script", "{}", entry.message),
        }

        self.logs.write().await.push(entry);
    }

    /// List directory contents (if permitted)
    pub async fn list_dir(&self, path: &str, recursive: bool) -> Result<Vec<DirEntry>> {
        let path = self.resolve_path(path);

        if !self.permissions.allows_read(&path) {
            return Err(ScriptError::Permission(format!(
                "Read access to '{}' not permitted",
                path.display()
            )));
        }

        const IGNORE_DIRS: &[&str] = &[
            "node_modules", "target", ".git", "__pycache__", ".venv",
            "venv", "dist", "build", ".next", ".nuxt", ".cache",
        ];

        let mut entries = Vec::new();

        if recursive {
            for result in walkdir::WalkDir::new(&path)
                .max_depth(10)
                .into_iter()
                .filter_entry(|e| {
                    e.file_name()
                        .to_str()
                        .map_or(true, |name| !IGNORE_DIRS.contains(&name))
                })
            {
                let entry = match result {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                // Skip the root directory itself
                if entry.depth() == 0 {
                    continue;
                }
                if let Some(de) = dir_entry_from_walkdir(&entry) {
                    entries.push(de);
                }
            }
        } else {
            let mut read_dir = tokio::fs::read_dir(&path)
                .await
                .map_err(|e| ScriptError::Bridge(format!("Failed to read directory: {e}")))?;

            while let Some(entry) = read_dir
                .next_entry()
                .await
                .map_err(|e| ScriptError::Bridge(format!("Failed to read entry: {e}")))?
            {
                let name = entry.file_name().to_string_lossy().to_string();
                let metadata = entry.metadata().await.ok();
                let entry_type = metadata.as_ref().map_or("unknown", |m| {
                    if m.is_dir() { "dir" } else if m.is_symlink() { "link" } else { "file" }
                }).to_string();
                let size = metadata.as_ref().map_or(0, |m| m.len());
                let modified = metadata
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());

                entries.push(DirEntry {
                    name,
                    entry_type,
                    size,
                    modified,
                });
            }
        }

        Ok(entries)
    }

    /// Get file/directory metadata (if permitted)
    pub async fn stat(&self, path: &str) -> Result<FileStat> {
        let path = self.resolve_path(path);

        if !self.permissions.allows_read(&path) {
            return Err(ScriptError::Permission(format!(
                "Read access to '{}' not permitted",
                path.display()
            )));
        }

        let metadata = tokio::fs::metadata(&path)
            .await
            .map_err(|e| ScriptError::Bridge(format!("Failed to stat '{}': {e}", path.display())))?;

        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());

        Ok(FileStat {
            size: metadata.len(),
            is_file: metadata.is_file(),
            is_dir: metadata.is_dir(),
            modified,
        })
    }

    /// Get environment variable (if permitted)
    pub fn get_env(&self, key: &str) -> Result<Option<String>> {
        if !self.permissions.env {
            return Err(ScriptError::Permission(
                "Environment variable access not permitted".to_string(),
            ));
        }
        Ok(std::env::var(key).ok())
    }

    /// Get all logs from this execution
    pub async fn get_logs(&self) -> Vec<LogEntry> {
        self.logs.read().await.clone()
    }

    /// Clear logs
    pub async fn clear_logs(&self) {
        self.logs.write().await.clear();
    }
}

/// Exec response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResponse {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Fetch request options
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FetchOptions {
    pub method: Option<String>,
    pub headers: Option<std::collections::HashMap<String, String>>,
    pub body: Option<String>,
}

/// Fetch response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchResponse {
    pub status: u16,
    pub headers: std::collections::HashMap<String, String>,
    pub body: String,
}

impl FetchResponse {
    /// Parse body as JSON
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_str(&self.body).map_err(Into::into)
    }
}

/// Directory entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub entry_type: String,
    pub size: u64,
    pub modified: Option<u64>,
}

/// File stat result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileStat {
    pub size: u64,
    pub is_file: bool,
    pub is_dir: bool,
    pub modified: Option<u64>,
}

/// Convert a walkdir entry to our DirEntry
fn dir_entry_from_walkdir(entry: &walkdir::DirEntry) -> Option<DirEntry> {
    let name = entry.path().to_string_lossy().to_string();
    let metadata = entry.metadata().ok()?;
    let entry_type = if metadata.is_dir() {
        "dir"
    } else if metadata.is_symlink() {
        "link"
    } else {
        "file"
    }
    .to_string();
    let size = metadata.len();
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());

    Some(DirEntry {
        name,
        entry_type,
        size,
        modified,
    })
}

/// Log level
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// Log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub level: LogLevel,
    pub message: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_check() {
        let bridge = NannaBridge::new(
            ToolPermissions::none()
                .with_net(["api.example.com"])
                .with_read(["/tmp"]),
        );

        assert!(bridge.permissions.allows_net("api.example.com"));
        assert!(!bridge.permissions.allows_net("evil.com"));
    }
}
