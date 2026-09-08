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
// Page selection
// ---------------------------------------------------------------------------

/// Which pages of a document to read.
///
/// The `read_pdf` skill has always sent a `pages` string (`"1-5"`, `"3"`) while
/// the tool only ever looked for an integer `max_pages`, so the selection was
/// parsed by the skill, serialised, and silently discarded: asking for page 3
/// of a forty-page contract returned all forty and never said the request had
/// been ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageSelection {
    /// Every page in the document.
    All,
    /// The first `n` pages — the legacy `max_pages` parameter.
    First(usize),
    /// An inclusive, 1-based range of page positions.
    Range { first: usize, last: usize },
}

impl PageSelection {
    /// The 1-based positions this selection admits in a document of
    /// `page_count` pages.
    ///
    /// Returns an empty range when the selection starts past the end, so a
    /// caller asking for page 90 of an 80-page file gets nothing plus the
    /// counts to see why — not a silent last page.
    fn positions(self, page_count: usize) -> std::ops::RangeInclusive<usize> {
        debug_assert!(page_count < usize::MAX, "page count must be representable");
        let (first, last) = match self {
            Self::All => (1, page_count),
            Self::First(n) => (1, n.min(page_count)),
            Self::Range { first, last } => (first, last.min(page_count)),
        };
        // `1..=0` is already empty, which is the right answer for an empty
        // document; `90..=80` likewise for a selection past the end.
        first..=last
    }
}

/// Parse a `pages` selection string.
///
/// Accepted forms: `"3"` (one page), `"2-5"` (inclusive range), `"4-"` (from
/// page 4 to the end), `"-4"` (up to page 4), and empty/whitespace (all pages).
///
/// # Errors
///
/// Returns a message naming the offending text when the string is none of
/// those, when a page number is zero (pages are 1-based), or when a range runs
/// backwards — reading `"5-2"` as `2-5` would return pages the caller did not
/// ask for, and reading it as empty would look like an empty document.
pub fn parse_page_selection(spec: &str) -> Result<PageSelection, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Ok(PageSelection::All);
    }

    let number = |text: &str| -> Result<usize, String> {
        let n: usize = text
            .trim()
            .parse()
            .map_err(|_| format!("`{spec}` is not a page number or range"))?;
        if n == 0 {
            return Err(format!("`{spec}` names page 0; pages are numbered from 1"));
        }
        Ok(n)
    };

    let Some((head, tail)) = spec.split_once('-') else {
        let only = number(spec)?;
        return Ok(PageSelection::Range {
            first: only,
            last: only,
        });
    };

    let selection = match (head.trim().is_empty(), tail.trim().is_empty()) {
        // "-4": everything up to page 4.
        (true, false) => PageSelection::First(number(tail)?),
        // "4-": page 4 to the end.
        (false, true) => PageSelection::Range {
            first: number(head)?,
            last: usize::MAX,
        },
        // "2-5"
        (false, false) => {
            let first = number(head)?;
            let last = number(tail)?;
            if last < first {
                return Err(format!("`{spec}` runs backwards: {last} is before {first}"));
            }
            PageSelection::Range { first, last }
        }
        // A bare "-".
        (true, true) => return Err(format!("`{spec}` names no pages")),
    };
    Ok(selection)
}

/// Text extracted from a PDF, with the counts a caller needs to page through it.
///
/// Mirrors what `read_file` reports (`total_lines` / `lines_returned`): the
/// caller can see it did not get the whole document without having to infer it
/// from the text.
#[derive(Debug, Clone)]
pub struct PdfExtract {
    /// The formatted text, with a header per page read.
    pub text: String,
    /// Pages in the document.
    pub page_count: usize,
    /// Pages this call actually read.
    pub pages_read: usize,
    /// Page numbers that yielded no text — the OCR-fallback candidates.
    pub empty_pages: Vec<u32>,
}

