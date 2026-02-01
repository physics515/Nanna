//! Scripted tool wrapper (Boa/Deno)
//!
//! Wraps nanna-scripting tools to implement the Tool trait.

use crate::{Tool, ToolDefinition, ToolError, ToolResult, ParameterType, ToolParameter};
use async_trait::async_trait;
use nanna_scripting::{ScriptEngine, ScriptedTool, ToolManifest, extract_manifest};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info};

/// A tool implemented in JavaScript/TypeScript, executed via Boa or Deno
pub struct ScriptedToolWrapper {
    /// The underlying scripted tool
    tool: ScriptedTool,
    /// Extracted manifest (name, description, parameters)
    manifest: ToolManifest,
    /// Script engine instance
    engine: Arc<ScriptEngine>,
}

impl ScriptedToolWrapper {
    /// Create from a tool.js or tool.ts file
    pub async fn from_file(path: &Path) -> Result<Self, ToolError> {
        let tool = ScriptedTool::from_file(path).map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to load script: {e}"))
        })?;
        
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
        })
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
        }
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        debug!(tool = %self.manifest.name, "Executing scripted tool");
        
        let input = Value::Object(params.into_iter().collect());
        
        let result = self.engine.execute(&self.tool, input).await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Script execution failed: {e}"))
        })?;
        
        debug!(
            tool = %self.manifest.name, 
            engine = %result.engine,
            duration_ms = result.duration_ms,
            fallback = result.used_fallback,
            "Script executed"
        );
        
        // Convert result value to string
        let content = match result.value {
            Value::String(s) => s,
            Value::Null => String::new(),
            other => serde_json::to_string_pretty(&other).unwrap_or_else(|_| other.to_string()),
        };
        
        Ok(ToolResult::success(content))
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
