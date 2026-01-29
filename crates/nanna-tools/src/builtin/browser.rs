//! Browser automation tools

use crate::{Tool, ToolDefinition, ToolError, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::debug;

/// Callback for browser operations (deferred to avoid circular deps)
pub type BrowserFn<T> = Arc<
    dyn Fn(String, HashMap<String, Value>) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, String>> + Send>>
        + Send
        + Sync,
>;

/// Browser screenshot tool
pub struct BrowserScreenshotTool {
    screenshot_fn: Option<BrowserFn<Vec<u8>>>,
}

impl BrowserScreenshotTool {
    #[must_use]
    pub fn new() -> Self {
        Self { screenshot_fn: None }
    }

    /// Set the screenshot callback.
    #[must_use]
    pub fn with_screenshot_fn(mut self, f: BrowserFn<Vec<u8>>) -> Self {
        self.screenshot_fn = Some(f);
        self
    }
}

impl Default for BrowserScreenshotTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for BrowserScreenshotTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("browser_screenshot", "Take a screenshot of a web page")
            .string_param("url", "URL of the page to screenshot", true)
            .bool_param("full_page", "Capture full page (not just viewport)", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'url' parameter".to_string()))?;

        let screenshot_fn = self.screenshot_fn.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed("Browser not configured".to_string())
        })?;

        debug!("Taking screenshot of {}", url);

        let screenshot = screenshot_fn(url.to_string(), params.clone())
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Screenshot failed: {}", e)))?;

        let base64 = base64_simd::STANDARD.encode_to_string(&screenshot);

        Ok(ToolResult::success(format!("Screenshot captured ({} bytes)", screenshot.len()))
            .with_data(serde_json::json!({
                "image_base64": base64,
                "format": "png",
                "size_bytes": screenshot.len(),
            })))
    }
}

/// Browser page extraction tool
pub struct BrowserExtractTool {
    extract_fn: Option<BrowserFn<String>>,
}

impl BrowserExtractTool {
    #[must_use]
    pub fn new() -> Self {
        Self { extract_fn: None }
    }

    /// Set the extract callback.
    #[must_use]
    pub fn with_extract_fn(mut self, f: BrowserFn<String>) -> Self {
        self.extract_fn = Some(f);
        self
    }
}

impl Default for BrowserExtractTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for BrowserExtractTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("browser_extract", "Extract text or HTML from a web page")
            .string_param("url", "URL of the page", true)
            .string_param("mode", "Extraction mode: 'text' or 'html'", false)
            .string_param("selector", "CSS selector to extract from (optional)", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'url' parameter".to_string()))?;

        let extract_fn = self.extract_fn.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed("Browser not configured".to_string())
        })?;

        debug!("Extracting content from {}", url);

        let content = extract_fn(url.to_string(), params.clone())
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Extraction failed: {}", e)))?;

        Ok(ToolResult::success(content))
    }
}

/// Browser action tool (click, type, etc.)
pub struct BrowserActionTool {
    action_fn: Option<BrowserFn<String>>,
}

impl BrowserActionTool {
    #[must_use]
    pub fn new() -> Self {
        Self { action_fn: None }
    }

    /// Set the action callback.
    #[must_use]
    pub fn with_action_fn(mut self, f: BrowserFn<String>) -> Self {
        self.action_fn = Some(f);
        self
    }
}

impl Default for BrowserActionTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for BrowserActionTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("browser_action", "Perform actions on a web page")
            .string_param("url", "URL of the page (or use existing tab)", true)
            .string_param("action", "Action: 'click', 'type', 'scroll', 'wait'", true)
            .string_param("selector", "CSS selector for the target element", false)
            .string_param("text", "Text to type (for 'type' action)", false)
            .int_param("wait_ms", "Milliseconds to wait (for 'wait' action)", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'url' parameter".to_string()))?;

        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'action' parameter".to_string()))?;

        let action_fn = self.action_fn.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed("Browser not configured".to_string())
        })?;

        debug!("Browser action '{}' on {}", action, url);

        let result = action_fn(url.to_string(), params.clone())
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Action failed: {}", e)))?;

        Ok(ToolResult::success(result))
    }
}

/// Browser evaluate tool (run JavaScript)
pub struct BrowserEvaluateTool {
    evaluate_fn: Option<BrowserFn<serde_json::Value>>,
}

impl BrowserEvaluateTool {
    #[must_use]
    pub fn new() -> Self {
        Self { evaluate_fn: None }
    }

    /// Set the evaluate callback.
    #[must_use]
    pub fn with_evaluate_fn(mut self, f: BrowserFn<serde_json::Value>) -> Self {
        self.evaluate_fn = Some(f);
        self
    }
}

impl Default for BrowserEvaluateTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for BrowserEvaluateTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("browser_evaluate", "Execute JavaScript on a web page")
            .string_param("url", "URL of the page", true)
            .string_param("script", "JavaScript code to execute", true)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'url' parameter".to_string()))?;

        let _script = params
            .get("script")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'script' parameter".to_string()))?;

        let evaluate_fn = self.evaluate_fn.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed("Browser not configured".to_string())
        })?;

        debug!("Evaluating JS on {}", url);

        let result = evaluate_fn(url.to_string(), params.clone())
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Evaluate failed: {}", e)))?;

        Ok(ToolResult::success(result.to_string()).with_data(result))
    }
}
