//! Vision tools - image analysis

use crate::{Tool, ToolDefinition, ToolError, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Callback for analyzing images with a vision model
pub type VisionFn = Arc<
    dyn Fn(String, String, String) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

/// Tool for analyzing images using a vision model
pub struct AnalyzeImageTool {
    vision_fn: Option<VisionFn>,
}

impl AnalyzeImageTool {
    #[must_use]
    pub fn new() -> Self {
        Self { vision_fn: None }
    }

    /// Set the vision function callback.
    #[must_use]
    pub fn with_vision_fn(mut self, f: VisionFn) -> Self {
        self.vision_fn = Some(f);
        self
    }
}

impl Default for AnalyzeImageTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for AnalyzeImageTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("analyze_image", "Analyze an image using a vision model")
            .string_param("image", "Base64-encoded image data or URL", true)
            .string_param("prompt", "What to analyze or look for in the image", true)
            .string_param("media_type", "MIME type (image/jpeg, image/png, etc.) - required for base64", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let image = params
            .get("image")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'image' parameter".to_string()))?;

        let prompt = params
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("Describe this image in detail.");

        let media_type = params
            .get("media_type")
            .and_then(|v| v.as_str())
            .unwrap_or("image/jpeg");

        let vision_fn = self.vision_fn.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed("Vision model not configured".to_string())
        })?;

        let result = vision_fn(image.to_string(), prompt.to_string(), media_type.to_string())
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Vision analysis failed: {}", e)))?;

        Ok(ToolResult::success(result))
    }
}

/// Tool for taking screenshots of web pages
pub struct ScreenshotTool {
    // Will be implemented with browser automation
}

impl ScreenshotTool {
    #[must_use]
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for ScreenshotTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ScreenshotTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("screenshot", "Take a screenshot of a web page (not yet implemented)")
            .string_param("url", "URL of the page to screenshot", true)
            .bool_param("full_page", "Capture full page (not just viewport)", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'url' parameter".to_string()))?;

        // Placeholder - will be implemented with browser automation
        Err(ToolError::ExecutionFailed(format!(
            "Screenshot tool not yet implemented. URL: {}",
            url
        )))
    }
}
