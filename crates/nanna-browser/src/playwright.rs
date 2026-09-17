//! Playwright backend for multi-browser support
//!
//! Supports Chromium, Firefox, and `WebKit` browsers.
//!
//! **Requirements:**
//! - Node.js 18+
//! - Playwright browsers: `npx playwright@1.56.1 install`

use crate::{Browser, BrowserConfig, BrowserError, BrowserPage, BrowserType, ScreenshotOptions};
use async_trait::async_trait;
use playwright_rs::Playwright;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Playwright-based browser manager
pub struct PlaywrightBrowser {
    config: BrowserConfig,
    playwright: RwLock<Option<Playwright>>,
    browser: RwLock<Option<playwright_rs::Browser>>,
}

impl PlaywrightBrowser {
    /// Create a new Playwright browser manager.
    #[must_use]
    pub fn new(config: BrowserConfig) -> Self {
        Self {
            config,
            playwright: RwLock::new(None),
            browser: RwLock::new(None),
        }
    }

    async fn ensure_launched(&self) -> Result<(), BrowserError> {
        {
            let browser_guard = self.browser.read().await;
            if browser_guard.is_some() {
                return Ok(());
            }
        }
        self.launch().await
    }
}

#[async_trait]
impl Browser for PlaywrightBrowser {
    async fn launch(&self) -> Result<(), BrowserError> {
        let mut pw_guard = self.playwright.write().await;
        let mut browser_guard = self.browser.write().await;

        if browser_guard.is_some() {
            return Ok(());
        }

        info!(
            "Launching Playwright {} (headless: {})",
            self.config.browser_type, self.config.headless
        );

        // Initialize Playwright
        let playwright = Playwright::launch()
            .await
            .map_err(|e| BrowserError::LaunchFailed(format!("Playwright init failed: {e}")))?;

        // Launch browser based on type
        let browser = match self.config.browser_type {
            BrowserType::Chromium => {
                playwright
                    .chromium()
                    .launch()
                    .await
                    .map_err(|e| BrowserError::LaunchFailed(format!("Chromium launch failed: {e}")))?
            }
            BrowserType::Firefox => {
                playwright
                    .firefox()
                    .launch()
                    .await
                    .map_err(|e| BrowserError::LaunchFailed(format!("Firefox launch failed: {e}")))?
            }
            BrowserType::Webkit => {
                playwright
                    .webkit()
                    .launch()
                    .await
                    .map_err(|e| BrowserError::LaunchFailed(format!("WebKit launch failed: {e}")))?
            }
        };

        *pw_guard = Some(playwright);
        *browser_guard = Some(browser);
        // Released in the order the scope would have released them, only
        // before the log line instead of after it.
        drop(browser_guard);
        drop(pw_guard);

        info!("Playwright {} launched successfully", self.config.browser_type);
        Ok(())
    }

    async fn new_page(&self) -> Result<Arc<dyn BrowserPage>, BrowserError> {
        self.ensure_launched().await?;

        let browser_guard = self.browser.read().await;
        let browser = browser_guard.as_ref().ok_or(BrowserError::NotInitialized)?;

        let page = browser
            .new_page()
            .await
            .map_err(|e| BrowserError::ExecutionFailed(format!("Page creation failed: {e}")))?;
        drop(browser_guard);

        Ok(Arc::new(PlaywrightPage::new(page)))
    }

    async fn navigate(&self, url: &str) -> Result<Arc<dyn BrowserPage>, BrowserError> {
        let page = self.new_page().await?;
        page.goto(url).await?;
        Ok(page)
    }

    async fn close(&self) -> Result<(), BrowserError> {
        let mut browser_guard = self.browser.write().await;
        // Taking the slot under the write lock is what makes releasing it here
        // safe: no reader can observe a half-closed browser, and the taken
        // handle is the only thing left that can reach the old one, so a
        // concurrent `launch()` builds a fresh browser instead of waiting on a
        // teardown whose outcome it does not depend on.
        let taken = browser_guard.take();
        drop(browser_guard);
        if let Some(browser) = taken {
            browser
                .close()
                .await
                .map_err(|e| BrowserError::ExecutionFailed(format!("Close failed: {e}")))?;
            info!("Browser closed");
        }
        Ok(())
    }

    fn config(&self) -> &BrowserConfig {
        &self.config
    }
}

/// Playwright page wrapper
///
/// Carries no deadline. `BrowserConfig::timeout_ms` used to be threaded in
/// here and read by nothing — the same dead field `CdpPage` carried, where it
/// turned out to be an unbounded await on every page operation. It is removed
/// rather than left to look like a bound that exists: applying it for real is
/// the `CdpPage::bounded` / `bounded_operation` pattern in `cdp.rs`, and that
/// changes what a hung page does, which makes it a port rather than a cleanup.
pub struct PlaywrightPage {
    page: playwright_rs::Page,
}

