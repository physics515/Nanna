//! Bridge between Nanna and scripted tools
//!
//! Exposes controlled Nanna functionality to JavaScript code.

use crate::{Result, ScriptError, tool::ToolPermissions};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Bridge providing Nanna capabilities to scripts
#[derive(Clone)]
pub struct NannaBridge {
    pub permissions: ToolPermissions,
    http_client: reqwest::Client,
    logs: Arc<RwLock<Vec<LogEntry>>>,
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
        }
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

        // Determine shell based on OS
        let (shell, shell_arg) = if cfg!(windows) {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };

        let mut cmd = tokio::process::Command::new(shell);
        cmd.arg(shell_arg).arg(command);

        if let Some(wd) = workdir {
            cmd.current_dir(wd);
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
        let path = std::path::Path::new(path);
        
        if !self.permissions.allows_read(path) {
            return Err(ScriptError::Permission(format!(
                "Read access to '{}' not permitted",
                path.display()
            )));
        }

        tokio::fs::read_to_string(path)
            .await
            .map_err(|e| ScriptError::Bridge(format!("Failed to read file: {e}")))
    }

    /// Write a file (if permitted)
    pub async fn write_file(&self, path: &str, content: &str) -> Result<()> {
        let path = std::path::Path::new(path);
        
        if !self.permissions.allows_write(path) {
            return Err(ScriptError::Permission(format!(
                "Write access to '{}' not permitted",
                path.display()
            )));
        }

        tokio::fs::write(path, content)
            .await
            .map_err(|e| ScriptError::Bridge(format!("Failed to write file: {e}")))
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
