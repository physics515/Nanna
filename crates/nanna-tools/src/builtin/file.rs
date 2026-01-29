//! File operation tools

use crate::{Tool, ToolDefinition, ToolError, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;
use tracing::debug;

/// Read file contents
pub struct ReadFileTool {
    /// Base directory for file operations (sandbox)
    pub base_dir: Option<String>,
    /// Maximum file size to read (bytes)
    pub max_size: usize,
}

impl ReadFileTool {
    #[must_use] 
    pub const fn new() -> Self {
        Self {
            base_dir: None,
            max_size: 10 * 1024 * 1024, // 10MB default
        }
    }

    pub fn with_base_dir(mut self, dir: impl Into<String>) -> Self {
        self.base_dir = Some(dir.into());
        self
    }

    fn resolve_path(&self, path: &str) -> Result<std::path::PathBuf, ToolError> {
        let path = Path::new(path);
        
        if let Some(ref base) = self.base_dir {
            let base = Path::new(base);
            let resolved = if path.is_absolute() {
                // Check if absolute path is within base
                let canonical = path.canonicalize().map_err(|e| {
                    ToolError::InvalidParams(format!("Invalid path: {e}"))
                })?;
                if !canonical.starts_with(base) {
                    return Err(ToolError::PermissionDenied(
                        "Path outside sandbox".to_string(),
                    ));
                }
                canonical
            } else {
                base.join(path)
            };
            Ok(resolved)
        } else {
            Ok(path.to_path_buf())
        }
    }
}

impl Default for ReadFileTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("read_file", "Read the contents of a file")
            .string_param("path", "Path to the file to read", true)
            .int_param("offset", "Line number to start reading from (1-indexed)", false)
            .int_param("limit", "Maximum number of lines to read", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let path_str = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'path' parameter".to_string()))?;

        let path = self.resolve_path(path_str)?;
        debug!("Reading file: {:?}", path);

        // Check file size
        let metadata = fs::metadata(&path).await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to read file metadata: {e}"))
        })?;

        if metadata.len() as usize > self.max_size {
            return Err(ToolError::ExecutionFailed(format!(
                "File too large: {} bytes (max: {} bytes)",
                metadata.len(),
                self.max_size
            )));
        }

        let content = fs::read_to_string(&path).await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to read file: {e}"))
        })?;

        // Handle offset and limit
        let offset = params
            .get("offset")
            .and_then(serde_json::Value::as_u64)
            .map_or(0, |v| v.saturating_sub(1) as usize);

        let limit = params
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .map(|v| v as usize);

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        let selected: Vec<&str> = if let Some(limit) = limit {
            lines.into_iter().skip(offset).take(limit).collect()
        } else {
            lines.into_iter().skip(offset).collect()
        };

        let result_content = selected.join("\n");

        Ok(ToolResult::success(result_content).with_data(serde_json::json!({
            "path": path_str,
            "total_lines": total_lines,
            "offset": offset + 1,
            "lines_returned": selected.len(),
        })))
    }
}

/// Write content to a file
pub struct WriteFileTool {
    pub base_dir: Option<String>,
}

impl WriteFileTool {
    #[must_use] 
    pub const fn new() -> Self {
        Self { base_dir: None }
    }

    pub fn with_base_dir(mut self, dir: impl Into<String>) -> Self {
        self.base_dir = Some(dir.into());
        self
    }

    fn resolve_path(&self, path: &str) -> Result<std::path::PathBuf, ToolError> {
        let path = Path::new(path);
        
        if let Some(ref base) = self.base_dir {
            let base = Path::new(base);
            if path.is_absolute() {
                return Err(ToolError::PermissionDenied(
                    "Absolute paths not allowed in sandbox mode".to_string(),
                ));
            }
            Ok(base.join(path))
        } else {
            Ok(path.to_path_buf())
        }
    }
}

impl Default for WriteFileTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("write_file", "Write content to a file (creates directories as needed)")
            .string_param("path", "Path to the file to write", true)
            .string_param("content", "Content to write to the file", true)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let path_str = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'path' parameter".to_string()))?;

        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'content' parameter".to_string()))?;

        let path = self.resolve_path(path_str)?;
        debug!("Writing file: {:?}", path);

        // Create parent directories
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to create directories: {e}"))
            })?;
        }

        fs::write(&path, content).await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to write file: {e}"))
        })?;

        Ok(ToolResult::success(format!(
            "Successfully wrote {} bytes to {}",
            content.len(),
            path_str
        )))
    }
}

/// List directory contents
pub struct ListDirTool {
    pub base_dir: Option<String>,
}

impl ListDirTool {
    #[must_use] 
    pub const fn new() -> Self {
        Self { base_dir: None }
    }

    pub fn with_base_dir(mut self, dir: impl Into<String>) -> Self {
        self.base_dir = Some(dir.into());
        self
    }
}

impl Default for ListDirTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ListDirTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("list_dir", "List contents of a directory")
            .string_param("path", "Path to the directory to list", true)
            .bool_param("recursive", "List recursively (default: false)", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let path_str = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'path' parameter".to_string()))?;

        let recursive = params
            .get("recursive")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        let path = Path::new(path_str);
        debug!("Listing directory: {:?}", path);

        if !path.exists() {
            return Err(ToolError::ExecutionFailed(format!(
                "Directory does not exist: {path_str}"
            )));
        }

        let mut entries = Vec::new();

        if recursive {
            for entry in walkdir::WalkDir::new(path).max_depth(10) {
                match entry {
                    Ok(e) => {
                        let entry_path = e.path().strip_prefix(path).unwrap_or(e.path());
                        let file_type = if e.file_type().is_dir() {
                            "dir"
                        } else if e.file_type().is_symlink() {
                            "link"
                        } else {
                            "file"
                        };
                        entries.push(format!("[{}] {}", file_type, entry_path.display()));
                    }
                    Err(e) => {
                        entries.push(format!("[error] {e}"));
                    }
                }
            }
        } else {
            let mut dir = fs::read_dir(path).await.map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to read directory: {e}"))
            })?;

            while let Some(entry) = dir.next_entry().await.map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to read entry: {e}"))
            })? {
                let file_type = entry.file_type().await.map_or("unknown", |ft| {
                    if ft.is_dir() {
                        "dir"
                    } else if ft.is_symlink() {
                        "link"
                    } else {
                        "file"
                    }
                });
                entries.push(format!("[{}] {}", file_type, entry.file_name().to_string_lossy()));
            }
        }

        entries.sort();
        Ok(ToolResult::success(entries.join("\n")).with_data(serde_json::json!({
            "path": path_str,
            "count": entries.len(),
            "recursive": recursive,
        })))
    }
}
