// Network interception and mocking using Chrome DevTools Protocol

use crate::types::{InterceptAction, InterceptRule, VisionResult};
use anyhow::Context;
use serde_json::json;

pub struct NetworkInterceptor {
    rules: Vec<InterceptRule>,
}

impl NetworkInterceptor {
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Add an interception rule
    pub fn add_rule(&mut self, rule: InterceptRule) {
        self.rules.push(rule);
    }

    /// Enable network interception for a browser session
    pub async fn enable(&self) -> VisionResult<()> {
        // This would integrate with chromiumoxide's Network domain
        // For now, this is a placeholder for the future implementation
        tracing::info!("Network interception enabled with {} rules", self.rules.len());
        Ok(())
    }

    /// Block requests matching pattern
    pub fn block_pattern(&mut self, url_pattern: String) {
        self.add_rule(InterceptRule {
            url_pattern,
            action: InterceptAction::Block,
        });
    }

    /// Mock response for requests matching pattern
    pub fn mock_response(&mut self, url_pattern: String, response_body: String, status_code: u16) {
        self.add_rule(InterceptRule {
            url_pattern,
            action: InterceptAction::Mock {
                response_body,
                status_code,
            },
        });
    }

    /// Modify headers for requests matching pattern
    pub fn modify_headers(&mut self, url_pattern: String, headers: Vec<(String, String)>) {
        self.add_rule(InterceptRule {
            url_pattern,
            action: InterceptAction::ModifyHeaders { headers },
        });
    }
}

impl Default for NetworkInterceptor {
    fn default() -> Self {
        Self::new()
    }
}

pub struct RequestInterceptor;

impl RequestInterceptor {
    /// Intercept and block ads/trackers
    pub fn block_ads() -> Vec<InterceptRule> {
        vec![
            InterceptRule {
                url_pattern: "*doubleclick.net*".to_string(),
                action: InterceptAction::Block,
            },
            InterceptRule {
                url_pattern: "*googlesyndication.com*".to_string(),
                action: InterceptAction::Block,
            },
            InterceptRule {
                url_pattern: "*google-analytics.com*".to_string(),
                action: InterceptAction::Block,
            },
        ]
    }

    /// Intercept and block images
    pub fn block_images() -> Vec<InterceptRule> {
        vec![InterceptRule {
            url_pattern: "*.{png,jpg,jpeg,gif,webp}".to_string(),
            action: InterceptAction::Block,
        }]
    }
}

pub struct ResponseMock;

impl ResponseMock {
    /// Create a mock JSON response
    pub fn json(data: serde_json::Value, status_code: u16) -> String {
        json!({
            "status": status_code,
            "body": data.to_string(),
            "headers": {
                "Content-Type": "application/json"
            }
        })
        .to_string()
    }

    /// Create a mock HTML response
    pub fn html(html: String, status_code: u16) -> String {
        json!({
            "status": status_code,
            "body": html,
            "headers": {
                "Content-Type": "text/html"
            }
        })
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_interceptor() {
        let mut interceptor = NetworkInterceptor::new();
        interceptor.block_pattern("*ads.example.com*".to_string());
        assert_eq!(interceptor.rules.len(), 1);
    }

    #[test]
    fn test_block_ads() {
        let rules = RequestInterceptor::block_ads();
        assert!(!rules.is_empty());
    }
}
