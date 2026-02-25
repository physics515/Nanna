//! Scripted tool wrapper (Boa/Deno)
//!
//! Wraps nanna-scripting tools to implement the Tool trait.

use crate::{Tool, ToolDefinition, ToolError, ToolResult, ToolRegistry, ParameterType, ToolParameter, OutputTarget};
use async_trait::async_trait;
use nanna_scripting::{ScriptEngine, ScriptedTool, ToolManifest, ToolPermissions, extract_manifest, ServiceFn};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use tracing::{debug, info};

/// A tool implemented in JavaScript/TypeScript, executed via Boa or Deno
pub struct ScriptedToolWrapper {
    /// The underlying scripted tool
    tool: ScriptedTool,
    /// Extracted manifest (name, description, parameters)
    manifest: ToolManifest,
    /// Script engine instance
    engine: Arc<ScriptEngine>,
    /// Optional weak reference to the tool registry (for `Nanna.listTools()`)
    registry: Option<Weak<ToolRegistry>>,
    /// Optional service functions for `Nanna.service()`
    services: Option<HashMap<String, ServiceFn>>,
}

impl ScriptedToolWrapper {
    /// Create from a tool.js or tool.ts file
    pub async fn from_file(path: &Path) -> Result<Self, ToolError> {
        let mut tool = ScriptedTool::from_file(path).map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to load script: {e}"))
        })?;

        // Check for permissions.json alongside the tool file
        if let Some(parent) = path.parent() {
            let perms_path = parent.join("permissions.json");
            if perms_path.exists() {
                if let Ok(perms_str) = std::fs::read_to_string(&perms_path) {
                    if let Ok(mut perms) = serde_json::from_str::<ToolPermissions>(&perms_str) {
                        // Expand ~ to home directory and resolve relative paths
                        let home = directories::UserDirs::new()
                            .map(|d| d.home_dir().to_path_buf())
                            .unwrap_or_else(|| PathBuf::from("."));
                        let resolve = |p: &PathBuf| -> PathBuf {
                            let s = p.to_string_lossy();
                            if s == "~" || s.starts_with("~/") {
                                home.join(s.strip_prefix("~/").unwrap_or(""))
                            } else if p.is_relative() {
                                std::env::current_dir().unwrap_or_default().join(p)
                            } else {
                                p.clone()
                            }
                        };
                        perms.read = perms.read.iter().map(resolve).collect();
                        perms.write = perms.write.iter().map(resolve).collect();
                        debug!(path = ?perms_path, read = ?perms.read, "Loaded permissions for scripted tool");
                        tool = tool.with_permissions(perms);
                    }
                }
            }
        }

        let manifest = extract_manifest(&tool.source).ok_or_else(|| {
            ToolError::InvalidParams(
                "Script must export default with name and description".to_string()
            )
        })?;

        info!(name = %manifest.name, path = ?path, "Loaded scripted tool");

        Ok(Self {
            tool,
            manifest,
            engine: Arc::new(ScriptEngine::new()),
            registry: None,
            services: None,
        })
    }

    /// Create from source code directly
    pub fn from_source(name: impl Into<String>, source: impl Into<String>) -> Result<Self, ToolError> {
        let tool = ScriptedTool::new(name, source);

        let manifest = extract_manifest(&tool.source).ok_or_else(|| {
            ToolError::InvalidParams(
                "Script must export default with name and description".to_string()
            )
        })?;

        Ok(Self {
            tool,
            manifest,
            engine: Arc::new(ScriptEngine::new()),
            registry: None,
            services: None,
        })
    }

    /// Attach a weak reference to the tool registry.
    ///
    /// When set, `Nanna.listTools()` will return all tool definitions from the registry.
    #[must_use]
    pub fn with_registry(mut self, registry: Weak<ToolRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Attach service functions for `Nanna.service()`.
    #[must_use]
    pub fn with_services(mut self, services: HashMap<String, ServiceFn>) -> Self {
        self.services = Some(services);
        self
    }
}

