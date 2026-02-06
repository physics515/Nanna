//! Tool registry for managing available tools

use crate::{Tool, ToolCall, ToolDefinition, ToolResponse, ToolResult};
use crate::skills::{load_skill, discover_skills};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Registry of available tools
pub struct ToolRegistry {
    tools: RwLock<HashMap<String, Arc<dyn Tool>>>,
    /// Alias names (not included in definitions sent to API)
    aliases: RwLock<HashSet<String>>,
}

impl ToolRegistry {
    #[must_use] 
    pub fn new() -> Self {
        Self {
            tools: RwLock::new(HashMap::new()),
            aliases: RwLock::new(HashSet::new()),
        }
    }

    /// Register a tool
    pub async fn register<T: Tool + 'static>(&self, tool: T) {
        let definition = tool.definition();
        let name = definition.name.clone();
        let mut tools = self.tools.write().await;
        tools.insert(name.clone(), Arc::new(tool));
        info!("Registered tool: {}", name);
    }

    /// Register a boxed tool
    pub async fn register_boxed(&self, tool: Arc<dyn Tool>) {
        let definition = tool.definition();
        let name = definition.name.clone();
        let mut tools = self.tools.write().await;
        tools.insert(name.clone(), tool);
        info!("Registered tool: {}", name);
    }

    /// Register an alias for an existing tool
    /// This allows the same tool to be called by different names.
    /// Aliases are used for execution lookup but NOT sent to the API (to avoid duplicates).
    pub async fn register_alias(&self, alias: &str, target: &str) {
        let tools = self.tools.read().await;
        if let Some(tool) = tools.get(target).cloned() {
            drop(tools); // Release read lock before acquiring write lock
            let mut tools = self.tools.write().await;
            tools.insert(alias.to_string(), tool);
            drop(tools);
            // Track this as an alias so we don't include it in definitions
            let mut aliases = self.aliases.write().await;
            aliases.insert(alias.to_string());
            info!("Registered tool alias: {} -> {}", alias, target);
        } else {
            warn!("Cannot create alias '{}': target tool '{}' not found", alias, target);
        }
    }

    /// Get a tool by name
    pub async fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let tools = self.tools.read().await;
        tools.get(name).cloned()
    }

    /// Get all tool definitions (excludes aliases to avoid duplicates)
    pub async fn definitions(&self) -> Vec<ToolDefinition> {
        let tools = self.tools.read().await;
        let aliases = self.aliases.read().await;
        tools
            .iter()
            .filter(|(name, _)| !aliases.contains(*name))
            .map(|(_, t)| t.definition())
            .collect()
    }

    /// Get tool definitions in Anthropic format (excludes aliases to avoid duplicates)
    pub async fn to_anthropic_format(&self) -> Vec<Value> {
        let tools = self.tools.read().await;
        let aliases = self.aliases.read().await;
        tools
            .iter()
            .filter(|(name, _)| !aliases.contains(*name))
            .map(|(_, t)| t.definition().to_anthropic_format())
            .collect()
    }

    /// Get tool definitions in `OpenAI` format (excludes aliases to avoid duplicates)
    pub async fn to_openai_format(&self) -> Vec<Value> {
        let tools = self.tools.read().await;
        let aliases = self.aliases.read().await;
        tools
            .iter()
            .filter(|(name, _)| !aliases.contains(*name))
            .map(|(_, t)| t.definition().to_openai_format())
            .collect()
    }

    /// Execute a tool call
    pub async fn execute(&self, call: ToolCall) -> ToolResponse {
        // Log input parameters (truncated for readability)
        let params_str = serde_json::to_string(&call.parameters).unwrap_or_default();
        let params_preview = if params_str.len() > 300 {
            let end = truncate_boundary(&params_str, 300);
            format!("{}...", &params_str[..end])
        } else {
            params_str
        };
        debug!("Executing tool: {} (id: {}) with params: {}", call.name, call.id, params_preview);

        let tool = match self.get(&call.name).await {
            Some(t) => t,
            None => {
                warn!("Tool not found: {}", call.name);
                return ToolResponse {
                    id: call.id,
                    name: call.name.clone(),
                    result: ToolResult::error(format!("Tool not found: {}", call.name)),
                };
            }
        };

        let start = std::time::Instant::now();
        
        // Execute with timeout if specified
        let result = if let Some(timeout_secs) = tool.timeout_secs() {
            match tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs),
                tool.execute(call.parameters.clone()),
            )
            .await
            {
                Ok(Ok(result)) => result,
                Ok(Err(e)) => {
                    warn!("Tool {} execution error: {}", call.name, e);
                    ToolResult::error(e.to_string())
                }
                Err(_) => {
                    warn!("Tool {} timed out after {}s", call.name, timeout_secs);
                    ToolResult::error("Tool execution timed out")
                }
            }
        } else {
            match tool.execute(call.parameters.clone()).await {
                Ok(result) => result,
                Err(e) => {
                    warn!("Tool {} execution error: {}", call.name, e);
                    ToolResult::error(e.to_string())
                }
            }
        };

        let duration_ms = start.elapsed().as_millis();
        
        // Log result summary
        let output_preview = if result.content.len() > 200 {
            let end = truncate_boundary(&result.content, 200);
            format!("{}...", &result.content[..end])
        } else {
            result.content.clone()
        };
        
        if result.success {
            debug!("Tool {} completed in {}ms: {}", call.name, duration_ms, output_preview);
        } else {
            warn!("Tool {} failed in {}ms: {}", call.name, duration_ms, output_preview);
        }

        ToolResponse {
            id: call.id,
            name: call.name,
            result,
        }
    }

    /// Execute multiple tool calls in parallel
    pub async fn execute_parallel(&self, calls: Vec<ToolCall>) -> Vec<ToolResponse> {
        let futures: Vec<_> = calls.into_iter().map(|call| self.execute(call)).collect();
        futures::future::join_all(futures).await
    }

    /// Load user-authored skills from a directory
    ///
    /// Discovers and loads all skills from the given directory (e.g., `workspace/skills/`).
    /// Returns the number of skills successfully loaded.
    pub async fn load_skills(&self, skills_dir: &Path) -> usize {
        let discovered = discover_skills(skills_dir);
        let total = discovered.len();
        let mut loaded = 0;
        
        for skill in discovered {
            match load_skill(&skill.path).await {
                Ok(tool) => {
                    let name = tool.definition().name.clone();
                    self.register_boxed(tool).await;
                    info!(name = %name, path = ?skill.path, "Loaded skill");
                    loaded += 1;
                }
                Err(e) => {
                    warn!(name = %skill.name, error = %e, "Failed to load skill");
                }
            }
        }
        
        info!(loaded, total, "Skills loaded from {:?}", skills_dir);
        loaded
    }

    /// Unregister a tool by name
    pub async fn unregister(&self, name: &str) -> bool {
        let mut tools = self.tools.write().await;
        tools.remove(name).is_some()
    }

    /// Check if a tool is registered
    pub async fn has(&self, name: &str) -> bool {
        let tools = self.tools.read().await;
        tools.contains_key(name)
    }

    /// Get the number of registered tools
    pub async fn len(&self) -> usize {
        let tools = self.tools.read().await;
        tools.len()
    }

    /// Check if the registry is empty
    pub async fn is_empty(&self) -> bool {
        let tools = self.tools.read().await;
        tools.is_empty()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Find the largest byte index <= max_bytes that is a valid char boundary.
fn truncate_boundary(s: &str, max_bytes: usize) -> usize {
    if s.len() <= max_bytes {
        return s.len();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::EchoTool;

    #[tokio::test]
    async fn test_registry() {
        let registry = ToolRegistry::new();
        registry.register(EchoTool).await;

        let definitions = registry.definitions().await;
        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].name, "echo");

        let call = ToolCall {
            id: "test-1".to_string(),
            name: "echo".to_string(),
            parameters: [("text".to_string(), Value::String("hello".to_string()))]
                .into_iter()
                .collect(),
        };

        let response = registry.execute(call).await;
        assert!(response.result.success);
        assert_eq!(response.result.content, "hello");
    }
}
