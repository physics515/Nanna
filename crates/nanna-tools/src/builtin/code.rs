//! Code analysis tools for token-efficient codebase understanding
//!
//! These tools return compact summaries of code structure rather than full file contents,
//! saving significant context space when the agent only needs to understand structure.

use crate::{Tool, ToolDefinition, ToolError, ToolResult};
use async_trait::async_trait;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use walkdir::WalkDir;

/// Patterns to ignore when walking directories
const IGNORE_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".git",
    ".hg",
    ".svn",
    "__pycache__",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    "dist",
    "build",
    ".next",
    ".nuxt",
    "vendor",
    ".cargo",
    "coverage",
    ".output",
    ".cache",
    ".parcel-cache",
    "bower_components",
    ".turbo",
    "out",
    ".vercel",
    ".svelte-kit",
];

/// Check if a path component should be ignored
fn should_ignore(name: &str) -> bool {
    IGNORE_DIRS.contains(&name)
}

// ── code_outline ──────────────────────────────────────────────────────────────

/// Returns function signatures, struct/enum/trait definitions, and impl blocks for a file.
/// Output is typically 5-20% the size of the full file.
pub struct CodeOutlineTool;

impl CodeOutlineTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CodeOutlineTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for CodeOutlineTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "code_outline",
            "Extract function signatures, struct/enum/trait definitions, and impl blocks from a \
             source file. Returns a compact outline (~5-20% of file size). Supports Rust, Python, \
             TypeScript, JavaScript, and Go.",
        )
        .string_param("path", "Path to the source file to outline.", true)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let path = params
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidParams("'path' is required".into()))?;

        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| ToolError::Io(e))?;

        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let outline = extract_outline(&content, ext);

        if outline.is_empty() {
            Ok(ToolResult::success(format!(
                "No definitions found in {} (unsupported language or empty file)",
                path
            )))
        } else {
            let line_count = content.lines().count();
            let outline_lines = outline.lines().count();
            Ok(ToolResult::success(format!(
                "# Outline of {} ({} lines -> {} outline lines, {:.0}% reduction)\n\n{}",
                path,
                line_count,
                outline_lines,
                (1.0 - outline_lines as f64 / line_count.max(1) as f64) * 100.0,
                outline
            )))
        }
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(10)
    }
}

/// Extract code outline based on language
fn extract_outline(content: &str, ext: &str) -> String {
    match ext {
        "rs" => extract_rust_outline(content),
        "py" => extract_python_outline(content),
        "ts" | "tsx" | "js" | "jsx" => extract_ts_outline(content),
        "go" => extract_go_outline(content),
        _ => extract_generic_outline(content),
    }
}

fn extract_rust_outline(content: &str) -> String {
    let patterns = [
        r"^\s*(pub\s+)?(async\s+)?fn\s+\w+",       // functions
        r"^\s*(pub\s+)?struct\s+\w+",                // structs
        r"^\s*(pub\s+)?enum\s+\w+",                  // enums
        r"^\s*(pub\s+)?trait\s+\w+",                  // traits
        r"^\s*(pub\s+)?type\s+\w+",                   // type aliases
        r"^\s*impl(<[^>]*>)?\s+\w+",                  // impl blocks
        r"^\s*(pub\s+)?mod\s+\w+",                    // modules
        r"^\s*(pub\s+)?const\s+\w+",                  // constants
        r"^\s*(pub\s+)?static\s+\w+",                 // statics
        r"^\s*use\s+",                                 // use statements
        r"^\s*#\[derive",                              // derive macros
    ];
    extract_by_patterns(content, &patterns)
}

fn extract_python_outline(content: &str) -> String {
    let patterns = [
        r"^\s*(async\s+)?def\s+\w+",    // functions
        r"^\s*class\s+\w+",             // classes
        r"^(import|from)\s+",            // imports
        r"^\s*@\w+",                      // decorators
    ];
    extract_by_patterns(content, &patterns)
}

