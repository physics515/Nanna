//! Browser backend wiring for tools
//!
//! Provides helpers to connect browser tools to actual browser backends
//! (CDP via chromiumoxide or Playwright).

use super::browser::{BrowserActionTool, BrowserEvaluateTool, BrowserExtractTool, BrowserFn, BrowserScreenshotTool};
use nanna_browser::{Browser, BrowserConfig, BrowserError, BrowserPage, ScreenshotOptions, ImageFormat};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
// Tracing available for future use
#[allow(unused_imports)]
use tracing::debug;

/// Browser manager that maintains a browser instance for tool use.
pub struct BrowserManager {
    browser: Arc<dyn Browser>,
    // Cache the current page for multi-step interactions
    current_page: RwLock<Option<Arc<dyn BrowserPage>>>,
}

impl BrowserManager {
    /// Create a new browser manager with the given browser instance.
    pub fn new(browser: Arc<dyn Browser>) -> Self {
        Self {
            browser,
            current_page: RwLock::new(None),
        }
    }

    /// Create from config using the default backend.
    ///
    /// # Errors
    ///
    /// Returns `BrowserError` if the browser cannot be created.
    pub fn from_config(config: BrowserConfig) -> Result<Self, BrowserError> {
        let browser = nanna_browser::create_browser(config)?;
        Ok(Self::new(browser))
    }

    /// Get or create a page for the given URL.
    async fn get_page(&self, url: &str) -> Result<Arc<dyn BrowserPage>, BrowserError> {
        // Check if we have a cached page at the same URL
        let cached = self.current_page.read().await;
        if cached.is_some() {
            // If same URL, reuse
            // Note: page.url() might not work well, so we always navigate
            drop(cached);
        } else {
            drop(cached);
        }

        // Navigate to URL
        let page = self.browser.navigate(url).await?;
        
        // Cache it
        let mut guard = self.current_page.write().await;
        *guard = Some(page.clone());
        
        Ok(page)
    }

    /// Take a screenshot.
    pub async fn screenshot(
        &self,
        url: &str,
        params: &HashMap<String, Value>,
    ) -> Result<Vec<u8>, String> {
        let page = self.get_page(url).await.map_err(|e| e.to_string())?;

        let full_page = params
            .get("full_page")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let format = params
            .get("format")
            .and_then(|v| v.as_str())
            .map(|f| match f.to_lowercase().as_str() {
                "jpeg" | "jpg" => ImageFormat::Jpeg,
                _ => ImageFormat::Png,
            })
            .unwrap_or(ImageFormat::Png);

        let options = ScreenshotOptions {
            full_page,
            format,
            quality: params.get("quality").and_then(|v| v.as_u64()).map(|q| q as u8),
            selector: params.get("selector").and_then(|v| v.as_str()).map(String::from),
        };

        page.screenshot(options).await.map_err(|e| e.to_string())
    }

    /// Extract content from a page.
    pub async fn extract(
        &self,
        url: &str,
        params: &HashMap<String, Value>,
    ) -> Result<String, String> {
        let page = self.get_page(url).await.map_err(|e| e.to_string())?;

        let mode = params
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("text");

        let selector = params.get("selector").and_then(|v| v.as_str());

        if let Some(sel) = selector {
            // Extract from specific selector
            let script = match mode {
                "html" => format!(
                    "document.querySelector('{}')?.outerHTML || ''",
                    sel.replace('\'', "\\'")
                ),
                _ => format!(
                    "document.querySelector('{}')?.textContent || ''",
                    sel.replace('\'', "\\'")
                ),
            };
            let result = page.evaluate(&script).await.map_err(|e| e.to_string())?;
            Ok(result.as_str().unwrap_or("").to_string())
        } else {
            // Extract full page
            match mode {
                "html" => page.html().await.map_err(|e| e.to_string()),
                _ => page.text_content().await.map_err(|e| e.to_string()),
            }
        }
    }