/// Largest PDF this tool will read, in bytes.
///
/// Deliberately the same 10 MB ceiling `ReadFileTool` already applies, rather
/// than a new number: `read_pdf` must not be a way to load a file that
/// `read_file` refuses.
pub const PDF_MAX_BYTES: usize = 10 * 1024 * 1024;

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

    /// Recover text from image-only pages, or say plainly why none was.
    ///
    /// Silence here would be indistinguishable from a scanned document that
    /// genuinely holds no text, so every branch writes something.
    ///
    /// # Errors
    ///
    /// Returns `ToolError` when image extraction fails.
    async fn append_ocr_fallback(
        &self,
        bytes: &[u8],
        selection: PageSelection,
        enabled: bool,
        empty_pages: &[u32],
        out: &mut String,
    ) -> Result<(), ToolError> {
        if !enabled || empty_pages.is_empty() {
            return Ok(());
        }
        let Some(ref ocr_fn) = self.ocr_fn else {
            out.push_str(&format!(
                "

*Note: {} page(s) had no extractable text. Configure an OCR \
                 pipeline to recover text from image-only pages.*",
                empty_pages.len()
            ));
            return Ok(());
        };

        let images = extract_pdf_images(bytes, selection)?;
        if images.is_empty() {
            out.push_str(
                "

*Note: Some pages had no extractable text and no embedded \
                 images were found for OCR fallback.*",
            );
            return Ok(());
        }

        out.push_str(
            "

## OCR Text (from image-only pages)

",
        );
        for (index, (image_data, media_type)) in images.into_iter().enumerate() {
            let encoded = base64_simd::STANDARD.encode_to_string(&image_data);
            let prompt = "Extract ALL text from this image using OCR. Output the \
                          extracted text only."
                .to_string();
            match ocr_fn(encoded, prompt, media_type).await {
                Ok(text) if !text.trim().is_empty() => {
                    out.push_str(&format!(
                        "### Image {} (OCR)
{text}

",
                        index + 1
                    ));
                }
                Ok(_) => {}
                Err(e) => {
                    out.push_str(&format!(
                        "### Image {} (OCR failed)
Error: {e}

",
                        index + 1
                    ));
                }
            }
        }
        Ok(())
    }

    /// Describe embedded images with the vision model, if one is configured.
    ///
    /// # Errors
    ///
    /// Returns `ToolError` when image extraction fails.
    async fn append_image_descriptions(
        &self,
        bytes: &[u8],
        selection: PageSelection,
        prompt: &str,
        out: &mut String,
    ) -> Result<(), ToolError> {
        let Some(ref vision_fn) = self.vision_fn else {
            out.push_str(
                "

*Note: Image extraction requested but vision model not configured.*",
            );
            return Ok(());
        };

        let images = extract_pdf_images(bytes, selection)?;
        if images.is_empty() {
            return Ok(());
        }

        out.push_str(
            "

## Extracted Images

",
        );
        for (index, (image_data, media_type)) in images.into_iter().enumerate() {
            let encoded = base64_simd::STANDARD.encode_to_string(&image_data);
            match vision_fn(encoded, prompt.to_string(), media_type).await {
                Ok(description) => {
                    out.push_str(&format!(
                        "### Image {}
{description}

",
                        index + 1
                    ));
                }
                Err(e) => {
                    out.push_str(&format!(
                        "### Image {} (analysis failed)
Error: {e}

",
                        index + 1
                    ));
                }
            }
        }
        Ok(())
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
        .string_param(
            "pages",
            "Pages to read: \"3\", \"2-5\", \"4-\", or \"-4\" (default: all)",
            false,
        )
        .int_param(
            "max_pages",
            "Maximum pages to read from the start (superseded by `pages`)",
            false,
        )
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

        // `pages` is what the read_pdf skill has always sent; `max_pages` is
        // the older integer form. When both arrive the range wins: it is the
        // more specific request, and silently preferring the count is exactly
        // the bug this replaces.
        let selection = match params.get("pages").and_then(|v| v.as_str()) {
            Some(spec) => parse_page_selection(spec).map_err(ToolError::InvalidParams)?,
            None => params
                .get("max_pages")
                .and_then(|v| v.as_u64())
                .map_or(PageSelection::All, |n| PageSelection::First(n as usize)),
        };

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

        // Refuse oversized input before parsing, with the same ceiling
        // `ReadFileTool` applies — `read_pdf` must not be a way in for a file
        // `read_file` would turn away.
        let size_bytes = tokio::fs::metadata(path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to stat file: {e}")))?
            .len();
        if size_bytes > PDF_MAX_BYTES as u64 {
            return Err(ToolError::ExecutionFailed(format!(
                "PDF too large: {size_bytes} bytes (max: {PDF_MAX_BYTES} bytes)"
            )));
        }

        // Read PDF bytes
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read file: {}", e)))?;

        // ------------------------------------------------------------------
        // Tier 1: lopdf text extraction
        // ------------------------------------------------------------------
        let extracted = read_pdf_text(&bytes, selection)?;
        let (text, empty_pages) = (extracted.text, extracted.empty_pages);
        let mut result = format!("# PDF Content: {}\n\n{}", path_str, text);

        // Both optional stages live in their own methods: `execute` is the
        // one place a reader looks to see what a call does, and it should not
        // have to scroll past two independent enrichment passes to find out.
        self.append_ocr_fallback(&bytes, selection, ocr_fallback, &empty_pages, &mut result)
            .await?;
        if extract_images {
            self.append_image_descriptions(&bytes, selection, image_prompt, &mut result)
                .await?;
        }

        Ok(ToolResult::success(result))
    }
}