#[async_trait]
impl Tool for ScriptedToolWrapper {
    fn definition(&self) -> ToolDefinition {
        // Parse parameters from manifest if available
        let parameters = if let Some(ref schema) = self.manifest.parameters {
            parse_params_from_schema(schema)
        } else {
            vec![]
        };
        
        ToolDefinition {
            name: self.manifest.name.clone(),
            description: self.manifest.description.clone().unwrap_or_default(),
            parameters,
            output_schema: None,
        }
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        debug!(tool = %self.manifest.name, "Executing scripted tool");

        // Build tool definitions JSON if registry is available
        let tool_defs = if let Some(ref weak) = self.registry {
            if let Some(registry) = weak.upgrade() {
                let defs = registry.definitions().await;
                let arr: Vec<Value> = defs.iter().map(|d| {
                    let params: Vec<Value> = d.parameters.iter().map(|p| {
                        serde_json::json!({
                            "name": p.name,
                            "type": format!("{:?}", p.param_type).to_lowercase(),
                            "required": p.required,
                            "description": p.description,
                        })
                    }).collect();
                    serde_json::json!({
                        "name": d.name,
                        "description": d.description,
                        "parameters": params,
                    })
                }).collect();
                Some(Value::Array(arr))
            } else {
                None
            }
        } else {
            None
        };

        let input = Value::Object(params.into_iter().collect());

        let result = self.engine.execute(&self.tool, input, tool_defs, self.services.clone()).await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Script execution failed: {e}"))
        })?;

        debug!(
            tool = %self.manifest.name,
            engine = %result.engine,
            duration_ms = result.duration_ms,
            fallback = result.used_fallback,
            "Script executed"
        );

        // Check for structured result: { content: "...", data: {...} }
        if let Value::Object(ref obj) = result.value {
            if let Some(content_val) = obj.get("content") {
                let content = match content_val {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
                };
                let mut tool_result = ToolResult::success(content);
                if let Some(data) = obj.get("data") {
                    tool_result = tool_result.with_data(data.clone());
                }
                return Ok(tool_result);
            }
        }

        // Fallback: plain string/null/other
        let content = match result.value {
            Value::String(s) => s,
            Value::Null => String::new(),
            other => serde_json::to_string_pretty(&other).unwrap_or_else(|_| other.to_string()),
        };

        Ok(ToolResult::success(content))
    }

    fn output_target(&self) -> OutputTarget {
        match self.manifest.output {
            nanna_scripting::OutputTarget::Context => OutputTarget::Context,
            nanna_scripting::OutputTarget::Memory => OutputTarget::Memory,
        }
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(self.tool.timeout_ms / 1000)
    }
}

/// Parse parameters from a JSON Schema value
fn parse_params_from_schema(schema: &Value) -> Vec<ToolParameter> {
    let mut params = Vec::new();
    
    if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
        let required: Vec<&str> = schema.get("required")
            .and_then(|r| r.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        
        for (name, prop) in properties {
            let param_type = match prop.get("type").and_then(|t| t.as_str()) {
                Some("string") => ParameterType::String,
                Some("integer") => ParameterType::Integer,
                Some("number") => ParameterType::Number,
                Some("boolean") => ParameterType::Boolean,
                Some("array") => ParameterType::Array,
                Some("object") => ParameterType::Object,
                _ => ParameterType::String,
            };
            
            let description = prop.get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string();
            
            // Extract enum values if present
            let enum_values = prop.get("enum")
                .and_then(|e| e.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect());
            
            params.push(ToolParameter {
                name: name.clone(),
                description,
                param_type,
                required: required.contains(&name.as_str()),
                default: prop.get("default").cloned(),
                enum_values,
            });
        }
    }
    
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_scripted_tool_from_source() {
        let source = r#"
            export default {
                name: "greet",
                description: "Greet someone",
                execute({ name }) {
                    return `Hello, ${name}!`;
                }
            }
        "#;
        
        let tool = ScriptedToolWrapper::from_source("greet", source).unwrap();
        let def = tool.definition();
        
        assert_eq!(def.name, "greet");
        assert_eq!(def.description, "Greet someone");
    }
}