    /// Perform an action on a page.
    pub async fn action(
        &self,
        url: &str,
        params: &HashMap<String, Value>,
    ) -> Result<String, String> {
        let page = self.get_page(url).await.map_err(|e| e.to_string())?;

        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or("Missing action")?;

        match action {
            "click" => {
                let selector = params
                    .get("selector")
                    .and_then(|v| v.as_str())
                    .ok_or("Click requires 'selector'")?;
                page.click(selector).await.map_err(|e| e.to_string())?;
                Ok(format!("Clicked '{}'", selector))
            }
            "type" => {
                let selector = params
                    .get("selector")
                    .and_then(|v| v.as_str())
                    .ok_or("Type requires 'selector'")?;
                let text = params
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or("Type requires 'text'")?;
                page.type_text(selector, text).await.map_err(|e| e.to_string())?;
                Ok(format!("Typed into '{}'", selector))
            }
            "fill" => {
                let selector = params
                    .get("selector")
                    .and_then(|v| v.as_str())
                    .ok_or("Fill requires 'selector'")?;
                let text = params
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or("Fill requires 'text'")?;
                page.fill(selector, text).await.map_err(|e| e.to_string())?;
                Ok(format!("Filled '{}'", selector))
            }
            "press" => {
                let selector = params
                    .get("selector")
                    .and_then(|v| v.as_str())
                    .ok_or("Press requires 'selector'")?;
                let key = params
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or("Press requires 'key'")?;
                page.press(selector, key).await.map_err(|e| e.to_string())?;
                Ok(format!("Pressed '{}' on '{}'", key, selector))
            }
            "wait" => {
                let ms = params
                    .get("wait_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1000);
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                Ok(format!("Waited {}ms", ms))
            }
            "wait_selector" => {
                let selector = params
                    .get("selector")
                    .and_then(|v| v.as_str())
                    .ok_or("wait_selector requires 'selector'")?;
                page.wait_for_selector(selector).await.map_err(|e| e.to_string())?;
                Ok(format!("Found '{}'", selector))
            }
            _ => Err(format!("Unknown action: {}", action)),
        }
    }

    /// Evaluate JavaScript on a page.
    pub async fn evaluate(
        &self,
        url: &str,
        params: &HashMap<String, Value>,
    ) -> Result<Value, String> {
        let page = self.get_page(url).await.map_err(|e| e.to_string())?;

        let script = params
            .get("script")
            .and_then(|v| v.as_str())
            .ok_or("Missing script")?;

        page.evaluate(script).await.map_err(|e| e.to_string())
    }

    /// Close the browser.
    pub async fn close(&self) -> Result<(), BrowserError> {
        self.browser.close().await
    }
}

/// Create wired browser tools from a browser manager.
///
/// Returns a tuple of (screenshot, extract, action, evaluate) tools.
pub fn create_browser_tools(
    manager: Arc<BrowserManager>,
) -> (BrowserScreenshotTool, BrowserExtractTool, BrowserActionTool, BrowserEvaluateTool) {
    // Screenshot tool
    let mgr = manager.clone();
    let screenshot_fn: BrowserFn<Vec<u8>> = Arc::new(move |url, params| {
        let mgr = mgr.clone();
        Box::pin(async move { mgr.screenshot(&url, &params).await })
    });
    let screenshot_tool = BrowserScreenshotTool::new().with_screenshot_fn(screenshot_fn);

    // Extract tool
    let mgr = manager.clone();
    let extract_fn: BrowserFn<String> = Arc::new(move |url, params| {
        let mgr = mgr.clone();
        Box::pin(async move { mgr.extract(&url, &params).await })
    });
    let extract_tool = BrowserExtractTool::new().with_extract_fn(extract_fn);

    // Action tool
    let mgr = manager.clone();
    let action_fn: BrowserFn<String> = Arc::new(move |url, params| {
        let mgr = mgr.clone();
        Box::pin(async move { mgr.action(&url, &params).await })
    });
    let action_tool = BrowserActionTool::new().with_action_fn(action_fn);

    // Evaluate tool
    let mgr = manager.clone();
    let evaluate_fn: BrowserFn<Value> = Arc::new(move |url, params| {
        let mgr = mgr.clone();
        Box::pin(async move { mgr.evaluate(&url, &params).await })
    });
    let evaluate_tool = BrowserEvaluateTool::new().with_evaluate_fn(evaluate_fn);

    (screenshot_tool, extract_tool, action_tool, evaluate_tool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_browser_manager_creation() {
        // Just test that the types are correct - actual browser tests need integration
        let config = BrowserConfig::default();
        // Would need actual browser installed to test:
        // let manager = BrowserManager::from_config(config);
    }
}
