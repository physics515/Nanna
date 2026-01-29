//! Web-related tools

use crate::{Tool, ToolDefinition, ToolError, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

/// Web search tool (stub - requires API key)
pub struct WebSearchTool {
    pub api_key: Option<String>,
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self { api_key: None }
    }

    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("web_search", "Search the web using a search API")
            .string_param("query", "Search query string", true)
            .int_param("count", "Number of results to return (1-10)", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'query' parameter".to_string()))?;

        let _count = params
            .get("count")
            .and_then(|v| v.as_u64())
            .unwrap_or(5)
            .min(10);

        // TODO: Implement actual search API (Brave, Serper, etc.)
        if self.api_key.is_none() {
            return Err(ToolError::ExecutionFailed(
                "Web search API key not configured".to_string(),
            ));
        }

        // Placeholder response
        Ok(ToolResult::error(format!(
            "Web search not yet implemented. Query: {}",
            query
        )))
    }
}

/// Fetch web page content
pub struct WebFetchTool {
    pub max_size: usize,
    pub timeout_secs: u64,
}

impl WebFetchTool {
    pub fn new() -> Self {
        Self {
            max_size: 1024 * 1024, // 1MB
            timeout_secs: 30,
        }
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("web_fetch", "Fetch and extract readable content from a URL")
            .string_param("url", "HTTP or HTTPS URL to fetch", true)
            .int_param("max_chars", "Maximum characters to return", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'url' parameter".to_string()))?;

        let max_chars = params
            .get("max_chars")
            .and_then(|v| v.as_u64())
            .unwrap_or(50000) as usize;

        // Validate URL
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(ToolError::InvalidParams(
                "URL must start with http:// or https://".to_string(),
            ));
        }

        // TODO: Use reqwest to fetch, then readability/html2text to extract
        // For now, just fetch raw HTML

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(self.timeout_secs))
            .build()
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to create HTTP client: {}", e)))?;

        let response = client
            .get(url)
            .header("User-Agent", "Mozilla/5.0 (compatible; Nanna/1.0)")
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            return Err(ToolError::ExecutionFailed(format!(
                "HTTP error: {}",
                status
            )));
        }

        let content = response
            .text()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read response: {}", e)))?;

        // Basic HTML stripping (TODO: use proper readability extraction)
        let text = strip_html_basic(&content);
        let truncated = if text.len() > max_chars {
            format!("{}...\n\n[Truncated at {} chars]", &text[..max_chars], max_chars)
        } else {
            text
        };

        Ok(ToolResult::success(truncated).with_data(serde_json::json!({
            "url": url,
            "status": status.as_u16(),
            "original_length": content.len(),
        })))
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(self.timeout_secs)
    }
}

/// Basic HTML tag stripping (very naive, should use a proper library)
fn strip_html_basic(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let in_script = false;
    let in_style = false;

    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
            }
            '>' => {
                in_tag = false;
            }
            _ if in_tag => {
                // Check for script/style tags
                // This is very naive
            }
            _ if !in_script && !in_style => {
                result.push(c);
            }
            _ => {}
        }
    }

    // Clean up whitespace
    let lines: Vec<&str> = result
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    lines.join("\n")
}
