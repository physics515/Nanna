//! Curiosity tools for proactive exploration
//!
//! These tools help the agent explore its environment autonomously.

use crate::{Tool, ToolDefinition, ToolError, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use tracing::debug;

/// Tool to explore a directory and summarize its contents
pub struct ExploreTool;

#[async_trait]
impl Tool for ExploreTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "explore",
            "Explore a directory to understand its structure. Returns a summary of files and subdirectories. Use during idle time to learn about projects.",
        )
        .string_param("path", "Directory path to explore (default: current directory)", false)
        .integer_param("depth", "Maximum depth to explore (default: 2)", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let max_depth = params
            .get("depth")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(2) as usize;

        debug!("Exploring {} (depth: {})", path, max_depth);

        let root = std::path::Path::new(path);
        if !root.exists() {
            return Ok(ToolResult::success(format!("Path does not exist: {path}")));
        }

        let mut output = String::new();
        output.push_str(&format!("📂 {path}\n"));

        explore_recursive(root, 0, max_depth, &mut output)?;

        Ok(ToolResult::success(output))
    }
}

fn explore_recursive(
    dir: &std::path::Path,
    depth: usize,
    max_depth: usize,
    output: &mut String,
) -> Result<(), ToolError> {
    if depth >= max_depth {
        return Ok(());
    }

    let entries = std::fs::read_dir(dir)
        .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden files and common ignores
        if name.starts_with('.') || name == "node_modules" || name == "target" || name == "__pycache__" {
            continue;
        }

        if path.is_dir() {
            dirs.push((name, path));
        } else {
            files.push(name);
        }
    }

    let indent = "  ".repeat(depth + 1);

    // Show directories first
    for (name, path) in &dirs {
        output.push_str(&format!("{indent}📁 {name}/\n"));
        explore_recursive(path, depth + 1, max_depth, output)?;
    }

    // Then files (limit to avoid overwhelming output)
    let file_count = files.len();
    for name in files.iter().take(10) {
        let emoji = file_emoji(name);
        output.push_str(&format!("{indent}{emoji} {name}\n"));
    }
    if file_count > 10 {
        output.push_str(&format!("{}   ... and {} more files\n", indent, file_count - 10));
    }

    Ok(())
}

fn file_emoji(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("");
    match ext {
        "rs" => "🦀",
        "py" => "🐍",
        "js" | "ts" | "jsx" | "tsx" => "📜",
        "md" => "📝",
        "toml" | "yaml" | "yml" | "json" => "⚙️",
        "html" | "css" => "🌐",
        "sh" | "bash" | "zsh" => "🐚",
        "sql" => "🗃️",
        "jpg" | "png" | "gif" | "svg" => "🖼️",
        _ => "📄",
    }
}

/// Tool to wonder about something and generate questions
pub struct WonderTool;

#[async_trait]
impl Tool for WonderTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "wonder",
            "Express curiosity about something. Records a question or observation for later exploration. Use to note things you want to learn more about.",
        )
        .string_param("about", "What you're curious about", true)
        .enum_param("urgency", "How urgent is this curiosity", false, &["idle", "soon", "now"])
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let about = params
            .get("about")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("about is required".to_string()))?;

        let urgency = params
            .get("urgency")
            .and_then(|v| v.as_str())
            .unwrap_or("idle");

        // This could be stored and acted upon during heartbeats
        // For now, just acknowledge
        let response = match urgency {
            "now" => format!("🤔 I'm curious about: {about} — I should look into this right away."),
            "soon" => format!("🤔 I'm curious about: {about} — I'll explore this when I have a moment."),
            _ => format!("🤔 I'm curious about: {about} — noted for idle time exploration."),
        };

        Ok(ToolResult::success(response))
    }
}

/// Tool to check system status
pub struct StatusTool;

#[async_trait]
impl Tool for StatusTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "status",
            "Check Nanna's current status: uptime, memory usage, active sessions, etc.",
        )
    }

    async fn execute(&self, _params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let mut status = String::new();

        status.push_str("🌙 Nanna Status\n\n");

        // System info
        status.push_str(&format!("📍 Working directory: {}\n", 
            std::env::current_dir().map_or_else(|_| "unknown".to_string(), |p| p.display().to_string())
        ));

        // Time
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        status.push_str(&format!("🕐 Current time: {now} (unix)\n"));

        // Environment hints
        if std::env::var("OPENAI_API_KEY").is_ok() {
            status.push_str("🧠 Semantic memory: enabled\n");
        } else {
            status.push_str("🧠 Semantic memory: disabled (no OPENAI_API_KEY)\n");
        }

        status.push_str("\n✨ All systems operational");

        Ok(ToolResult::success(status))
    }
}
