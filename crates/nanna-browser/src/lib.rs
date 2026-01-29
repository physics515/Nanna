#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! Browser automation for Nanna using Chrome DevTools Protocol
//!
//! Provides headless browser control for web scraping, screenshots, and automation.

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::page::{CaptureScreenshotFormat, CaptureScreenshotParams};
use chromiumoxide::page::ScreenshotParams;
use chromiumoxide::Page;
use futures::StreamExt;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

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
}

/// Browser configuration
#[derive(Debug, Clone)]
pub struct BrowserManagerConfig {
    /// Path to Chrome/Chromium executable (None = auto-detect)
    pub chrome_path: Option<String>,
    /// Run in headless mode
    pub headless: bool,
    /// Default viewport width
    pub viewport_width: u32,
    /// Default viewport height
    pub viewport_height: u32,
    /// Navigation timeout in seconds
    pub timeout_secs: u64,
}

impl Default for BrowserManagerConfig {
    fn default() -> Self {
        Self {
            chrome_path: None,
            headless: true,
            viewport_width: 1920,
            viewport_height: 1080,
            timeout_secs: 30,
        }
    }
}

/// Manages browser instances
pub struct BrowserManager {
    config: BrowserManagerConfig,
    browser: RwLock<Option<Browser>>,
}

impl BrowserManager {
    /// Create a new browser manager.
    #[must_use]
    pub fn new(config: BrowserManagerConfig) -> Self {
        Self {
            config,
            browser: RwLock::new(None),
        }
    }

    /// Launch the browser if not already running.
    ///
    /// # Errors
    ///
    /// Returns `BrowserError::LaunchFailed` if the browser cannot be started.
    pub async fn launch(&self) -> Result<(), BrowserError> {
        let mut browser_guard = self.browser.write().await;

        if browser_guard.is_some() {
            return Ok(());
        }

        info!("Launching browser (headless: {})", self.config.headless);

        let mut builder = BrowserConfig::builder();

        if self.config.headless {
            builder = builder.with_head();
        }

        if let Some(ref path) = self.config.chrome_path {
            builder = builder.chrome_executable(path);
        }

        builder = builder
            .viewport(chromiumoxide::handler::viewport::Viewport {
                width: self.config.viewport_width,
                height: self.config.viewport_height,
                device_scale_factor: None,
                emulating_mobile: false,
                is_landscape: true,
                has_touch: false,
            })
            .arg("--disable-gpu")
            .arg("--no-sandbox")
            .arg("--disable-dev-shm-usage");

        let config = builder.build().map_err(|e| BrowserError::LaunchFailed(e.to_string()))?;

        let (browser, mut handler) = Browser::launch(config)
            .await
            .map_err(|e| BrowserError::LaunchFailed(e.to_string()))?;

        // Spawn handler task
        tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                debug!("Browser event: {:?}", event);
            }
        });

        *browser_guard = Some(browser);
        info!("Browser launched successfully");
        Ok(())
    }

    /// Navigate to a URL and return the page.
    ///
    /// # Errors
    ///
    /// Returns `BrowserError::NavigationFailed` if navigation fails.
    pub async fn navigate(&self, url: &str) -> Result<Arc<Page>, BrowserError> {
        self.launch().await?;

        let browser_guard = self.browser.read().await;
        let browser = browser_guard
            .as_ref()
            .ok_or(BrowserError::NotInitialized)?;

        let page = browser
            .new_page(url)
            .await
            .map_err(|e| BrowserError::NavigationFailed(e.to_string()))?;

        // Wait for page to load
        page.wait_for_navigation()
            .await
            .map_err(|e| BrowserError::NavigationFailed(e.to_string()))?;

        Ok(Arc::new(page))
    }

    /// Take a screenshot of the current page.
    ///
    /// # Errors
    ///
    /// Returns `BrowserError::ScreenshotFailed` if the screenshot fails.
    pub async fn screenshot(&self, url: &str, full_page: bool) -> Result<Vec<u8>, BrowserError> {
        let page = self.navigate(url).await?;

        let params = ScreenshotParams::builder()
            .format(CaptureScreenshotFormat::Png)
            .full_page(full_page)
            .build();

        let screenshot = page
            .screenshot(params)
            .await
            .map_err(|e| BrowserError::ScreenshotFailed(e.to_string()))?;

        Ok(screenshot)
    }

    /// Extract text content from a page.
    ///
    /// # Errors
    ///
    /// Returns `BrowserError::ExecutionFailed` if extraction fails.
    pub async fn extract_text(&self, url: &str) -> Result<String, BrowserError> {
        let page = self.navigate(url).await?;

        let text = page
            .evaluate("document.body.innerText")
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?
            .into_value::<String>()
            .unwrap_or_default();

        Ok(text)
    }

    /// Extract HTML content from a page.
    ///
    /// # Errors
    ///
    /// Returns `BrowserError::ExecutionFailed` if extraction fails.
    pub async fn extract_html(&self, url: &str) -> Result<String, BrowserError> {
        let page = self.navigate(url).await?;

        let html = page
            .evaluate("document.documentElement.outerHTML")
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?
            .into_value::<String>()
            .unwrap_or_default();

        Ok(html)
    }

    /// Click an element by selector.
    ///
    /// # Errors
    ///
    /// Returns `BrowserError::ElementNotFound` if the element doesn't exist.
    pub async fn click(&self, page: &Page, selector: &str) -> Result<(), BrowserError> {
        let element = page
            .find_element(selector)
            .await
            .map_err(|e| BrowserError::ElementNotFound(format!("{}: {}", selector, e)))?;

        element
            .click()
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?;

        Ok(())
    }

    /// Type text into an element.
    ///
    /// # Errors
    ///
    /// Returns `BrowserError::ElementNotFound` if the element doesn't exist.
    pub async fn type_text(&self, page: &Page, selector: &str, text: &str) -> Result<(), BrowserError> {
        let element = page
            .find_element(selector)
            .await
            .map_err(|e| BrowserError::ElementNotFound(format!("{}: {}", selector, e)))?;

        element
            .type_str(text)
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?;

        Ok(())
    }

    /// Execute JavaScript on a page.
    ///
    /// # Errors
    ///
    /// Returns `BrowserError::ExecutionFailed` if the script fails.
    pub async fn evaluate(&self, page: &Page, script: &str) -> Result<serde_json::Value, BrowserError> {
        let result = page
            .evaluate(script)
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?;

        Ok(result.into_value().unwrap_or(serde_json::Value::Null))
    }

    /// Close the browser.
    pub async fn close(&self) {
        let mut browser_guard = self.browser.write().await;
        if browser_guard.is_some() {
            *browser_guard = None;
            info!("Browser closed");
        }
    }
}

impl Drop for BrowserManager {
    fn drop(&mut self) {
        // Browser will be dropped automatically
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires Chrome to be installed
    async fn test_screenshot() {
        let config = BrowserManagerConfig::default();
        let manager = BrowserManager::new(config);

        let screenshot = manager.screenshot("https://example.com", false).await;
        assert!(screenshot.is_ok());

        let data = screenshot.unwrap();
        assert!(!data.is_empty());

        manager.close().await;
    }
}