// ---------------------------------------------------------------------------
// lopdf helpers
// ---------------------------------------------------------------------------

/// Extract text from PDF bytes for the given page selection.
///
/// # Errors
///
/// Returns `ToolError::ExecutionFailed` when the bytes are not a parseable PDF.
pub fn read_pdf_text(bytes: &[u8], selection: PageSelection) -> Result<PdfExtract, ToolError> {
    use lopdf::Document;

    // Enforced here, not merely upstream: this is a public entry point, and a
    // caller that skipped its own check should get an error, not a panic.
    if bytes.len() > PDF_MAX_BYTES {
        return Err(ToolError::ExecutionFailed(format!(
            "PDF too large: {} bytes (max: {PDF_MAX_BYTES} bytes)",
            bytes.len()
        )));
    }

    let doc = Document::load_mem(bytes)
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to parse PDF: {e}")))?;

    // `get_pages` is ordered by page number, so position N in this vector is
    // the Nth page of the document — which is what a `pages` range means.
    let all_pages: Vec<u32> = doc.get_pages().keys().copied().collect();
    let page_count = all_pages.len();

    let wanted = selection.positions(page_count);
    let selected: Vec<u32> = all_pages
        .iter()
        .enumerate()
        .filter(|(index, _)| wanted.contains(&(index + 1)))
        .map(|(_, page)| *page)
        .collect();
    let pages_read = selected.len();
    debug_assert!(pages_read <= page_count, "cannot read more than exists");

    let mut text = String::new();
    text.push_str(&format!(
        "*{page_count} pages total, reading {pages_read}*\n\n"
    ));

    // A selection that matched nothing must say so. An empty body plus a
    // "0 pages read" header is otherwise indistinguishable from a document
    // whose pages were all blank.
    if pages_read == 0 && page_count > 0 {
        text.push_str(&format!(
            "*[No pages matched the requested selection; the document has {page_count} pages]*\n"
        ));
    }

    let mut empty_pages: Vec<u32> = Vec::new();
    for page_num in &selected {
        text.push_str(&format!("--- Page {page_num} ---\n"));

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
                text.push_str(&format!("*[Failed to extract: {e}]*\n"));
                empty_pages.push(*page_num);
            }
        }
        text.push('\n');
    }

    // Announce the cut in counts rather than as "N more pages": the latter
    // reads as the tail, which is wrong for a range that skipped the front.
    if pages_read < page_count {
        text.push_str(&format!(
            "\n*... {} of {page_count} pages not shown (selection read {pages_read})*",
            page_count - pages_read
        ));
    }

    Ok(PdfExtract {
        text,
        page_count,
        pages_read,
        empty_pages,
    })
}

