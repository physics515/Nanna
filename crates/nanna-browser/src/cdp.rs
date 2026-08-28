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
        // The deadline is the only bound on a CDP round-trip, so a zero would
        // mean "every page operation times out instantly" and a caller would
        // read that as the browser being broken. `BrowserConfig` defaults to
        // 30_000 and its builder takes a `u64`, so zero is reachable only as a
        // programmer error.
        assert!(timeout_ms > 0, "browser timeout_ms must be positive, got {timeout_ms}");
        Self { page, timeout_ms }
    }

    /// Run one page operation under the browser's configured deadline.
    ///
    /// **Every CDP call in this file goes through here, and none of them used
    /// to.** `BrowserConfig::timeout_ms` was threaded into this struct and read
    /// by nothing — a dead-code warning that was really an unbounded await: a
    /// page that stopped responding hung `goto`, `screenshot`, `evaluate` and
    /// `wait_for_selector` forever, with no cancellation and no error. That is
    /// the one thing this codebase's own doctrine says never to ship ("bound
    /// everything — every loop, queue, cache, retry"), and the bound already
    /// existed; it simply was not applied.
    ///
    /// The deadline is per operation, not per page: it is what
    /// `BrowserConfig::timeout_ms` documents itself as, and a page-lifetime
    /// budget would make a long session's last call fail for reasons the caller
    /// cannot see.
    async fn bounded<T, F>(&self, operation: &str, work: F) -> Result<T, BrowserError>
    where
        F: std::future::Future<Output = Result<T, BrowserError>>,
    {
        bounded_operation(self.timeout_ms, operation, work).await
    }
}

/// The deadline itself, as a free function over primitives.
///
/// Separate from [`CdpPage::bounded`] only so it can be tested: the method
/// needs a live `chromiumoxide::Page`, which needs a browser, which is exactly
/// the dependency that kept this bound untested (and, for a long time,
/// unapplied) in the first place. Nothing here depends on the page.
async fn bounded_operation<T, F>(
    timeout_ms: u64,
    operation: &str,
    work: F,
) -> Result<T, BrowserError>
where
    F: std::future::Future<Output = Result<T, BrowserError>>,
{
    debug_assert!(!operation.is_empty(), "a bounded operation must name itself");
    debug_assert!(timeout_ms > 0, "a zero deadline would fail every operation");
    match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), work).await {
        Ok(result) => result,
        Err(_elapsed) => Err(BrowserError::Timeout {
            operation: operation.to_string(),
            timeout_ms,
        }),
    }
}

#[async_trait]
impl BrowserPage for CdpPage {
    fn url(&self) -> &str {
        ""
    }

    async fn goto(&self, url: &str) -> Result<(), BrowserError> {
        // One deadline covers the navigation AND the load wait: a page that
        // starts loading and never finishes is the exact hang this bounds, and
        // splitting the two would hand it twice the budget.
        self.bounded("goto", async {
            self.page
                .goto(url)
                .await
                .map_err(|e| BrowserError::NavigationFailed(e.to_string()))?;

            self.page
                .wait_for_navigation()
                .await
                .map_err(|e| BrowserError::NavigationFailed(e.to_string()))?;

            Ok(())
        })
        .await
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

        self.bounded("screenshot", async {
            self.page
                .screenshot(params)
                .await
                .map_err(|e| BrowserError::ScreenshotFailed(e.to_string()))
        })
        .await
    }

