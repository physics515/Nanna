//! OCR tools - extract text from images and describe image contents

use crate::{Tool, ToolDefinition, ToolError, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Callback for vision model (same signature as VisionFn)
pub type OcrVisionFn = Arc<
    dyn Fn(String, String, String) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

/// Tool for extracting text from images using OCR (via vision model)
pub struct OcrTool {
    vision_fn: Option<OcrVisionFn>,
}

impl OcrTool {
    #[must_use]
    pub fn new() -> Self {
        Self { vision_fn: None }
    }

    /// Set the vision function callback.
    #[must_use]
    pub fn with_vision_fn(mut self, f: OcrVisionFn) -> Self {
        self.vision_fn = Some(f);
        self
    }
}

impl Default for OcrTool {
    fn default() -> Self {
        Self::new()
    }
}

const OCR_PROMPT: &str = r#"Extract ALL text from this image using OCR. 

Instructions:
- Transcribe every piece of text you can see, exactly as written
- Preserve the original formatting, layout, and structure as much as possible
- Include headers, labels, captions, watermarks, handwritten text, etc.
- Use markdown formatting to represent structure (headers, lists, tables)
- If text is unclear or partially visible, indicate with [unclear] or [partial]
- For tables, use markdown table format
- For multi-column layouts, process left-to-right, top-to-bottom

Output the extracted text only, no additional commentary."#;

#[async_trait]
impl Tool for OcrTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("ocr", "Extract text from an image using OCR (optical character recognition)")
            .string_param("image", "Path to image file, URL, or base64-encoded image data", true)
            .string_param("media_type", "MIME type (image/jpeg, image/png, etc.) - required for base64", false)
            .string_param("language", "Expected language hint (e.g., 'english', 'chinese') - optional", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let image = params
            .get("image")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'image' parameter".to_string()))?;

        let media_type = params
            .get("media_type")
            .and_then(|v| v.as_str())
            .unwrap_or("image/jpeg");

        let language = params
            .get("language")
            .and_then(|v| v.as_str());

        let vision_fn = self.vision_fn.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed("Vision model not configured for OCR".to_string())
        })?;

        // Prepare the image (load from file if path, or use as-is if URL/base64)
        let (image_data, actual_media_type) = prepare_image(image, media_type).await?;

        // Build OCR prompt with optional language hint
        let prompt = if let Some(lang) = language {
            format!("{}\n\nLanguage hint: {}", OCR_PROMPT, lang)
        } else {
            OCR_PROMPT.to_string()
        };

        let result = vision_fn(image_data, prompt, actual_media_type)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("OCR failed: {}", e)))?;

        Ok(ToolResult::success(result))
    }
}

/// Tool for describing image contents in detail
pub struct DescribeImageTool {
    vision_fn: Option<OcrVisionFn>,
}

impl DescribeImageTool {
    #[must_use]
    pub fn new() -> Self {
        Self { vision_fn: None }
    }

    /// Set the vision function callback.
    #[must_use]
    pub fn with_vision_fn(mut self, f: OcrVisionFn) -> Self {
        self.vision_fn = Some(f);
        self
    }
}

impl Default for DescribeImageTool {
    fn default() -> Self {
        Self::new()
    }
}

const DESCRIBE_PROMPT: &str = r#"Provide a comprehensive description of this image.

Include:
1. **Overview**: What type of image is this? (photo, diagram, chart, screenshot, etc.)
2. **Main Subject**: What is the primary focus or subject matter?
3. **Details**: Describe specific elements, objects, people, text, colors, etc.
4. **Context**: What setting, environment, or situation is depicted?
5. **Notable Features**: Any interesting, unusual, or important details
6. **Text Content**: If there's any text, summarize what it says
7. **Quality/Style**: Image quality, artistic style, or technical aspects if relevant

Be thorough but organized. Use clear structure."#;

#[async_trait]
impl Tool for DescribeImageTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("describe_image", "Get a detailed description of an image's contents")
            .string_param("image", "Path to image file, URL, or base64-encoded image data", true)
            .string_param("media_type", "MIME type (image/jpeg, image/png, etc.) - required for base64", false)
            .string_param("focus", "What to focus on (e.g., 'people', 'text', 'objects', 'colors')", false)
            .bool_param("brief", "Return a brief 1-2 sentence description instead of detailed", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let image = params
            .get("image")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'image' parameter".to_string()))?;

        let media_type = params
            .get("media_type")
            .and_then(|v| v.as_str())
            .unwrap_or("image/jpeg");

        let focus = params
            .get("focus")
            .and_then(|v| v.as_str());

        let brief = params
            .get("brief")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let vision_fn = self.vision_fn.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed("Vision model not configured".to_string())
        })?;

        // Prepare the image
        let (image_data, actual_media_type) = prepare_image(image, media_type).await?;

        // Build prompt based on options
        let prompt = if brief {
            "Describe this image in 1-2 sentences. Be concise but capture the essential content.".to_string()
        } else if let Some(f) = focus {
            format!("{}\n\nFocus especially on: {}", DESCRIBE_PROMPT, f)
        } else {
            DESCRIBE_PROMPT.to_string()
        };

        let result = vision_fn(image_data, prompt, actual_media_type)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Image description failed: {}", e)))?;

        Ok(ToolResult::success(result))
    }
}

/// Prepare image data - handles file paths, URLs, and base64
async fn prepare_image(image: &str, default_media_type: &str) -> Result<(String, String), ToolError> {
    // Check if it's a file path
    let path = Path::new(image);
    if path.exists() && path.is_file() {
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read image file: {}", e)))?;

        // Detect media type from extension
        let media_type = match path.extension().and_then(|e| e.to_str()) {
            Some("jpg") | Some("jpeg") => "image/jpeg",
            Some("png") => "image/png",
            Some("gif") => "image/gif",
            Some("webp") => "image/webp",
            Some("bmp") => "image/bmp",
            Some("svg") => "image/svg+xml",
            _ => default_media_type,
        };

        let base64_data = base64_simd::STANDARD.encode_to_string(&bytes);
        return Ok((base64_data, media_type.to_string()));
    }

    // Check if it's a URL
    if image.starts_with("http://") || image.starts_with("https://") {
        // Return URL as-is - vision models typically accept URLs directly
        return Ok((image.to_string(), default_media_type.to_string()));
    }

    // Assume it's already base64
    Ok((image.to_string(), default_media_type.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ocr_tool_definition() {
        let tool = OcrTool::new();
        let def = tool.definition();
        assert_eq!(def.name, "ocr");
    }

    #[tokio::test]
    async fn test_describe_image_tool_definition() {
        let tool = DescribeImageTool::new();
        let def = tool.definition();
        assert_eq!(def.name, "describe_image");
    }
}