/// Extract images from PDF bytes.
/// Returns `Vec<(image_bytes, media_type)>`.
fn extract_pdf_images(
    bytes: &[u8],
    _selection: PageSelection,
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

    #[test]
    fn a_bare_number_selects_exactly_that_page() {
        assert_eq!(
            parse_page_selection("3"),
            Ok(PageSelection::Range { first: 3, last: 3 })
        );
        assert_eq!(
            parse_page_selection("  7  "),
            Ok(PageSelection::Range { first: 7, last: 7 })
        );
    }

    #[test]
    fn ranges_parse_in_every_documented_form() {
        assert_eq!(
            parse_page_selection("2-5"),
            Ok(PageSelection::Range { first: 2, last: 5 })
        );
        assert_eq!(parse_page_selection("-4"), Ok(PageSelection::First(4)));
        assert_eq!(
            parse_page_selection("4-"),
            Ok(PageSelection::Range {
                first: 4,
                last: usize::MAX
            })
        );
        assert_eq!(parse_page_selection(""), Ok(PageSelection::All));
        assert_eq!(parse_page_selection("   "), Ok(PageSelection::All));
    }

    #[test]
    fn a_malformed_selection_is_refused_rather_than_guessed() {
        // Reading "5-2" as 2-5 would return pages nobody asked for.
        assert!(parse_page_selection("5-2").is_err());
        // Pages are 1-based; page 0 is a caller bug worth naming.
        assert!(parse_page_selection("0").is_err());
        assert!(parse_page_selection("0-3").is_err());
        assert!(parse_page_selection("-").is_err());
        assert!(parse_page_selection("first").is_err());
        assert!(parse_page_selection("1-2-3").is_err());
        // The message must quote the input so the failure is actionable.
        let err = parse_page_selection("banana").expect_err("not a range");
        assert!(err.contains("banana"), "unhelpful message: {err}");
    }

    #[test]
    fn positions_clamp_to_the_document_and_never_wrap() {
        assert_eq!(PageSelection::All.positions(4), 1..=4);
        assert_eq!(PageSelection::First(2).positions(4), 1..=2);
        // A cap larger than the document reads the document, not more.
        assert_eq!(PageSelection::First(99).positions(4), 1..=4);
        assert_eq!(
            PageSelection::Range { first: 2, last: 99 }.positions(4),
            2..=4
        );
        // Past the end selects nothing rather than silently the last page.
        let past = PageSelection::Range {
            first: 90,
            last: 95,
        }
        .positions(4);
        assert!(past.is_empty(), "a selection past the end must be empty");
        // An empty document yields nothing for every selection.
        assert!(PageSelection::All.positions(0).is_empty());
    }

    #[test]
    fn an_oversized_document_is_refused_instead_of_parsed() {
        // The ceiling exists so `read_pdf` cannot load what `read_file` turns
        // away. Bytes just past it are refused before lopdf ever sees them —
        // note this input is not a PDF at all, so reaching the parser would
        // produce the wrong error.
        let oversized = vec![0_u8; PDF_MAX_BYTES + 1];
        let err = read_pdf_text(&oversized, PageSelection::All)
            .expect_err("a document past the ceiling must be refused");
        let message = err.to_string();
        assert!(message.contains("too large"), "wrong reason: {message}");
        assert!(
            !message.contains("parse"),
            "the ceiling must be checked before parsing: {message}"
        );
    }
    /// Build a real three-page PDF in memory, each page carrying a distinct
    /// marker, so the selection is proven against a parsed document rather than
    /// against the parser being mocked out.
    fn three_page_pdf() -> Vec<u8> {
        use lopdf::content::{Content, Operation};
        use lopdf::{Document, Object, Stream, dictionary};

        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });

        let mut kids: Vec<Object> = Vec::new();
        for page in 1..=3 {
            let content = Content {
                operations: vec![
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec!["F1".into(), 24.into()]),
                    Operation::new("Td", vec![72.into(), 720.into()]),
                    Operation::new("Tj", vec![Object::string_literal(format!("MARKER{page}"))]),
                    Operation::new("ET", vec![]),
                ],
            };
            let content_id = doc.add_object(Stream::new(
                dictionary! {},
                content.encode().expect("content encodes"),
            ));
            let leaf_id = doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Contents" => content_id,
            });
            kids.push(leaf_id.into());
        }

        let count = i64::try_from(kids.len()).expect("three fits");
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => kids,
                "Count" => count,
                "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            }),
        );
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);

        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("document saves");
        bytes
    }

    #[test]
    fn a_page_range_reads_those_pages_and_only_those() {
        let pdf = three_page_pdf();

        let all = read_pdf_text(&pdf, PageSelection::All).expect("all pages read");
        assert_eq!(all.page_count, 3);
        assert_eq!(all.pages_read, 3);

        // The regression this fixes: `pages: "2"` used to be discarded and the
        // whole document came back. Page 2 must be the ONLY page present.
        let single = parse_page_selection("2").expect("valid selection");
        let one = read_pdf_text(&pdf, single).expect("page 2 read");
        assert_eq!(one.page_count, 3, "the document is still three pages");
        assert_eq!(one.pages_read, 1, "only one page was asked for");
        assert!(!one.text.contains("MARKER1"), "page 1 leaked: {}", one.text);
        assert!(!one.text.contains("MARKER3"), "page 3 leaked: {}", one.text);

        let span = parse_page_selection("2-3").expect("valid selection");
        let two = read_pdf_text(&pdf, span).expect("pages 2-3 read");
        assert_eq!(two.pages_read, 2);
        assert!(
            two.text.contains("not shown"),
            "a partial read must announce the cut: {}",
            two.text
        );
    }

    #[test]
    fn a_selection_past_the_end_says_so_instead_of_returning_a_page() {
        let pdf = three_page_pdf();
        let past = parse_page_selection("9").expect("valid selection");
        let extract = read_pdf_text(&pdf, past).expect("read succeeds");

        assert_eq!(extract.pages_read, 0, "nothing matched");
        assert_eq!(extract.page_count, 3);
        assert!(
            extract.text.contains("No pages matched"),
            "an empty selection must be distinguishable from a blank document: {}",
            extract.text
        );
        assert!(!extract.text.contains("MARKER"), "no page may leak");
    }
}
