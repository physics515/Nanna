//! Executable tool wrapper (Python, shell, binary, command)

use crate::{Tool, ToolDefinition, ToolError, ToolResult, ParameterType, ToolParameter};
use crate::skills::manifest::{SkillManifest, ExecutionMethod};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use tracing::{debug, warn};

/// A tool executed via external process (Python, shell, binary, or command)
pub struct ExecutableTool {
    manifest: SkillManifest,
    skill_dir: PathBuf,
}

impl ExecutableTool {
    /// Create from a manifest file path
    pub fn from_manifest(manifest_path: &Path) -> Result<Self, ToolError> {
        let manifest = SkillManifest::from_file(manifest_path)?;
        let skill_dir = manifest_path.parent()
            .ok_or_else(|| ToolError::InvalidParams("Invalid manifest path".to_string()))?
            .to_path_buf();
        
        Ok(Self { manifest, skill_dir })
    }

    /// Build the command to execute
    fn build_command(&self, params: &HashMap<String, Value>) -> Result<Command, ToolError> {
        let workdir = self.manifest.resolve_workdir(&self.skill_dir);
        
        let mut cmd = match &self.manifest.execution {
            ExecutionMethod::Python(script) => {
                let script_path = self.skill_dir.join(script);
                let mut cmd = Command::new("python");
                cmd.arg(&script_path);
                cmd.arg("--json");
                cmd.arg(serde_json::to_string(params).map_err(|e| {
                    ToolError::InvalidParams(format!("Failed to serialize params: {e}"))
                })?);
                cmd
            }
            ExecutionMethod::Shell(script) => {
                let script_path = self.skill_dir.join(script);
                let shell = if cfg!(windows) { "cmd" } else { "bash" };
                let shell_arg = if cfg!(windows) { "/C" } else { "-c" };
                
                let mut cmd = Command::new(shell);
                cmd.arg(shell_arg);
                
                // Build: bash script.sh 'json_params'
                let json_params = serde_json::to_string(params).map_err(|e| {
                    ToolError::InvalidParams(format!("Failed to serialize params: {e}"))
                })?;
                
                if cfg!(windows) {
                    cmd.arg(format!("{} {}", script_path.display(), shell_escape(&json_params)));
                } else {
                    cmd.arg(format!("bash {} '{}'", script_path.display(), json_params.replace('\'', "'\\''")));
                }
                cmd
            }
            ExecutionMethod::Command(template) => {
                // Substitute {{param}} placeholders
                let expanded = substitute_params(template, params)?;
                
                let shell = if cfg!(windows) { "cmd" } else { "sh" };
                let shell_arg = if cfg!(windows) { "/C" } else { "-c" };
                
                let mut cmd = Command::new(shell);
                cmd.arg(shell_arg);
                cmd.arg(&expanded);
                cmd
            }
            ExecutionMethod::Binary(binary) => {
                let binary_path = self.skill_dir.join(binary);
                let mut cmd = Command::new(&binary_path);
                // JSON input via stdin
                cmd.stdin(Stdio::piped());
                cmd
            }
        };
        
        // Set working directory
        cmd.current_dir(&workdir);
        
        // Set environment variables
        for (key, value) in &self.manifest.env {
            cmd.env(key, value);
        }
        
        // Capture output
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        
        Ok(cmd)
    }
}

#[async_trait]
impl Tool for ExecutableTool {
    fn definition(&self) -> ToolDefinition {
        // Convert JSON Schema parameters to ToolParameter format
        let parameters = if let Some(schema) = &self.manifest.parameters {
            parse_json_schema_params(schema)
        } else {
            vec![]
        };
        
        ToolDefinition {
            name: self.manifest.name.clone(),
            description: self.manifest.description.clone(),
            parameters,
        }
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        debug!(tool = %self.manifest.name, "Executing executable tool");
        
        let mut cmd = self.build_command(&params)?;
        
        // For binary execution, we need to write to stdin
        let is_binary = matches!(self.manifest.execution, ExecutionMethod::Binary(_));
        
        let output = if is_binary {
            let mut child = cmd.spawn().map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to spawn process: {e}"))
            })?;
            
            // Write JSON to stdin
            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let json = serde_json::to_vec(&params).map_err(|e| {
                    ToolError::InvalidParams(format!("Failed to serialize params: {e}"))
                })?;
                stdin.write_all(&json).await.map_err(|e| {
                    ToolError::ExecutionFailed(format!("Failed to write to stdin: {e}"))
                })?;
            }
            
            child.wait_with_output().await.map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to wait for process: {e}"))
            })?
        } else {
            cmd.output().await.map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to execute: {e}"))
            })?
        };
        
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        
        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            warn!(tool = %self.manifest.name, code, stderr = %stderr, "Tool execution failed");
            return Ok(ToolResult::error(format!(
                "Process exited with code {code}: {stderr}"
            )));
        }
        
        if !stderr.is_empty() {
            debug!(tool = %self.manifest.name, stderr = %stderr, "Tool stderr output");
        }
        
        Ok(ToolResult::success(stdout.trim().to_string()))
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(self.manifest.timeout)
    }
}

/// Substitute `{{param}}` placeholders in a command template
fn substitute_params(template: &str, params: &HashMap<String, Value>) -> Result<String, ToolError> {
    let mut result = template.to_string();
    
    // Find all {{param}} patterns
    let re = regex::Regex::new(r"\{\{(\w+)\}\}").map_err(|e| {
        ToolError::InvalidParams(format!("Invalid regex: {e}"))
    })?;
    
    for cap in re.captures_iter(template) {
        let full_match = &cap[0];
        let param_name = &cap[1];
        
        let value = params.get(param_name).ok_or_else(|| {
            ToolError::InvalidParams(format!("Missing required parameter: {param_name}"))
        })?;
        
        let value_str = match value {
            Value::String(s) => shell_escape(s),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            _ => shell_escape(&value.to_string()),
        };
        
        result = result.replace(full_match, &value_str);
    }
    
    Ok(result)
}

/// Escape a string for shell use
fn shell_escape(s: &str) -> String {
    if cfg!(windows) {
        // Windows: use double quotes, escape internal quotes
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        // Unix: use single quotes, escape internal single quotes
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Parse JSON Schema parameters into ToolParameter format
fn parse_json_schema_params(schema: &Value) -> Vec<ToolParameter> {
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

    #[test]
    fn test_substitute_params() {
        let template = "convert {{input}} -resize {{size}} {{output}}";
        let mut params = HashMap::new();
        params.insert("input".to_string(), Value::String("test.jpg".to_string()));
        params.insert("size".to_string(), Value::String("800x600".to_string()));
        params.insert("output".to_string(), Value::String("out.jpg".to_string()));
        
        let result = substitute_params(template, &params).unwrap();
        // The exact format depends on platform (quoting style)
        assert!(result.contains("test.jpg"));
        assert!(result.contains("800x600"));
        assert!(result.contains("out.jpg"));
    }
}
