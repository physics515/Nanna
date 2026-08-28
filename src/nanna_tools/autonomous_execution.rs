#![allow(dead_code)]
use nanna_core::prelude::*;
use nanna_memory::MemoryManager;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use nanna_agent::AgentLoop;
use nanna_tools::tool_utils::ToolUtils;

/// Unified autonomous execution tool entry point.
/// Replaces 'mission mode' with a first-class tool for long-running tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousExecutionTool {
    pub name: String,
    pub description: String,
    pub config: serde_json::Value,
    pub memory_key: Option<String>, // Optional key for memory integration
}

/// Error types for the autonomous execution tool.
#[derive(Error, Debug)]
enum AutonomousExecutionError {
    #[error("Invalid task configuration")]
    InvalidConfig(String),
    
    #[error("Task failed to start: {0}")]
    TaskFailedToStart(String),
    
    #[error("Task execution timeout")]
    ExecutionTimeout,
    
    #[error("Memory/dreaming integration error: {0}")]
    MemoryIntegration(String),
    
    #[error("Task lifecycle error: {0}")]
    LifecycleError(String),
}

impl AutonomousExecutionTool {
    /// Initialize the tool with default configuration.
    pub fn new(name: &str, description: &str) -> Self {
        AutonomousExecutionTool {
            name: name.to_string(),
            description: description.to_string(),
            config: serde_json::json!({
                "timeout_secs": 3600,
                "max_retries": 3,
                "dreaming_enabled": true,
            }),
            memory_key: None,
        }
    }

    /// Register the tool in Nanna's system.
    pub fn register(self, registry: &mut ToolRegistry) -> Result<(), Box<dyn std::error::Error>> {
        registry.register_tool(
            self.name.clone(),
            Box::new(move || Self::new(&self.name, &self.description)),
        )
    }

    /// Execute a task with memory/dreaming integration.
    pub async fn execute_task(
        &self,
        task: TaskRequest,
        memory: &mut MemoryManager,
    ) -> Result<TaskResult, AutonomousExecutionError> {
        // Validate task configuration
        if task.config.is_null() || task.config.as_object().unwrap().is_empty() {
            return Err(AutonomousExecutionError::InvalidConfig("Empty task config".to_string()));
        }

        // Integrate with memory/dreaming systems
        let mut task_memory = memory.clone();
        if let Some(memory_key) = &self.memory_key {
            task_memory = memory.get_or_create_mnemonic(memory_key)?;
        }

        // Spawn task execution as a long-running job
        let (tx, rx) = mpsc::channel::<TaskResult>();
        tokio::spawn(async move {
            let result = Self::run_task(task, task_memory).await;
            tx.send(result).await.unwrap()
        });

        // Block until task completes or times out
        let timeout = Duration::from_secs(
            self.config["timeout_secs"].as_u64().unwrap_or(3600),
        );
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Some(result) => Ok(result),
                    None => Err(AutonomousExecutionError::TaskFailedToStart("Channel closed unexpectedly".to_string()))
                }
            },
            _ = tokio::time::sleep(timeout) => {
                Err(AutonomousExecutionError::ExecutionTimeout)
            }
        }
    }

    /// Spawn and manage task lifecycle with memory/dreaming integration.
    async fn run_task(
        task: TaskRequest,
        memory: MemoryManager,
    ) -> TaskResult {
        // Simulate task execution logic
        let mut agent_loop = AgentLoop::new(memory);
        agent_loop.set_dreaming_enabled(
            task.config["dreaming_enabled"].as_bool().unwrap_or(true),
        );
        
        // Parse and validate task configuration
        if let Some(config) = task.config.as_object() {
            if config.contains_key("max_retries") {
                agent_loop.set_max_retries(config["max_retries"].as_u64().unwrap_or(3));
            }
        }
        
        // Execute the task
        let output = format!("Autonomous task '{}' executed with memory integration", task.id);
        
        // Simulate task steps and integrate with dreaming
        for _ in 0..3 {
            sleep(Duration::from_secs(1)).await;
            agent_loop.update_memory(&output).await.unwrap();
        }

        TaskResult {
            status: "completed".to_string(),
            output,
            metadata: serde_json::json!({
                "task_id": task.id,
                "memory_key": task.memory_key.unwrap_or_default(),
                "dreaming_integration": true,
            }),
        }
    }
}

/// Task request structure for autonomous execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRequest {
    pub id: String,
    pub description: String,
    pub config: serde_json::Value,
    pub memory_key: Option<String>, // Optional key for memory integration
}

/// Task result structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub status: String,
    pub output: String,
    pub metadata: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nanna_memory::MemoryManager;
    
    #[tokio::test]
    async fn test_autonomous_execution_tool() {
        let tool = AutonomousExecutionTool::new("autonomous_execution", "Unified autonomous execution tool");
        let mut memory = MemoryManager::new();
        
        let task = TaskRequest {
            id: "test_task_123".to_string(),
            description: "Test autonomous execution".to_string(),
            config: serde_json::json!({"timeout_secs": 60}),
            memory_key: Some("test_memory".to_string()),
        };
        
        let result = tool.execute_task(task, &mut memory).await.unwrap();
        assert_eq!(result.status, "completed");
    }
}