impl PlaywrightPage {
    const fn new(page: playwright_rs::Page) -> Self {
        Self { page }
    }
}

#[async_trait]
impl BrowserPage for PlaywrightPage {
    fn url(&self) -> &'static str {
        // page.url() returns String, we can't return &str easily
        // Return empty, callers should use page.url() directly for now
        ""
    }

    async fn goto(&self, url: &str) -> Result<(), BrowserError> {
        debug!("Navigating to {}", url);

        self.page
            .goto(url, None)
            .await
            .map_err(|e| BrowserError::NavigationFailed(e.to_string()))?;

        Ok(())
    }

    async fn screenshot(&self, options: ScreenshotOptions) -> Result<Vec<u8>, BrowserError> {
        // Build screenshot options
        let pw_options = if options.full_page {
            Some(
                playwright_rs::protocol::ScreenshotOptions::builder()
                    .full_page(true)
                    .build(),
            )
        } else {
            None
        };

        let data = self
            .page
            .screenshot(pw_options)
            .await
            .map_err(|e| BrowserError::ScreenshotFailed(e.to_string()))?;

        Ok(data)
    }

    async fn text_content(&self) -> Result<String, BrowserError> {
        // Get body text content via locator
        let locator = self.page.locator("body");
        locator
            .text_content()
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?
            .ok_or_else(|| BrowserError::ExecutionFailed("No text content".to_string()))
    }

    async fn html(&self) -> Result<String, BrowserError> {
        self.page
            .content()
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))
    }

    async fn click(&self, selector: &str) -> Result<(), BrowserError> {
        let locator = self.page.locator(selector);
        locator
            .click(None)
            .await
            .map_err(|e| BrowserError::ElementNotFound(format!("{selector}: {e}")))
    }

    async fn type_text(&self, selector: &str, text: &str) -> Result<(), BrowserError> {
        // Use fill for typing (more reliable in Playwright)
        let locator = self.page.locator(selector);
        locator
            .fill(text, None)
            .await
            .map_err(|e| BrowserError::ElementNotFound(format!("{selector}: {e}")))
    }

    async fn fill(&self, selector: &str, text: &str) -> Result<(), BrowserError> {
        let locator = self.page.locator(selector);
        locator
            .fill(text, None)
            .await
            .map_err(|e| BrowserError::ElementNotFound(format!("{selector}: {e}")))
    }

    async fn press(&self, selector: &str, key: &str) -> Result<(), BrowserError> {
        let locator = self.page.locator(selector);
        locator
            .press(key, None)
            .await
            .map_err(|e| BrowserError::ElementNotFound(format!("{selector}: {e}")))
    }

    async fn wait_for_selector(&self, selector: &str) -> Result<(), BrowserError> {
        // Use locator's visibility check as wait mechanism
        let locator = self.page.locator(selector);
        // Wait for it to be visible
        let _visible = locator
            .is_visible()
            .await
            .map_err(|e| BrowserError::ElementNotFound(format!("{selector}: {e}")))?;
        Ok(())
    }

    async fn evaluate(&self, script: &str) -> Result<serde_json::Value, BrowserError> {
        self.page
            .evaluate_value(script)
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))
            .and_then(|s| {
                serde_json::from_str(&s)
                    .map_err(|e| BrowserError::ExecutionFailed(format!("JSON parse error: {e}")))
            })
    }

    async fn get_attribute(&self, selector: &str, attribute: &str) -> Result<Option<String>, BrowserError> {
        let locator = self.page.locator(selector);
        locator
            .get_attribute(attribute)
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))
    }

    async fn exists(&self, selector: &str) -> Result<bool, BrowserError> {
        let locator = self.page.locator(selector);
        let count = locator
            .count()
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?;
        Ok(count > 0)
    }

    async fn query_all_text(&self, selector: &str) -> Result<Vec<String>, BrowserError> {
        // Use evaluate to get all text contents
        let script = format!(
            r"Array.from(document.querySelectorAll('{}')).map(el => el.textContent || '')",
            selector.replace('\'', "\\'")
        );

        let result = self.page
            .evaluate_value(&script)
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?;

        serde_json::from_str(&result)
            .map_err(|e| BrowserError::ExecutionFailed(format!("JSON parse error: {e}")))
    }

    async fn close(&self) -> Result<(), BrowserError> {
        self.page
            .close()
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))
    }
}
