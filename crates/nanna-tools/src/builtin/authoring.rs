//! Tool authoring - allows the agent to create its own tools
//!
//! Tools are stored as script files and dynamically loaded.

use crate::{Tool, ToolDefinition, ToolError, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Script-based tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptTool {
    /// Tool name
    pub name: String,
    /// Description
    pub description: String,
    /// Parameters schema
    pub parameters: Vec<ScriptToolParam>,
    /// Script content (shell/python/etc)
    pub script: String,
    /// Script type (bash, python, powershell)
    pub script_type: String,
    /// Working directory for script execution
    pub workdir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptToolParam {
    pub name: String,
    pub description: String,
    pub param_type: String, // "string", "number", "boolean"
    pub required: bool,
}

/// Store for user-created tools
#[derive(Debug, Default)]
pub struct ToolStore {
    tools: RwLock<HashMap<String, ScriptTool>>,
    storage_path: Option<PathBuf>,
}

impl ToolStore {
    /// Create a new tool store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a tool store with persistence.
    #[must_use]
    pub fn with_storage(path: PathBuf) -> Self {
        Self {
            tools: RwLock::new(HashMap::new()),
            storage_path: Some(path),
        }
    }

    /// Add a tool to the store.
    pub async fn add(&self, tool: ScriptTool) -> Result<(), ToolError> {
        let name = tool.name.clone();
        self.tools.write().await.insert(name.clone(), tool);
        info!("Added user tool: {}", name);

        // Persist if storage path is set
        if let Some(ref path) = self.storage_path {
            self.save(path).await?;
        }

        Ok(())
    }

    /// Remove a tool from the store.
    pub async fn remove(&self, name: &str) -> Result<(), ToolError> {
        let mut tools = self.tools.write().await;
        if tools.remove(name).is_some() {
            info!("Removed user tool: {}", name);
            drop(tools);

            if let Some(ref path) = self.storage_path {
                self.save(path).await?;
            }
            Ok(())
        } else {
            Err(ToolError::NotFound(format!("Tool not found: {}", name)))
        }
    }

    /// Get a tool by name.
    pub async fn get(&self, name: &str) -> Option<ScriptTool> {
        self.tools.read().await.get(name).cloned()
    }

    /// List all tools.
    pub async fn list(&self) -> Vec<ScriptTool> {
        self.tools.read().await.values().cloned().collect()
    }

    /// Save tools to file.
    async fn save(&self, path: &PathBuf) -> Result<(), ToolError> {
        let tools = self.tools.read().await;
        let json = serde_json::to_string_pretty(&*tools)
            .map_err(|e| ToolError::ExecutionFailed(format!("Serialization error: {}", e)))?;

        tokio::fs::write(path, json)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to save tools: {}", e)))?;

        debug!("Saved {} tools to {:?}", tools.len(), path);
        Ok(())
    }

    /// Load tools from file.
    pub async fn load(&self, path: &PathBuf) -> Result<(), ToolError> {
        if !path.exists() {
            return Ok(());
        }

        let json = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read tools: {}", e)))?;

        let loaded: HashMap<String, ScriptTool> = serde_json::from_str(&json)
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to parse tools: {}", e)))?;

        let count = loaded.len();
        *self.tools.write().await = loaded;
        info!("Loaded {} user tools from {:?}", count, path);
        Ok(())
    }
}

/// Tool for creating new tools
pub struct CreateToolTool {
    store: Arc<ToolStore>,
}

impl CreateToolTool {
    #[must_use]
    pub fn new(store: Arc<ToolStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for CreateToolTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("create_tool", "Create a new custom tool (script-based)")
            .string_param("name", "Tool name (snake_case)", true)
            .string_param("description", "What the tool does", true)
            .string_param("script", "Script content (bash/python/powershell)", true)
            .string_param("script_type", "Script type: bash, python, powershell", true)
            .string_param("parameters", "JSON array of parameters [{name, description, type, required}]", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'name'".to_string()))?;

        let description = params
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'description'".to_string()))?;

        let script = params
            .get("script")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'script'".to_string()))?;

        let script_type = params
            .get("script_type")
            .and_then(|v| v.as_str())
            .unwrap_or("bash");

        // Validate script type
        if !["bash", "python", "powershell", "sh"].contains(&script_type) {
            return Err(ToolError::InvalidParams(format!(
                "Invalid script_type: {}. Use bash, python, or powershell",
                script_type
            )));
        }

        // Parse parameters if provided
        let parameters: Vec<ScriptToolParam> = params
            .get("parameters")
            .and_then(|v| {
                if let Some(s) = v.as_str() {
                    serde_json::from_str(s).ok()
                } else if v.is_array() {
                    serde_json::from_value(v.clone()).ok()
                } else {
                    None
                }
            })
            .unwrap_or_default();

        let tool = ScriptTool {
            name: name.to_string(),
            description: description.to_string(),
            parameters,
            script: script.to_string(),
            script_type: script_type.to_string(),
            workdir: None,
        };

        self.store.add(tool).await?;

        Ok(ToolResult::success(format!(
            "Created tool '{}'. It will be available after restart or reload.",
            name
        )))
    }
}

/// Tool for listing user-created tools
pub struct ListToolsTool {
    store: Arc<ToolStore>,
}

impl ListToolsTool {
    #[must_use]
    pub fn new(store: Arc<ToolStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for ListToolsTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("list_custom_tools", "List all custom tools created by the agent")
    }

    async fn execute(&self, _params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let tools = self.store.list().await;

        if tools.is_empty() {
            return Ok(ToolResult::success("No custom tools have been created yet."));
        }

        let mut output = format!("Custom tools ({}):\n\n", tools.len());
        for tool in &tools {
            output.push_str(&format!(
                "• {} ({})\n  {}\n",
                tool.name, tool.script_type, tool.description
            ));
        }

        Ok(ToolResult::success(output))
    }
}

/// Tool for deleting user-created tools
pub struct DeleteToolTool {
    store: Arc<ToolStore>,
}

impl DeleteToolTool {
    #[must_use]
    pub fn new(store: Arc<ToolStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for DeleteToolTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("delete_tool", "Delete a custom tool")
            .string_param("name", "Name of the tool to delete", true)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'name'".to_string()))?;

        self.store.remove(name).await?;

        Ok(ToolResult::success(format!("Deleted tool '{}'", name)))
    }
}

/// Wrapper to execute a script tool dynamically
pub struct ScriptToolExecutor {
    tool: ScriptTool,
}

impl ScriptToolExecutor {
    #[must_use]
    pub fn new(tool: ScriptTool) -> Self {
        Self { tool }
    }
}

#[async_trait]
impl Tool for ScriptToolExecutor {
    fn definition(&self) -> ToolDefinition {
        let mut def = ToolDefinition::new(&self.tool.name, &self.tool.description);

        for param in &self.tool.parameters {
            def = match param.param_type.as_str() {
                "number" | "integer" => def.int_param(&param.name, &param.description, param.required),
                "boolean" => def.bool_param(&param.name, &param.description, param.required),
                _ => def.string_param(&param.name, &param.description, param.required),
            };
        }

        def
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        // Build environment variables from parameters
        let mut env_vars: HashMap<String, String> = HashMap::new();
        for (key, value) in &params {
            let str_value = match value {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => value.to_string(),
            };
            env_vars.insert(format!("PARAM_{}", key.to_uppercase()), str_value);
        }

        // Determine shell command
        let (shell, shell_arg) = match self.tool.script_type.as_str() {
            "python" => ("python", "-c"),
            "powershell" => ("powershell", "-Command"),
            _ => {
                if cfg!(windows) {
                    ("cmd", "/C")
                } else {
                    ("sh", "-c")
                }
            }
        };

        // Execute the script
        let mut cmd = tokio::process::Command::new(shell);
        cmd.arg(shell_arg).arg(&self.tool.script);

        for (key, value) in &env_vars {
            cmd.env(key, value);
        }

        if let Some(ref workdir) = self.tool.workdir {
            cmd.current_dir(workdir);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to execute script: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if output.status.success() {
            Ok(ToolResult::success(stdout.to_string()))
        } else {
            Ok(ToolResult::error(format!(
                "Script failed (exit code {:?}):\n{}{}",
                output.status.code(),
                stdout,
                stderr
            )))
        }
    }
}
