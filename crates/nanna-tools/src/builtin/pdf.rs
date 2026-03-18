//! PDF tools - read text and extract images from PDFs
//!
//! # OCR Fallback
//!
//! `ReadPdfTool` now accepts an optional `OcrFn` callback that mirrors the
//! full tiered OCR pipeline from `OcrTool`.  When `lopdf` extracts an empty
//! (or whitespace-only) page, the tool can call `ocr_fn` on any embedded
//! images to recover text.
//!
//! Wiring the `OcrFn` is done in the daemon/GUI layer by passing in an async
//! closure that calls the full `OcrTool` pipeline.
//!
//! ## Future: pdfium page rendering
//! Rendering a whole PDF page to pixels (as opposed to extracting embedded
//! image objects) requires a PDF rendering library such as `pdfium-render`.
//! This is not implemented here to avoid a large C dependency; instead, we
//! fall back to extracting embedded image objects from the PDF stream.

use crate::{Tool, ToolDefinition, ToolError, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Public type aliases
// ---------------------------------------------------------------------------

/// Async callback for analyzing images embedded in a PDF (vision model or OCR).
///
/// Arguments: `(base64_image_data, prompt, media_type)` → `Result<text, err_msg>`
pub type PdfVisionFn = Arc<
    dyn Fn(String, String, String) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

/// Full OCR pipeline callback — same signature as `PdfVisionFn`.
///
/// When set on `ReadPdfTool`, this is called for pages that have no
/// extractable text but do contain embedded image objects.  The daemon
/// wires this to the tiered `OcrTool` pipeline.
pub type OcrFn = Arc<
    dyn Fn(String, String, String) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

// ---------------------------------------------------------------------------
// ReadPdfTool
// ---------------------------------------------------------------------------

/// Tool for reading text from PDF files.
pub struct ReadPdfTool {
    /// Vision model for analyzing embedded images (decorative / non-OCR).
    vision_fn: Option<PdfVisionFn>,
    /// Full OCR pipeline callback used when `lopdf` returns empty pages.
    ocr_fn: Option<OcrFn>,
}

impl ReadPdfTool {
    #[must_use]
    pub fn new() -> Self {
        Self {
            vision_fn: None,
            ocr_fn: None,
        }
    }

    /// Set vision function for analyzing embedded images.
    #[must_use]
    pub fn with_vision_fn(mut self, f: PdfVisionFn) -> Self {
        self.vision_fn = Some(f);
        self
    }

    /// Set the OCR pipeline callback used as a fallback for image-only pages.
    ///
    /// When a PDF page contains no extractable text, the tool will attempt to
    /// extract embedded image objects from the page and run them through this
    /// OCR function to recover text.
    #[must_use]
    pub fn with_ocr_fn(mut self, f: OcrFn) -> Self {
        self.ocr_fn = Some(f);
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
        ToolDefinition::new(
            "read_pdf",
            "Read text content from a PDF file. Can also extract and analyze images.",
        )
        .string_param("path", "Path to the PDF file", true)
        .bool_param(
            "extract_images",
            "Whether to extract and analyze embedded images (default: false)",
            false,
        )
        .int_param("max_pages", "Maximum pages to read (default: all)", false)
        .string_param(
            "image_prompt",
            "Prompt for analyzing extracted images (default: 'Describe this image')",
            false,
        )
        .bool_param(
            "ocr_fallback",
            "Use OCR on embedded images when a page has no extractable text (default: true)",
            false,
        )
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

        // OCR fallback is on by default when an OCR function is configured
        let ocr_fallback = params
            .get("ocr_fallback")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let path = Path::new(path_str);
        if !path.exists() {
            return Err(ToolError::ExecutionFailed(format!(
                "File not found: {}",
                path_str
            )));
        }

        // Read PDF bytes
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read file: {}", e)))?;

        // ------------------------------------------------------------------
        // Tier 1: lopdf text extraction
        // ------------------------------------------------------------------
        let (text, empty_pages) = extract_pdf_text_with_empty_pages(&bytes, max_pages)?;
        let mut result = format!("# PDF Content: {}\n\n{}", path_str, text);

        // ------------------------------------------------------------------
        // Tier 2: OCR fallback for empty pages
        // ------------------------------------------------------------------
        if ocr_fallback && !empty_pages.is_empty() {
            if let Some(ref ocr_fn) = self.ocr_fn {
                let all_images = extract_pdf_images(&bytes, max_pages)?;
                if !all_images.is_empty() {
                    result.push_str("\n\n## OCR Text (from image-only pages)\n\n");
                    for (i, (image_data, media_type)) in all_images.into_iter().enumerate() {
                        let b64 = base64_simd::STANDARD.encode_to_string(&image_data);
                        let ocr_prompt = "Extract ALL text from this image using OCR. Output the extracted text only.".to_string();
                        match ocr_fn(b64, ocr_prompt, media_type).await {
                            Ok(t) if !t.trim().is_empty() => {
                                result.push_str(&format!("### Image {} (OCR)\n{}\n\n", i + 1, t));
                            }
                            Ok(_) => {}
                            Err(e) => {
                                result.push_str(&format!(
                                    "### Image {} (OCR failed)\nError: {}\n\n",
                                    i + 1,
                                    e
                                ));
                            }
                        }
                    }
                } else {
                    result.push_str("\n\n*Note: Some pages had no extractable text and no embedded images were found for OCR fallback.*");
                }
            } else {
                result.push_str(&format!(
                    "\n\n*Note: {} page(s) had no extractable text. Configure an OCR pipeline to recover text from image-only pages.*",
                    empty_pages.len()
                ));
            }
        }

        // ------------------------------------------------------------------
        // Optional: extract + analyze embedded images via vision model
        // ------------------------------------------------------------------
        if extract_images {
            if let Some(ref vision_fn) = self.vision_fn {
                let images = extract_pdf_images(&bytes, max_pages)?;

                if !images.is_empty() {
                    result.push_str("\n\n## Extracted Images\n\n");

                    for (i, (image_data, media_type)) in images.into_iter().enumerate() {
                        let base64_image =
                            base64_simd::STANDARD.encode_to_string(&image_data);

                        match vision_fn(base64_image, image_prompt.to_string(), media_type).await {
                            Ok(description) => {
                                result.push_str(&format!(
                                    "### Image {}\n{}\n\n",
                                    i + 1,
                                    description
                                ));
                            }
                            Err(e) => {
                                result.push_str(&format!(
                                    "### Image {} (analysis failed)\nError: {}\n\n",
                                    i + 1,
                                    e
                                ));
                            }
                        }
                    }
                }
            } else {
                result.push_str(
                    "\n\n*Note: Image extraction requested but vision model not configured.*",
                );
            }
        }

        Ok(ToolResult::success(result))
    }
}

// ---------------------------------------------------------------------------
// lopdf helpers
// ---------------------------------------------------------------------------

/// Extract text from PDF bytes, returning:
/// - The formatted text string (with page headers)
/// - A list of page numbers that produced empty text (candidates for OCR)
fn extract_pdf_text_with_empty_pages(
    bytes: &[u8],
    max_pages: Option<usize>,
) -> Result<(String, Vec<u32>), ToolError> {
    use lopdf::Document;

    let doc = Document::load_mem(bytes)
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to parse PDF: {}", e)))?;

    let pages = doc.get_pages();
    let page_count = pages.len();
    let pages_to_read = max_pages.unwrap_or(page_count).min(page_count);

    let mut text = String::new();
    text.push_str(&format!(
        "*{} pages total, reading {}*\n\n",
        page_count, pages_to_read
    ));

    let mut empty_pages: Vec<u32> = Vec::new();
    let page_numbers: Vec<u32> = pages.keys().copied().take(pages_to_read).collect();

    for page_num in &page_numbers {
        text.push_str(&format!("--- Page {} ---\n", page_num));

        match doc.extract_text(&[*page_num]) {
            Ok(page_text) => {
                let cleaned = page_text.trim();
                if cleaned.is_empty() {
                    text.push_str("*[No extractable text — may contain images only]*\n");
                    empty_pages.push(*page_num);
                } else {
                    text.push_str(cleaned);
                    text.push('\n');
                }
            }
            Err(e) => {
                text.push_str(&format!("*[Failed to extract: {}]*\n", e));
                empty_pages.push(*page_num);
            }
        }
        text.push('\n');
    }

    if pages_to_read < page_count {
        text.push_str(&format!(
            "\n*... {} more pages not shown*",
            page_count - pages_to_read
        ));
    }

    Ok((text, empty_pages))
}

/// Extract images from PDF bytes.
/// Returns `Vec<(image_bytes, media_type)>`.
fn extract_pdf_images(
    bytes: &[u8],
    _max_pages: Option<usize>,
) -> Result<Vec<(Vec<u8>, String)>, ToolError> {
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
            let is_image = dict
                .get(b"Subtype")
                .map(|o| matches!(o, Object::Name(n) if n == b"Image"))
                .unwrap_or(false);

            if is_image {
                // Try to get the image data
                if let Ok(data) = stream.decompressed_content() {
                    // Determine image type from filter
                    let media_type = dict
                        .get(b"Filter")
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
