//! Tool registry for managing available tools

use crate::{Tool, ToolCall, ToolDefinition, ToolResponse, ToolResult};
use crate::skills::{load_skill, discover_skills};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Registry of available tools
pub struct ToolRegistry {
    tools: RwLock<HashMap<String, Arc<dyn Tool>>>,
}

impl ToolRegistry {
    #[must_use] 
    pub fn new() -> Self {
        Self {
            tools: RwLock::new(HashMap::new()),
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

    /// Get a tool by name
    pub async fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let tools = self.tools.read().await;
        tools.get(name).cloned()
    }

    /// Get all tool definitions
    pub async fn definitions(&self) -> Vec<ToolDefinition> {
        let tools = self.tools.read().await;
        tools.values().map(|t| t.definition()).collect()
    }

    /// Get tool definitions in Anthropic format
    pub async fn to_anthropic_format(&self) -> Vec<Value> {
        let tools = self.tools.read().await;
        tools
            .values()
            .map(|t| t.definition().to_anthropic_format())
            .collect()
    }

    /// Get tool definitions in `OpenAI` format
    pub async fn to_openai_format(&self) -> Vec<Value> {
        let tools = self.tools.read().await;
        tools
            .values()
            .map(|t| t.definition().to_openai_format())
            .collect()
    }

    /// Execute a tool call
    pub async fn execute(&self, call: ToolCall) -> ToolResponse {
        debug!("Executing tool: {} (id: {})", call.name, call.id);

        let tool = match self.get(&call.name).await {
            Some(t) => t,
            None => {
                return ToolResponse {
                    id: call.id,
                    name: call.name.clone(),
                    result: ToolResult::error(format!("Tool not found: {}", call.name)),
                };
            }
        };

        // Execute with timeout if specified
        let result = if let Some(timeout_secs) = tool.timeout_secs() {
            match tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs),
                tool.execute(call.parameters.clone()),
            )
            .await
            {
                Ok(Ok(result)) => result,
                Ok(Err(e)) => ToolResult::error(e.to_string()),
                Err(_) => ToolResult::error("Tool execution timed out"),
            }
        } else {
            match tool.execute(call.parameters.clone()).await {
                Ok(result) => result,
                Err(e) => ToolResult::error(e.to_string()),
            }
        };

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
