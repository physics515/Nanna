//! CDP (Chrome DevTools Protocol) backend via chromiumoxide

use crate::{Browser, BrowserConfig, BrowserError, BrowserPage, BrowserType, ImageFormat, ScreenshotOptions};
use async_trait::async_trait;
use chromiumoxide::browser::{Browser as CoBrowser, BrowserConfig as CoConfig};
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::page::ScreenshotParams;
use chromiumoxide::Page;
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// CDP-based browser manager (Chromium only)
pub struct CdpBrowser {
    config: BrowserConfig,
    browser: RwLock<Option<CoBrowser>>,
}

impl CdpBrowser {
    /// Create a new CDP browser manager.
    #[must_use]
    pub fn new(config: BrowserConfig) -> Self {
        Self {
            config,
            browser: RwLock::new(None),
        }
    }

    async fn ensure_launched(&self) -> Result<(), BrowserError> {
        let browser_guard = self.browser.read().await;
        if browser_guard.is_none() {
            drop(browser_guard);
            self.launch().await?;
        }
        Ok(())
    }
}

#[async_trait]
impl Browser for CdpBrowser {
    async fn launch(&self) -> Result<(), BrowserError> {
        let mut browser_guard = self.browser.write().await;

        if browser_guard.is_some() {
            return Ok(());
        }

        if self.config.browser_type != BrowserType::Chromium {
            return Err(BrowserError::UnsupportedBrowser(
                "CDP backend only supports Chromium".to_string(),
            ));
        }

        info!("Launching CDP browser (headless: {})", self.config.headless);

        let mut builder = CoConfig::builder();

        if !self.config.headless {
            builder = builder.with_head();
        }

        if let Some(ref path) = self.config.executable_path {
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

        for arg in &self.config.args {
            // chromiumoxide 0.9: Arg is not From<&String>; pass an owned String.
            builder = builder.arg(arg.clone());
        }

        let co_config = builder
            .build()
            .map_err(|e| BrowserError::LaunchFailed(e.to_string()))?;

        let (browser, mut handler) = CoBrowser::launch(co_config)
            .await
            .map_err(|e| BrowserError::LaunchFailed(e.to_string()))?;

        // Spawn handler task
        tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                debug!("CDP event: {:?}", event);
            }
        });

        *browser_guard = Some(browser);
        info!("CDP browser launched successfully");
        Ok(())
    }

    async fn new_page(&self) -> Result<Arc<dyn BrowserPage>, BrowserError> {
        self.ensure_launched().await?;

        let browser_guard = self.browser.read().await;
        let browser = browser_guard.as_ref().ok_or(BrowserError::NotInitialized)?;

        let page = browser
            .new_page("about:blank")
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?;

        Ok(Arc::new(CdpPage::new(page, self.config.timeout_ms)))
    }

    async fn navigate(&self, url: &str) -> Result<Arc<dyn BrowserPage>, BrowserError> {
        self.ensure_launched().await?;

        let browser_guard = self.browser.read().await;
        let browser = browser_guard.as_ref().ok_or(BrowserError::NotInitialized)?;

        let page = browser
            .new_page(url)
            .await
            .map_err(|e| BrowserError::NavigationFailed(e.to_string()))?;

        page.wait_for_navigation()
            .await
            .map_err(|e| BrowserError::NavigationFailed(e.to_string()))?;

        Ok(Arc::new(CdpPage::new(page, self.config.timeout_ms)))
    }

    async fn close(&self) -> Result<(), BrowserError> {
        let mut browser_guard = self.browser.write().await;
        if browser_guard.take().is_some() {
            info!("CDP browser closed");
        }
        Ok(())
    }

    fn config(&self) -> &BrowserConfig {
        &self.config
    }
}

/// CDP page wrapper
pub struct CdpPage {
    page: Page,
    timeout_ms: u64,
}

impl CdpPage {
    fn new(page: Page, timeout_ms: u64) -> Self {
        Self { page, timeout_ms }
    }
}

#[async_trait]
impl BrowserPage for CdpPage {
    fn url(&self) -> &str {
        ""
    }

    async fn goto(&self, url: &str) -> Result<(), BrowserError> {
        self.page
            .goto(url)
            .await
            .map_err(|e| BrowserError::NavigationFailed(e.to_string()))?;
        
        self.page
            .wait_for_navigation()
            .await
            .map_err(|e| BrowserError::NavigationFailed(e.to_string()))?;
        
        Ok(())
    }

