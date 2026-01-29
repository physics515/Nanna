#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! Browser automation for Nanna
//!
//! Supports multiple backends:
//! - **Playwright** (default): Multi-browser support (Chromium, Firefox, WebKit)
//! - **CDP**: Direct Chrome DevTools Protocol via chromiumoxide

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

#[cfg(feature = "cdp")]
pub mod cdp;

#[cfg(feature = "playwright")]
pub mod playwright;

#[derive(Error, Debug)]
pub enum BrowserError {
    #[error("Browser not initialized")]
    NotInitialized,
    #[error("Browser launch failed: {0}")]
    LaunchFailed(String),
    #[error("Navigation failed: {0}")]
    NavigationFailed(String),
    #[error("Screenshot failed: {0}")]
    ScreenshotFailed(String),
    #[error("Element not found: {0}")]
    ElementNotFound(String),
    #[error("Execution failed: {0}")]
    ExecutionFailed(String),
    #[error("Timeout")]
    Timeout,
    #[error("Unsupported browser: {0}")]
    UnsupportedBrowser(String),
}

/// Supported browser types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserType {
    Chromium,
    Firefox,
    Webkit,
}

impl Default for BrowserType {
    fn default() -> Self {
        Self::Chromium
    }
}

impl std::fmt::Display for BrowserType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Chromium => write!(f, "chromium"),
            Self::Firefox => write!(f, "firefox"),
            Self::Webkit => write!(f, "webkit"),
        }
    }
}

/// Browser configuration
#[derive(Debug, Clone)]
pub struct BrowserConfig {
    /// Browser type (Chromium, Firefox, WebKit)
    pub browser_type: BrowserType,
    /// Run in headless mode
    pub headless: bool,
    /// Default viewport width
    pub viewport_width: u32,
    /// Default viewport height
    pub viewport_height: u32,
    /// Navigation timeout in milliseconds
    pub timeout_ms: u64,
    /// Custom executable path (None = auto-detect)
    pub executable_path: Option<String>,
    /// Additional launch arguments
    pub args: Vec<String>,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            browser_type: BrowserType::Chromium,
            headless: true,
            viewport_width: 1920,
            viewport_height: 1080,
            timeout_ms: 30_000,
            executable_path: None,
            args: Vec::new(),
        }
    }
}