fn extract_ts_outline(content: &str) -> String {
    let patterns = [
        r"^\s*(export\s+)?(async\s+)?function\s+\w+", // functions
        r"^\s*(export\s+)?(default\s+)?class\s+\w+",   // classes
        r"^\s*(export\s+)?interface\s+\w+",             // interfaces
        r"^\s*(export\s+)?type\s+\w+",                  // type aliases
        r"^\s*(export\s+)?enum\s+\w+",                  // enums
        r"^\s*(export\s+)?const\s+\w+",                 // const exports
        r"^import\s+",                                   // imports
    ];
    extract_by_patterns(content, &patterns)
}

fn extract_go_outline(content: &str) -> String {
    let patterns = [
        r"^func\s+(\(\w+\s+\*?\w+\)\s+)?\w+", // functions and methods
        r"^type\s+\w+\s+(struct|interface)",     // types
        r"^package\s+\w+",                        // package
        r"^import\s+",                             // imports
        r"^var\s+\w+",                             // var declarations
        r"^const\s+",                              // constants
    ];
    extract_by_patterns(content, &patterns)
}

fn extract_generic_outline(content: &str) -> String {
    // Fallback: extract lines that look like definitions
    let patterns = [
        r"^\s*(pub\s+|export\s+|public\s+)?(async\s+)?(fn|function|def|func|class|struct|enum|trait|interface|type|impl)\s+\w+",
    ];
    extract_by_patterns(content, &patterns)
}

fn extract_by_patterns(content: &str, patterns: &[&str]) -> String {
    let regexes: Vec<Regex> = patterns
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect();

    let mut result = Vec::new();
    let mut prev_matched = false;

    for (i, line) in content.lines().enumerate() {
        let matches = regexes.iter().any(|r| r.is_match(line));

        if matches {
            if !prev_matched && !result.is_empty() {
                result.push(String::new()); // blank separator
            }
            result.push(format!("{:>4} | {}", i + 1, line));
            prev_matched = true;
        } else {
            // Include brace-only lines immediately after a match for context
            let trimmed = line.trim();
            if prev_matched && (trimmed == "{" || trimmed == "}" || trimmed == "{}" || trimmed.starts_with("where")) {
                result.push(format!("{:>4} | {}", i + 1, line));
            } else {
                prev_matched = false;
            }
        }
    }

    result.join("\n")
}

// ── code_search ──────────────────────────────────────────────────────────────

/// Regex search across files with context lines.
pub struct CodeSearchTool;

impl CodeSearchTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CodeSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for CodeSearchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "code_search",
            "Search for a regex pattern across files in a directory. Returns matching lines with \
             context. More efficient than reading entire files when looking for specific patterns.",
        )
        .string_param("pattern", "Regex pattern to search for.", true)
        .string_param("path", "Directory to search in (default: current dir).", false)
        .string_param("file_pattern", "Glob pattern to filter files (e.g. '*.rs', '*.py').", false)
        .int_param("context_lines", "Lines of context before/after each match (default: 2).", false)
        .int_param("max_results", "Maximum number of matches to return (default: 50).", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let pattern_str = params
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidParams("'pattern' is required".into()))?;

        let search_path = params
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".");

        let file_pattern = params
            .get("file_pattern")
            .and_then(Value::as_str);

        let context_lines = params
            .get("context_lines")
            .and_then(Value::as_u64)
            .unwrap_or(2) as usize;

        let max_results = params
            .get("max_results")
            .and_then(Value::as_u64)
            .unwrap_or(50) as usize;

        let re = Regex::new(pattern_str)
            .map_err(|e| ToolError::InvalidParams(format!("Invalid regex: {e}")))?;

        let glob_pattern = file_pattern.map(|p| {
            glob::Pattern::new(p).ok()
        }).flatten();

        let mut results = Vec::new();
        let mut files_searched = 0;
        let mut total_matches = 0;

        for entry in WalkDir::new(search_path)
            .into_iter()
            .filter_entry(|e| {
                e.file_name()
                    .to_str()
                    .map(|name| !should_ignore(name))
                    .unwrap_or(true)
            })
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();

            // Apply file pattern filter
            if let Some(ref glob) = glob_pattern {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if !glob.matches(name) {
                        continue;
                    }
                }
            }

            // Skip binary files (check first 512 bytes)
            if let Ok(bytes) = std::fs::read(path) {
                if bytes.len() > 512 && bytes[..512].contains(&0) {
                    continue;
                }
            }

            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue, // Skip unreadable files
            };

            // Skip likely minified/bundled files (any line > 500 chars)
            if content.lines().any(|l| l.len() > 500) {
                continue;
            }

            files_searched += 1;
            let lines: Vec<&str> = content.lines().collect();

            for (i, line) in lines.iter().enumerate() {
                if re.is_match(line) {
                    total_matches += 1;
                    if results.len() >= max_results {
                        break;
                    }

                    let start = i.saturating_sub(context_lines);
                    let end = (i + context_lines + 1).min(lines.len());

                    let mut match_block = format!("{}:{}\n", path.display(), i + 1);
                    for j in start..end {
                        let marker = if j == i { ">" } else { " " };
                        match_block.push_str(&format!("{} {:>4} | {}\n", marker, j + 1, lines[j]));
                    }
                    results.push(match_block);
                }
            }

            if results.len() >= max_results {
                break;
            }
        }

        let header = format!(
            "Found {} matches in {} files searched (showing {})\n\n",
            total_matches,
            files_searched,
            results.len().min(max_results)
        );

        Ok(ToolResult::success(format!("{}{}", header, results.join("\n"))))
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(30)
    }
}