    async fn text_content(&self) -> Result<String, BrowserError> {
        self.bounded("text_content", async {
            self.page
                .evaluate("document.body.innerText")
                .await
                .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?
                .into_value::<String>()
                .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))
        })
        .await
    }

    async fn html(&self) -> Result<String, BrowserError> {
        self.bounded("html", async {
            self.page
                .evaluate("document.documentElement.outerHTML")
                .await
                .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?
                .into_value::<String>()
                .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))
        })
        .await
    }

    async fn click(&self, selector: &str) -> Result<(), BrowserError> {
        self.bounded("click", async {
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
        })
        .await
    }

    async fn type_text(&self, selector: &str, text: &str) -> Result<(), BrowserError> {
        self.bounded("type_text", async {
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
        })
        .await
    }

    async fn fill(&self, selector: &str, text: &str) -> Result<(), BrowserError> {
        // One deadline for the whole clear-and-type sequence, not one per CDP
        // call: `fill` is a single operation from the caller's side, and four
        // nested budgets would let a degraded page spend four times the stated
        // timeout while every individual step stayed inside it.
        self.bounded("fill", async {
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
        })
        .await
    }

    async fn press(&self, selector: &str, key: &str) -> Result<(), BrowserError> {
        self.bounded("press", async {
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
        })
        .await
    }

    async fn wait_for_selector(&self, selector: &str) -> Result<(), BrowserError> {
        // The method most obviously in need of a deadline, and the one that had
        // none: an element that never appears is not a state chromiumoxide
        // returns an error for on its own.
        self.bounded("wait_for_selector", async {
            self.page
                .find_element(selector)
                .await
                .map_err(|e| BrowserError::ElementNotFound(format!("{}: {}", selector, e)))?;
            Ok(())
        })
        .await
    }

    async fn evaluate(&self, script: &str) -> Result<serde_json::Value, BrowserError> {
        self.bounded("evaluate", async {
            self.page
                .evaluate(script)
                .await
                .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?
                .into_value()
                .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))
        })
        .await
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

        self.bounded("get_attribute", async {
            self.page
                .evaluate(script.as_str())
                .await
                .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?
                .into_value::<Option<String>>()
                .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))
        })
        .await
    }

    async fn exists(&self, selector: &str) -> Result<bool, BrowserError> {
        let script = format!(
            "document.querySelector('{}') !== null",
            selector.replace('\'', "\\'")
        );

        self.bounded("exists", async {
            self.page
                .evaluate(script.as_str())
                .await
                .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?
                .into_value::<bool>()
                .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))
        })
        .await
    }

    async fn query_all_text(&self, selector: &str) -> Result<Vec<String>, BrowserError> {
        let script = format!(
            r#"Array.from(document.querySelectorAll('{}')).map(el => el.textContent || '')"#,
            selector.replace('\'', "\\'")
        );

        self.bounded("query_all_text", async {
            self.page
                .evaluate(script.as_str())
                .await
                .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))?
                .into_value::<Vec<String>>()
                .map_err(|e| BrowserError::ExecutionFailed(e.to_string()))
        })
        .await
    }

    async fn close(&self) -> Result<(), BrowserError> {
        // Page will be closed when dropped
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::bounded_operation;
    use crate::BrowserError;
    use std::time::Duration;

    /// The defect: `BrowserConfig::timeout_ms` was threaded into `CdpPage` and
    /// read by nothing, so a page that stopped responding hung every operation
    /// forever. Without the deadline this test does not fail — it never
    /// returns.
    #[tokio::test]
    async fn an_operation_that_never_finishes_is_cut_off() {
        let hang = async {
            std::future::pending::<()>().await;
            Ok::<(), BrowserError>(())
        };
        let outcome = bounded_operation(20, "wait_for_selector", hang).await;
        match outcome {
            Err(BrowserError::Timeout { operation, timeout_ms }) => {
                // The error has to say WHAT was waiting and for how long: the
                // bare `Timeout` this replaced carried neither, which is part
                // of why nobody noticed it was never constructed.
                assert_eq!(operation, "wait_for_selector");
                assert_eq!(timeout_ms, 20);
            }
            other => panic!("a hung operation must time out, got {other:?}"),
        }
    }

    /// Positive space: the deadline must not interfere with work that finishes.
    #[tokio::test]
    async fn an_operation_that_finishes_keeps_its_result() {
        let quick = async { Ok::<u32, BrowserError>(7) };
        assert_eq!(bounded_operation(30_000, "evaluate", quick).await.unwrap(), 7);
    }

    /// And a real failure must surface as itself, not be relabelled a timeout.
    #[tokio::test]
    async fn a_failing_operation_reports_its_own_error() {
        let failing = async {
            Err::<(), BrowserError>(BrowserError::ElementNotFound("#missing".into()))
        };
        match bounded_operation(30_000, "click", failing).await {
            Err(BrowserError::ElementNotFound(what)) => assert_eq!(what, "#missing"),
            other => panic!("the operation's own error must survive, got {other:?}"),
        }
    }

    /// The deadline is enforced, not merely declared: a slow operation inside
    /// the budget still returns its value.
    #[tokio::test]
    async fn a_slow_operation_inside_the_budget_still_succeeds() {
        let slow = async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            Ok::<&str, BrowserError>("done")
        };
        assert_eq!(bounded_operation(5_000, "goto", slow).await.unwrap(), "done");
    }
}
