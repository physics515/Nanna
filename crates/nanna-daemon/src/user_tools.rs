//! User Tool Manager - Create, edit, and manage user-defined tools
//!
//! Moved from GUI to daemon so all clients can access user tools.

use async_trait::async_trait;
use nanna_scripting::{ScriptEngine, ScriptedTool, ToolPermissions, extract_manifest};
use nanna_tools::{Tool, ToolDefinition, ToolError, ToolRegistry, ToolResult, ParameterType, ToolParameter};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// User-created tool metadata (persisted to disk)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserToolMeta {
    pub name: String,
    pub description: String,
    pub source: String,
    pub language: String, // "javascript" or "typescript"
    pub parameters: Option<Value>, // JSON Schema
    pub permissions: UserToolPermissions,
    pub created_at: i64,
    pub updated_at: i64,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserToolPermissions {
    pub net: Vec<String>,
    pub read: Vec<String>,
    pub write: Vec<String>,
    pub env: bool,
    #[serde(default)]
    pub run: bool,
}

impl From<UserToolPermissions> for ToolPermissions {
    fn from(p: UserToolPermissions) -> Self {
        let mut perms = ToolPermissions::none()
            .with_net(p.net)
            .with_read(p.read.into_iter().map(PathBuf::from))
            .with_write(p.write.into_iter().map(PathBuf::from));
        perms.env = p.env;
        perms.run = p.run;
        perms
    }
}

/// Manager for user-created tools
pub struct UserToolManager {
    tools_dir: PathBuf,
    engine: Arc<ScriptEngine>,
    tools: RwLock<HashMap<String, UserToolMeta>>,
}

impl UserToolManager {
    /// Create a new manager with the given tools directory
    pub fn new(tools_dir: PathBuf) -> Self {
        // Ensure directory exists
        if let Err(e) = std::fs::create_dir_all(&tools_dir) {
            warn!("Failed to create tools directory: {}", e);
        }
        
        Self {
            tools_dir,
            engine: Arc::new(ScriptEngine::new()),
            tools: RwLock::new(HashMap::new()),
        }
    }