impl BrowserConfig {
    #[must_use]
    pub fn chromium() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn firefox() -> Self {
        Self {
            browser_type: BrowserType::Firefox,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn webkit() -> Self {
        Self {
            browser_type: BrowserType::Webkit,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn headless(mut self, headless: bool) -> Self {
        self.headless = headless;
        self
    }

    #[must_use]
    pub fn viewport(mut self, width: u32, height: u32) -> Self {
        self.viewport_width = width;
        self.viewport_height = height;
        self
    }

    #[must_use]
    pub fn timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }
}

/// Screenshot options
#[derive(Debug, Clone, Default)]
pub struct ScreenshotOptions {
    /// Capture full page (not just viewport)
    pub full_page: bool,
    /// Image format
    pub format: ImageFormat,
    /// Quality (1-100, for JPEG)
    pub quality: Option<u8>,
    /// CSS selector to screenshot (None = full page)
    pub selector: Option<String>,
}

/// Image format for screenshots
#[derive(Debug, Clone, Copy, Default)]
pub enum ImageFormat {
    #[default]
    Png,
    Jpeg,
}

/// Page handle returned by browser operations
#[async_trait]
pub trait BrowserPage: Send + Sync {
    /// Get the page URL
    fn url(&self) -> &str;

    /// Navigate to a URL
    async fn goto(&self, url: &str) -> Result<(), BrowserError>;

    /// Take a screenshot
    async fn screenshot(&self, options: ScreenshotOptions) -> Result<Vec<u8>, BrowserError>;

    /// Get page content as text
    async fn text_content(&self) -> Result<String, BrowserError>;

    /// Get page HTML
    async fn html(&self) -> Result<String, BrowserError>;

    /// Click an element by selector
    async fn click(&self, selector: &str) -> Result<(), BrowserError>;

    /// Type text into an element
    async fn type_text(&self, selector: &str, text: &str) -> Result<(), BrowserError>;

    /// Fill an input (clears first)
    async fn fill(&self, selector: &str, text: &str) -> Result<(), BrowserError>;

    /// Press a key
    async fn press(&self, selector: &str, key: &str) -> Result<(), BrowserError>;

    /// Wait for a selector to appear
    async fn wait_for_selector(&self, selector: &str) -> Result<(), BrowserError>;

    /// Evaluate JavaScript and return result
    async fn evaluate(&self, script: &str) -> Result<serde_json::Value, BrowserError>;

    /// Get element attribute
    async fn get_attribute(&self, selector: &str, attribute: &str) -> Result<Option<String>, BrowserError>;

    /// Check if element exists
    async fn exists(&self, selector: &str) -> Result<bool, BrowserError>;

    /// Get all matching elements' text content
    async fn query_all_text(&self, selector: &str) -> Result<Vec<String>, BrowserError>;

    /// Close the page
    async fn close(&self) -> Result<(), BrowserError>;
}

/// Browser manager trait
#[async_trait]
pub trait Browser: Send + Sync {
    /// Launch the browser
    async fn launch(&self) -> Result<(), BrowserError>;

    /// Create a new page
    async fn new_page(&self) -> Result<Arc<dyn BrowserPage>, BrowserError>;

    /// Navigate to URL and return page
    async fn navigate(&self, url: &str) -> Result<Arc<dyn BrowserPage>, BrowserError>;

    /// Take a screenshot of a URL
    async fn screenshot(&self, url: &str, options: ScreenshotOptions) -> Result<Vec<u8>, BrowserError> {
        let page = self.navigate(url).await?;
        page.screenshot(options).await
    }

    /// Extract text from a URL
    async fn extract_text(&self, url: &str) -> Result<String, BrowserError> {
        let page = self.navigate(url).await?;
        page.text_content().await
    }

    /// Extract HTML from a URL
    async fn extract_html(&self, url: &str) -> Result<String, BrowserError> {
        let page = self.navigate(url).await?;
        page.html().await
    }

    /// Close the browser
    async fn close(&self) -> Result<(), BrowserError>;

    /// Get browser configuration
    fn config(&self) -> &BrowserConfig;
}

/// Create a browser with the given configuration
///
/// Uses Playwright for Firefox/WebKit, CDP for Chromium (if only CDP is enabled).
///
/// # Errors
///
/// Returns `BrowserError::UnsupportedBrowser` if the requested backend is not compiled in.
#[allow(unused_variables)]
pub fn create_browser(config: BrowserConfig) -> Result<Arc<dyn Browser>, BrowserError> {
    // Playwright supports all browsers
    #[cfg(feature = "playwright")]
    {
        return Ok(Arc::new(playwright::PlaywrightBrowser::new(config)));
    }

    // CDP only supports Chromium
    #[cfg(all(feature = "cdp", not(feature = "playwright")))]
    {
        if config.browser_type != BrowserType::Chromium {
            return Err(BrowserError::UnsupportedBrowser(
                "CDP backend only supports Chromium. Enable 'playwright' feature for Firefox/WebKit.".to_string(),
            ));
        }
        return Ok(Arc::new(cdp::CdpBrowser::new(config)));
    }

    #[cfg(not(any(feature = "cdp", feature = "playwright")))]
    {
        Err(BrowserError::UnsupportedBrowser(
            "No browser backend enabled. Enable 'cdp' or 'playwright' feature.".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_browser_config() {
        let config = BrowserConfig::firefox().headless(true).viewport(1280, 720);
        assert_eq!(config.browser_type, BrowserType::Firefox);
        assert!(config.headless);
        assert_eq!(config.viewport_width, 1280);
    }
}