    async fn screenshot(&self, options: ScreenshotOptions) -> Result<Vec<u8>, BrowserError> {
        let format = match options.format {
            ImageFormat::Png => CaptureScreenshotFormat::Png,
            ImageFormat::Jpeg => CaptureScreenshotFormat::Jpeg,
        };

        let params = ScreenshotParams::builder()
            .format(format)
            .full_page(options.full_page)
            .build();

        self.page
            .screenshot(params)
            .await
            .map_err(|e| BrowserError::ScreenshotFailed(e.to_string()))
    }

    async fn text_content(&self) -> Result<String, BrowserError> {
        self.page
            .evaluate("document.body.innerText")
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?
            .into_value::<String>()
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))
    }

    async fn html(&self) -> Result<String, BrowserError> {
        self.page
            .evaluate("document.documentElement.outerHTML")
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?
            .into_value::<String>()
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))
    }

    async fn click(&self, selector: &str) -> Result<(), BrowserError> {
        let element = self
            .page
            .find_element(selector)
            .await
            .map_err(|e| BrowserError::ElementNotFound(format!("{}: {}", selector, e)))?;

        element
            .click()
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?;

        Ok(())
    }

    async fn type_text(&self, selector: &str, text: &str) -> Result<(), BrowserError> {
        let element = self
            .page
            .find_element(selector)
            .await
            .map_err(|e| BrowserError::ElementNotFound(format!("{}: {}", selector, e)))?;

        element
            .type_str(text)
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?;

        Ok(())
    }

    async fn fill(&self, selector: &str, text: &str) -> Result<(), BrowserError> {
        // CDP doesn't have a native "fill" - clear and type
        let element = self
            .page
            .find_element(selector)
            .await
            .map_err(|e| BrowserError::ElementNotFound(format!("{}: {}", selector, e)))?;

        // Focus and clear
        element.focus().await.ok();
        element.click().await.ok();

        // Select all and delete
        self.page
            .evaluate(format!(
                "document.querySelector('{}').value = ''",
                selector.replace('\'', "\\'")
            ))
            .await
            .ok();

        // Type new text
        element
            .type_str(text)
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?;

        Ok(())
    }

    async fn press(&self, selector: &str, key: &str) -> Result<(), BrowserError> {
        let element = self
            .page
            .find_element(selector)
            .await
            .map_err(|e| BrowserError::ElementNotFound(format!("{}: {}", selector, e)))?;

        element
            .press_key(key)
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?;

        Ok(())
    }

    async fn wait_for_selector(&self, selector: &str) -> Result<(), BrowserError> {
        self.page
            .find_element(selector)
            .await
            .map_err(|e| BrowserError::ElementNotFound(format!("{}: {}", selector, e)))?;
        Ok(())
    }

    async fn evaluate(&self, script: &str) -> Result<serde_json::Value, BrowserError> {
        self.page
            .evaluate(script)
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?
            .into_value()
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))
    }

    async fn get_attribute(&self, selector: &str, attribute: &str) -> Result<Option<String>, BrowserError> {
        let script = format!(
            r#"(() => {{
                const el = document.querySelector('{}');
                return el ? el.getAttribute('{}') : null;
            }})()"#,
            selector.replace('\'', "\\'"),
            attribute.replace('\'', "\\'")
        );

        self.page
            .evaluate(script.as_str())
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?
            .into_value::<Option<String>>()
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))
    }

    async fn exists(&self, selector: &str) -> Result<bool, BrowserError> {
        let script = format!(
            "document.querySelector('{}') !== null",
            selector.replace('\'', "\\'")
        );

        self.page
            .evaluate(script.as_str())
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?
            .into_value::<bool>()
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))
    }

    async fn query_all_text(&self, selector: &str) -> Result<Vec<String>, BrowserError> {
        let script = format!(
            r#"Array.from(document.querySelectorAll('{}')).map(el => el.textContent || '')"#,
            selector.replace('\'', "\\'")
        );

        self.page
            .evaluate(script.as_str())
            .await
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?
            .into_value::<Vec<String>>()
            .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))
    }

    async fn close(&self) -> Result<(), BrowserError> {
        // Page will be closed when dropped
        Ok(())
    }
}
