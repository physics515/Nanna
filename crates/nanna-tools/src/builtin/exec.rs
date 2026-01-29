//! Shell execution tool

use crate::{Tool, ToolDefinition, ToolError, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use tokio::process::Command;
use tracing::debug;

/// Execute shell commands
pub struct ExecTool {
    /// Working directory for commands
    pub workdir: Option<String>,
    /// Allow list of commands (None = allow all)
    pub allowlist: Option<Vec<String>>,
    /// Deny list of commands
    pub denylist: Vec<String>,
}

impl ExecTool {
    #[must_use] 
    pub fn new() -> Self {
        Self {
            workdir: None,
            allowlist: None,
            denylist: vec![
                "rm -rf /".to_string(),
                "format".to_string(),
                "mkfs".to_string(),
                "dd if=/dev/zero".to_string(),
            ],
        }
    }

    pub fn with_workdir(mut self, workdir: impl Into<String>) -> Self {
        self.workdir = Some(workdir.into());
        self
    }
}

impl Default for ExecTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ExecTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "exec",
            "Execute a shell command and return the output. Use with caution.",
        )
        .string_param("command", "The shell command to execute", true)
        .string_param("workdir", "Working directory (optional)", false)
        .int_param("timeout", "Timeout in seconds (default: 30)", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'command' parameter".to_string()))?;

        // Check denylist
        for denied in &self.denylist {
            if command.contains(denied) {
                return Err(ToolError::PermissionDenied(format!(
                    "Command contains denied pattern: {denied}"
                )));
            }
        }

        // Check allowlist if set
        if let Some(ref allowlist) = self.allowlist {
            let allowed = allowlist.iter().any(|a| command.starts_with(a));
            if !allowed {
                return Err(ToolError::PermissionDenied(
                    "Command not in allowlist".to_string(),
                ));
            }
        }

        let workdir = params
            .get("workdir")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string)
            .or_else(|| self.workdir.clone());

        let timeout_secs = params
            .get("timeout")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(30);

        debug!("Executing command: {}", command);

        // Determine shell based on OS
        let (shell, shell_arg) = if cfg!(windows) {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };

        let mut cmd = Command::new(shell);
        cmd.arg(shell_arg).arg(command);

        if let Some(ref wd) = workdir {
            cmd.current_dir(wd);
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            cmd.output(),
        )
        .await
        .map_err(|_| ToolError::Timeout)?
        .map_err(ToolError::Io)?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let result = if output.status.success() {
            let content = if stderr.is_empty() {
                stdout.to_string()
            } else {
                format!("{stdout}\n\nStderr:\n{stderr}")
            };
            ToolResult::success(content)
        } else {
            let error_msg = if stderr.is_empty() {
                format!("Command failed with exit code: {:?}", output.status.code())
            } else {
                stderr.to_string()
            };
            ToolResult {
                success: false,
                content: stdout.to_string(),
                error: Some(error_msg),
                data: None,
            }
        };

        Ok(result)
    }

    fn requires_elevation(&self) -> bool {
        false // Could be true for certain commands
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(60) // Longer timeout for exec
    }
}