    /// Load all user tools from disk
    pub async fn load_all(&self) -> Result<usize, std::io::Error> {
        let mut count = 0;
        let mut tools = self.tools.write().await;
        tools.clear();

        let entries = std::fs::read_dir(&self.tools_dir)?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "json") {
                match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        match serde_json::from_str::<UserToolMeta>(&content) {
                            Ok(meta) => {
                                info!("Loaded user tool: {}", meta.name);
                                tools.insert(meta.name.clone(), meta);
                                count += 1;
                            }
                            Err(e) => warn!("Failed to parse tool {:?}: {}", path, e),
                        }
                    }
                    Err(e) => warn!("Failed to read tool {:?}: {}", path, e),
                }
            }
        }

        Ok(count)
    }

    /// Save a tool to disk
    fn save_tool(&self, meta: &UserToolMeta) -> Result<(), std::io::Error> {
        let path = self.tools_dir.join(format!("{}.json", meta.name));
        let content = serde_json::to_string_pretty(meta)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(path, content)
    }

    /// Create a new user tool
    pub async fn create_tool(
        &self,
        name: String,
        description: String,
        source: String,
        language: Option<String>,
        parameters: Option<Value>,
        permissions: Option<UserToolPermissions>,
    ) -> Result<UserToolMeta, String> {
        // Security: the name becomes `{name}.json` on disk, so it must be a safe
        // single filename component — reject path traversal before anything else.
        validate_tool_name(&name)?;

        // Validate the source compiles
        let _test_tool = ScriptedTool::new(&name, &source);
        
        // Try to extract manifest to validate
        if extract_manifest(&source).is_none() {
            return Err("Source must export default with name and description. Example:\nexport default {\n  name: \"my_tool\",\n  description: \"Does something\",\n  execute(params) { return \"result\"; }\n}".to_string());
        }

        let now = chrono::Utc::now().timestamp();
        let meta = UserToolMeta {
            name: name.clone(),
            description,
            source,
            language: language.unwrap_or_else(|| "javascript".to_string()),
            parameters,
            permissions: permissions.unwrap_or_default(),
            created_at: now,
            updated_at: now,
            enabled: true,
        };

        // Save to disk
        self.save_tool(&meta).map_err(|e| e.to_string())?;

        // Add to in-memory cache
        self.tools.write().await.insert(name.clone(), meta.clone());

        info!("Created user tool: {}", name);
        Ok(meta)
    }

    /// Update an existing tool
    pub async fn update_tool(
        &self,
        name: &str,
        description: Option<String>,
        source: Option<String>,
        parameters: Option<Value>,
        permissions: Option<UserToolPermissions>,
        enabled: Option<bool>,
    ) -> Result<UserToolMeta, String> {
        let mut tools = self.tools.write().await;
        let meta = tools.get_mut(name).ok_or_else(|| format!("Tool not found: {name}"))?;

        if let Some(desc) = description {
            meta.description = desc;
        }
        if let Some(src) = source {
            // Validate new source
            if extract_manifest(&src).is_none() {
                return Err("Invalid source: must export default with name and description".to_string());
            }
            meta.source = src;
        }
        if let Some(params) = parameters {
            meta.parameters = Some(params);
        }
        if let Some(perms) = permissions {
            meta.permissions = perms;
        }
        if let Some(en) = enabled {
            meta.enabled = en;
        }

        meta.updated_at = chrono::Utc::now().timestamp();

        // Save to disk
        self.save_tool(meta).map_err(|e| e.to_string())?;

        info!("Updated user tool: {}", name);
        Ok(meta.clone())
    }

    /// Delete a tool
    pub async fn delete_tool(&self, name: &str) -> Result<(), String> {
        let mut tools = self.tools.write().await;
        tools.remove(name).ok_or_else(|| format!("Tool not found: {name}"))?;

        let path = self.tools_dir.join(format!("{name}.json"));
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }

        info!("Deleted user tool: {}", name);
        Ok(())
    }

    /// Get all user tools
    pub async fn list_tools(&self) -> Vec<UserToolMeta> {
        self.tools.read().await.values().cloned().collect()
    }

    /// Get a specific tool
    pub async fn get_tool(&self, name: &str) -> Option<UserToolMeta> {
        self.tools.read().await.get(name).cloned()
    }

    /// Test a tool with given input (doesn't save)
    pub async fn test_tool(
        &self,
        source: &str,
        input: HashMap<String, Value>,
    ) -> Result<String, String> {
        let tool = ScriptedTool::new("_test", source);
        let input_value = Value::Object(input.into_iter().collect());
        
        match self.engine.execute(&tool, input_value, None, None).await {
            Ok(result) => {
                let output = match result.value {
                    Value::String(s) => s,
                    Value::Null => "(no output)".to_string(),
                    other => serde_json::to_string_pretty(&other).unwrap_or_else(|_| other.to_string()),
                };
                Ok(format!("✓ Executed in {}ms via {}\n\n{}", result.duration_ms, result.engine, output))
            }
            Err(e) => Err(format!("✗ Execution failed: {e}")),
        }
    }

    /// Create a Tool implementation for a user tool
    pub fn create_tool_impl(&self, meta: &UserToolMeta) -> Result<Arc<dyn Tool>, String> {
        let tool = ScriptedTool::new(&meta.name, &meta.source)
            .with_permissions(meta.permissions.clone().into());
        
        let wrapper = UserToolWrapper {
            meta: meta.clone(),
            tool,
            engine: self.engine.clone(),
        };
        
        Ok(Arc::new(wrapper))
    }

    /// Register all enabled user tools with the registry
    pub async fn register_with_registry(&self, registry: &ToolRegistry) -> usize {
        let tools = self.tools.read().await;
        let mut count = 0;
        
        for meta in tools.values() {
            if !meta.enabled {
                continue;
            }
            
            match self.create_tool_impl(meta) {
                Ok(tool) => {
                    registry.register_boxed(tool).await;
                    count += 1;
                }
                Err(e) => {
                    warn!("Failed to register user tool {}: {}", meta.name, e);
                }
            }
        }
        
        count
    }
}

/// Wrapper to make UserToolMeta implement Tool trait
struct UserToolWrapper {
    meta: UserToolMeta,
    tool: ScriptedTool,
    engine: Arc<ScriptEngine>,
}

