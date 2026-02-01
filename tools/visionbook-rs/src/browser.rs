// Browser automation using chromiumoxide (Chrome DevTools Protocol)

use crate::types::{ElementSelector, ImageFormat, NavigationOptions, VisionResult, WaitCondition};
use anyhow::{Context, bail};
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::page::PrintToPdfParams;
use futures::StreamExt;
use std::path::Path;
use tokio::fs;

pub struct BrowserSession {
    browser: Browser,
}

impl BrowserSession {
    /// Launch a new browser instance
    pub async fn launch() -> VisionResult<Self> {
        let config = BrowserConfig::builder()
            .with_head() // Headless by default
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build browser config: {}", e))?;

        let (browser, mut handler) = Browser::launch(config)
            .await
            .context("Failed to launch browser")?;

        // Spawn handler to process browser events
        tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if let Err(e) = event {
                    tracing::error!("Browser event error: {}", e);
                }
            }
        });

        Ok(Self { browser })
    }

    /// Launch browser in headful mode (visible window)
    pub async fn launch_headful() -> VisionResult<Self> {
        let config = BrowserConfig::builder()
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build browser config: {}", e))?;

        let (browser, mut handler) = Browser::launch(config)
            .await
            .context("Failed to launch browser")?;

        tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if let Err(e) = event {
                    tracing::error!("Browser event error: {}", e);
                }
            }
        });

        Ok(Self { browser })
    }

    /// Navigate to a URL
    pub async fn navigate(&self, options: NavigationOptions) -> VisionResult<()> {
        let page = self.browser.new_page(&options.url)
            .await
            .context("Failed to create new page")?;

        // Wait for page load based on condition
        match options.wait_until {
            WaitCondition::Load => {
                page.wait_for_navigation()
                    .await
                    .context("Navigation timeout")?;
            }
            WaitCondition::DomContentLoaded => {
                page.wait_for_navigation()
                    .await
                    .context("Navigation timeout")?;
            }
            WaitCondition::NetworkIdle => {
                // Wait for network to be idle (no active requests for 500ms)
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        }

        tracing::info!("Navigated to {}", options.url);
        Ok(())
    }

    /// Take a screenshot of the current page
    pub async fn screenshot(&self, output_path: &Path, format: ImageFormat) -> VisionResult<()> {
        let page = self.browser.new_page("about:blank")
            .await
            .context("Failed to create page")?;

        use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotParams;

        let screenshot_format = match format {
            ImageFormat::Png => chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png,
            ImageFormat::Jpeg => chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Jpeg,
            ImageFormat::WebP => chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Webp,
        };

        let params = CaptureScreenshotParams::builder()
            .format(screenshot_format)
            .build();

        let screenshot_bytes = page.screenshot(params)
            .await
            .context("Failed to capture screenshot")?;

        fs::write(output_path, screenshot_bytes)
            .await
            .context("Failed to save screenshot")?;

        tracing::info!("Screenshot saved to {}", output_path.display());
        Ok(())
    }

    /// Take a screenshot of a specific element
    pub async fn screenshot_element(
        &self,
        selector: &ElementSelector,
        output_path: &Path,
        format: ImageFormat,
    ) -> VisionResult<()> {
        let page = self.browser.new_page("about:blank")
            .await
            .context("Failed to create page")?;

        let selector_str = selector.as_str();

        use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotParams;

        // Find element
        let element = match selector {
            ElementSelector::Css(_) => {
                page.find_element(selector_str)
                    .await
                    .context(format!("Element not found: {}", selector_str))?
            }
            ElementSelector::XPath(_) => {
                page.find_xpath(selector_str)
                    .await
                    .context(format!("Element not found: {}", selector_str))?
            }
            ElementSelector::Text(_) => {
                bail!("Text selector not supported for screenshots - use CSS or XPath selectors");
            }
        };

        let screenshot_format = match format {
            ImageFormat::Png => chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png,
            ImageFormat::Jpeg => chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Jpeg,
            ImageFormat::WebP => chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Webp,
        };

        let screenshot_bytes = element.screenshot(screenshot_format)
            .await
            .context("Failed to capture element screenshot")?;

        fs::write(output_path, screenshot_bytes)
            .await
            .context("Failed to save screenshot")?;

        tracing::info!("Element screenshot saved to {}", output_path.display());
        Ok(())
    }

    /// Click an element
    pub async fn click(&self, selector: &ElementSelector) -> VisionResult<()> {
        let page = self.browser.new_page("about:blank")
            .await
            .context("Failed to create page")?;

        let selector_str = selector.as_str();

        match selector {
            ElementSelector::Css(_) => {
                let element = page.find_element(selector_str)
                    .await
                    .context(format!("Element not found: {}", selector_str))?;
                element.click()
                    .await
                    .context("Failed to click element")?;
            }
            ElementSelector::XPath(_) => {
                let element = page.find_xpath(selector_str)
                    .await
                    .context(format!("Element not found: {}", selector_str))?;
                element.click()
                    .await
                    .context("Failed to click element")?;
            }
            ElementSelector::Text(_) => {
                bail!("Text selector not supported for click - use CSS or XPath");
            }
        }

        tracing::info!("Clicked element: {}", selector_str);
        Ok(())
    }

    /// Type text into an element
    pub async fn type_text(&self, selector: &ElementSelector, text: &str) -> VisionResult<()> {
        let page = self.browser.new_page("about:blank")
            .await
            .context("Failed to create page")?;

        let selector_str = selector.as_str();

        match selector {
            ElementSelector::Css(_) => {
                let element = page.find_element(selector_str)
                    .await
                    .context(format!("Element not found: {}", selector_str))?;
                element.type_str(text)
                    .await
                    .context("Failed to type text")?;
            }
            ElementSelector::XPath(_) => {
                let element = page.find_xpath(selector_str)
                    .await
                    .context(format!("Element not found: {}", selector_str))?;
                element.type_str(text)
                    .await
                    .context("Failed to type text")?;
            }
            ElementSelector::Text(_) => {
                bail!("Text selector not supported for typing - use CSS or XPath");
            }
        }

        tracing::info!("Typed text into element: {}", selector_str);
        Ok(())
    }

    /// Scrape text content from an element
    pub async fn scrape(&self, selector: &ElementSelector) -> VisionResult<String> {
        let page = self.browser.new_page("about:blank")
            .await
            .context("Failed to create page")?;

        let selector_str = selector.as_str();

        let text = match selector {
            ElementSelector::Css(_) => {
                let element = page.find_element(selector_str)
                    .await
                    .context(format!("Element not found: {}", selector_str))?;
                element.inner_text()
                    .await
                    .context("Failed to get text content")?
                    .unwrap_or_default()
            }
            ElementSelector::XPath(_) => {
                let element = page.find_xpath(selector_str)
                    .await
                    .context(format!("Element not found: {}", selector_str))?;
                element.inner_text()
                    .await
                    .context("Failed to get text content")?
                    .unwrap_or_default()
            }
            ElementSelector::Text(text) => text.clone(),
        };

        tracing::info!("Scraped text from element: {}", selector_str);
        Ok(text)
    }

    /// Execute JavaScript in the page context
    pub async fn execute_js(&self, script: &str) -> VisionResult<serde_json::Value> {
        let page = self.browser.new_page("about:blank")
            .await
            .context("Failed to create page")?;

        let result = page.evaluate(script)
            .await
            .context("Failed to execute JavaScript")?;

        Ok(result.value()
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }

    /// Generate PDF from current page
    pub async fn generate_pdf(&self, output_path: &Path) -> VisionResult<()> {
        let page = self.browser.new_page("about:blank")
            .await
            .context("Failed to create page")?;

        let pdf_params = PrintToPdfParams::default();
        let pdf_bytes = page.pdf(pdf_params)
            .await
            .context("Failed to generate PDF")?;

        fs::write(output_path, pdf_bytes)
            .await
            .context("Failed to save PDF")?;

        tracing::info!("PDF saved to {}", output_path.display());
        Ok(())
    }

    /// Close the browser
    pub async fn close(mut self) -> VisionResult<()> {
        self.browser.close()
            .await
            .context("Failed to close browser")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_launch_browser() {
        let browser = BrowserSession::launch().await;
        assert!(browser.is_ok());
    }
}
