//! PDF tools - read text and extract images from PDFs

use crate::{Tool, ToolDefinition, ToolError, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Callback for analyzing images (reuse vision callback)
pub type PdfVisionFn = Arc<
    dyn Fn(String, String, String) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

/// Tool for reading text from PDF files
pub struct ReadPdfTool {
    vision_fn: Option<PdfVisionFn>,
}

impl ReadPdfTool {
    #[must_use]
    pub fn new() -> Self {
        Self { vision_fn: None }
    }

    /// Set vision function for analyzing embedded images.
    #[must_use]
    pub fn with_vision_fn(mut self, f: PdfVisionFn) -> Self {
        self.vision_fn = Some(f);
        self
    }
}

impl Default for ReadPdfTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ReadPdfTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("read_pdf", "Read text content from a PDF file. Can also extract and analyze images.")
            .string_param("path", "Path to the PDF file", true)
            .bool_param("extract_images", "Whether to extract and analyze embedded images (default: false)", false)
            .int_param("max_pages", "Maximum pages to read (default: all)", false)
            .string_param("image_prompt", "Prompt for analyzing extracted images (default: 'Describe this image')", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let path_str = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'path' parameter".to_string()))?;

        let extract_images = params
            .get("extract_images")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let max_pages = params
            .get("max_pages")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);

        let image_prompt = params
            .get("image_prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("Describe this image in detail.");

        let path = Path::new(path_str);
        if !path.exists() {
            return Err(ToolError::ExecutionFailed(format!("File not found: {}", path_str)));
        }

        // Read PDF bytes
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read file: {}", e)))?;

        // Extract text using pdf-extract
        let text = extract_pdf_text(&bytes, max_pages)?;

        let mut result = format!("# PDF Content: {}\n\n{}", path_str, text);

        // Extract and analyze images if requested
        if extract_images {
            if let Some(ref vision_fn) = self.vision_fn {
                let images = extract_pdf_images(&bytes, max_pages)?;
                
                if !images.is_empty() {
                    result.push_str("\n\n## Extracted Images\n\n");
                    
                    for (i, (image_data, media_type)) in images.into_iter().enumerate() {
                        let base64_image = base64_simd::STANDARD.encode_to_string(&image_data);
                        
                        match vision_fn(base64_image, image_prompt.to_string(), media_type).await {
                            Ok(description) => {
                                result.push_str(&format!("### Image {}\n{}\n\n", i + 1, description));
                            }
                            Err(e) => {
                                result.push_str(&format!("### Image {} (analysis failed)\nError: {}\n\n", i + 1, e));
                            }
                        }
                    }
                }
            } else {
                result.push_str("\n\n*Note: Image extraction requested but vision model not configured.*");
            }
        }

        Ok(ToolResult::success(result))
    }
}

/// Extract text from PDF bytes
fn extract_pdf_text(bytes: &[u8], max_pages: Option<usize>) -> Result<String, ToolError> {
    use lopdf::Document;

    let doc = Document::load_mem(bytes)
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to parse PDF: {}", e)))?;

    let pages = doc.get_pages();
    let page_count = pages.len();
    let pages_to_read = max_pages.unwrap_or(page_count).min(page_count);

    let mut text = String::new();
    text.push_str(&format!("*{} pages total, reading {}*\n\n", page_count, pages_to_read));

    // Collect page numbers for extraction
    let page_numbers: Vec<u32> = pages.keys().copied().take(pages_to_read).collect();
    
    for page_num in &page_numbers {
        text.push_str(&format!("--- Page {} ---\n", page_num));
        
        match doc.extract_text(&[*page_num]) {
            Ok(page_text) => {
                let cleaned = page_text.trim();
                if cleaned.is_empty() {
                    text.push_str("*[No extractable text - may contain images only]*\n");
                } else {
                    text.push_str(cleaned);
                    text.push('\n');
                }
            }
            Err(e) => {
                text.push_str(&format!("*[Failed to extract: {}]*\n", e));
            }
        }
        text.push('\n');
    }

    if pages_to_read < page_count {
        text.push_str(&format!("\n*... {} more pages not shown*", page_count - pages_to_read));
    }

    Ok(text)
}

/// Extract images from PDF bytes
/// Returns Vec<(image_bytes, media_type)>
fn extract_pdf_images(bytes: &[u8], _max_pages: Option<usize>) -> Result<Vec<(Vec<u8>, String)>, ToolError> {
    use lopdf::{Document, Object};

    let doc = Document::load_mem(bytes)
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to parse PDF: {}", e)))?;

    let mut images = Vec::new();

    // Iterate through objects looking for images
    for (_obj_id, object) in doc.objects.iter() {
        if images.len() >= 20 {
            // Limit to 20 images
            break;
        }

        if let Object::Stream(stream) = object {
            let dict = &stream.dict;
            
            // Check if this is an image
            let is_image = dict.get(b"Subtype")
                .map(|o| matches!(o, Object::Name(n) if n == b"Image"))
                .unwrap_or(false);

            if is_image {
                // Try to get the image data
                if let Ok(data) = stream.decompressed_content() {
                    // Determine image type from filter
                    let media_type = dict.get(b"Filter")
                        .map(|f| match f {
                            Object::Name(n) if n == b"DCTDecode" => "image/jpeg",
                            Object::Name(n) if n == b"FlateDecode" => "image/png",
                            Object::Name(n) if n == b"JPXDecode" => "image/jp2",
                            _ => "image/png",
                        })
                        .unwrap_or("image/png");

                    images.push((data, media_type.to_string()));
                }
            }
        }
    }

    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_read_pdf_tool_definition() {
        let tool = ReadPdfTool::new();
        let def = tool.definition();
        assert_eq!(def.name, "read_pdf");
        assert!(!def.parameters.is_empty());
    }
}