// ── project_structure ─────────────────────────────────────────────────────────

/// Directory tree with file sizes and line counts.
pub struct ProjectStructureTool;

impl ProjectStructureTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ProjectStructureTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ProjectStructureTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "project_structure",
            "Show directory tree with file sizes and line counts. Gives a compact overview of \
             project structure without reading file contents.",
        )
        .string_param("path", "Root directory to scan (default: current dir).", false)
        .int_param("max_depth", "Maximum directory depth (default: 3).", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let root = params
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".");

        let max_depth = params
            .get("max_depth")
            .and_then(Value::as_u64)
            .unwrap_or(3) as usize;

        let root_path = Path::new(root);
        if !root_path.exists() {
            return Ok(ToolResult::error(format!("Path does not exist: {}", root)));
        }

        let mut output = Vec::new();
        let mut total_files = 0u64;
        let mut total_size = 0u64;
        let mut total_lines = 0u64;

        output.push(format!("{}/", root));

        for entry in WalkDir::new(root)
            .max_depth(max_depth)
            .into_iter()
            .filter_entry(|e| {
                e.file_name()
                    .to_str()
                    .map(|name| !should_ignore(name))
                    .unwrap_or(true)
            })
            .filter_map(Result::ok)
            .skip(1) // skip root
        {
            let depth = entry.depth();
            let indent = "  ".repeat(depth);
            let name = entry.file_name().to_string_lossy();

            if entry.file_type().is_dir() {
                output.push(format!("{}{}/", indent, name));
            } else if entry.file_type().is_file() {
                let metadata = entry.metadata().ok();
                let size = metadata.as_ref().map_or(0, |m| m.len());
                total_files += 1;
                total_size += size;

                // Count lines for text files (skip large files)
                let lines = if size < 1_000_000 {
                    std::fs::read_to_string(entry.path())
                        .ok()
                        .map(|c| {
                            let l = c.lines().count() as u64;
                            total_lines += l;
                            l
                        })
                } else {
                    None
                };

                let size_str = format_size(size);
                let line_str = lines
                    .map(|l| format!(", {} lines", l))
                    .unwrap_or_default();

                output.push(format!("{}{} ({}{})", indent, name, size_str, line_str));
            }
        }

        output.push(String::new());
        output.push(format!(
            "Total: {} files, {}, {} lines",
            total_files,
            format_size(total_size),
            total_lines
        ));

        Ok(ToolResult::success(output.join("\n")))
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(15)
    }
}

/// Format byte size to human-readable
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;

    if bytes >= MB {
        format!("{:.1}MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}KB", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}