#[async_trait]
impl Tool for UserToolWrapper {
    fn definition(&self) -> ToolDefinition {
        let parameters = if let Some(ref schema) = self.meta.parameters {
            parse_params_from_schema(schema)
        } else {
            vec![]
        };
        
        ToolDefinition {
            name: self.meta.name.clone(),
            description: self.meta.description.clone(),
            parameters,
            output_schema: None,
        }
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let input = Value::Object(params.into_iter().collect());
        
        let result = self.engine.execute(&self.tool, input, None, None).await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Script execution failed: {e}"))
        })?;
        
        // If the script returns an object with { content, success } fields,
        // respect the success flag. This lets tools like `exec` signal failure
        // (e.g. command exited nonzero) while still returning output for the LLM.
        let (content, success, data) = match &result.value {
            Value::Object(map) if map.contains_key("content") => {
                let c = map.get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let s = map.get("success")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                // Pass through extra fields as structured data (e.g. "written" for write_file)
                let mut extra = serde_json::Map::new();
                for (k, v) in map.iter() {
                    if k != "content" && k != "success" {
                        extra.insert(k.clone(), v.clone());
                    }
                }
                let d = if extra.is_empty() { None } else { Some(Value::Object(extra)) };
                (c, s, d)
            }
            Value::String(s) => {
                // Heuristic: strings starting with "Error:" indicate tool failure.
                // This catches tools that return error messages as plain strings
                // without needing to update every tool script.
                let failed = s.starts_with("Error:") || s.starts_with("Error ");
                (s.clone(), !failed, None)
            }
            Value::Null => (String::new(), true, None),
            other => (serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()), true, None),
        };
        
        if success {
            let mut r = ToolResult::success(content);
            r.data = data;
            Ok(r)
        } else {
            // Return with success=false so notifications fire, but include the
            // full output as content so the LLM can still read what happened.
            Ok(ToolResult {
                success: false,
                content,
                error: None,
                data,
            })
        }
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(30)
    }
}

/// Maximum length of a user-tool name, in bytes.
const TOOL_NAME_LEN_MAX: usize = 64;

/// Validate a user-tool name against `^[a-z][a-z0-9_]{0,63}$`.
///
/// Security guard: the name is used to build the on-disk file `{name}.json`, so
/// an unsanitized name like `../../etc/cron.d/evil` would escape the tools dir.
/// The pattern permits only lowercase ASCII letters, digits, and underscore, and
/// requires a leading letter — none of which can form a path separator, `.`, or
/// `..`, and the length is bounded per Tiger Style.
///
/// # Errors
/// Returns a human-readable message if the name is empty, too long, or contains
/// a disallowed character.
fn validate_tool_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > TOOL_NAME_LEN_MAX {
        return Err(format!(
            "tool name must be 1..={TOOL_NAME_LEN_MAX} chars, got {} ({name:?})",
            name.len()
        ));
    }
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase());
    let rest_ok = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !first_ok || !rest_ok {
        return Err(format!(
            "tool name must match ^[a-z][a-z0-9_]{{0,63}}$ (lowercase letter first, then \
             lowercase/digit/underscore): {name:?}"
        ));
    }
    Ok(())
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
    fn validate_tool_name_accepts_valid() {
        for good in ["a", "my_tool", "tool2", "read_file", "x_1_2_3"] {
            assert!(validate_tool_name(good).is_ok(), "should accept {good:?}");
        }
    }

    #[test]
    fn validate_tool_name_rejects_traversal_and_bad_chars() {
        for bad in [
            "",
            "../../etc/cron.d/evil",
            "../evil",
            "a/b",
            "a\\b",
            "Tool",      // uppercase leading
            "1tool",     // digit leading
            "_tool",     // underscore leading
            "my-tool",   // hyphen
            "my tool",   // space
            "tool.json", // dot
            "..",
        ] {
            assert!(validate_tool_name(bad).is_err(), "should reject {bad:?}");
        }
        let too_long = "a".repeat(TOOL_NAME_LEN_MAX + 1);
        assert!(validate_tool_name(&too_long).is_err());
    }

    #[tokio::test]
    async fn create_tool_rejects_traversal_name() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = UserToolManager::new(dir.path().to_path_buf());
        let src = "export default { name: \"x\", description: \"d\", execute() { return \"ok\"; } }";
        let res = mgr
            .create_tool("../escaped".to_string(), "d".to_string(), src.to_string(), None, None, None)
            .await;
        assert!(res.is_err());
        // Nothing must have been written outside the tools dir.
        assert!(!dir.path().parent().unwrap().join("escaped.json").exists());
    }
}
