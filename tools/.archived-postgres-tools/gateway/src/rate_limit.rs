//! Rate Limiting
//!
//! Tiered rate limiting based on subscription tier.
//! Uses governor crate for efficient token bucket implementation.

use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter as GovernorRateLimiter,
};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::Config;

/// Rate limiter with tiered limits
#[derive(Clone)]
pub struct RateLimiter {
    /// Per-AI rate limiters (keyed by ai_id)
    limiters: Arc<RwLock<HashMap<String, TieredLimiter>>>,
    /// Configuration for rate limits
    config: RateLimitConfig,
}

#[derive(Clone)]
struct RateLimitConfig {
    free_rpm: u32,
    basic_rpm: u32,
    pro_rpm: u32,
}

struct TieredLimiter {
    limiter: GovernorRateLimiter<NotKeyed, InMemoryState, DefaultClock>,
    tier: String,
}

impl RateLimiter {
    /// Create a new rate limiter from config
    pub fn new(config: &Config) -> Self {
        Self {
            limiters: Arc::new(RwLock::new(HashMap::new())),
            config: RateLimitConfig {
                free_rpm: config.rate_limit_free,
                basic_rpm: config.rate_limit_basic,
                pro_rpm: config.rate_limit_pro,
            },
        }
    }

    /// Check if a request is allowed for the given AI
    pub async fn check(&self, ai_id: &str, tier: &str) -> bool {
        let mut limiters = self.limiters.write().await;

        // Get or create limiter for this AI
        let entry = limiters.entry(ai_id.to_string()).or_insert_with(|| {
            TieredLimiter {
                limiter: self.create_limiter(tier),
                tier: tier.to_string(),
            }
        });

        // If tier changed, recreate limiter
        if entry.tier != tier {
            entry.limiter = self.create_limiter(tier);
            entry.tier = tier.to_string();
        }

        // Check the rate limit
        entry.limiter.check().is_ok()
    }

    /// Get remaining requests for an AI
    pub async fn remaining(&self, ai_id: &str, tier: &str) -> u32 {
        let limiters = self.limiters.read().await;

        if let Some(entry) = limiters.get(ai_id) {
            // Estimate remaining based on tier limit
            let limit = self.get_limit(tier);
            // This is approximate - governor doesn't expose exact remaining count easily
            limit
        } else {
            self.get_limit(tier)
        }
    }

    /// Get the limit for a tier
    pub fn get_limit(&self, tier: &str) -> u32 {
        match tier {
            "pro" => self.config.pro_rpm,
            "basic" => self.config.basic_rpm,
            _ => self.config.free_rpm,
        }
    }

    fn create_limiter(&self, tier: &str) -> GovernorRateLimiter<NotKeyed, InMemoryState, DefaultClock> {
        let rpm = self.get_limit(tier);
        let quota = Quota::per_minute(NonZeroU32::new(rpm).unwrap_or(NonZeroU32::new(1).unwrap()));
        GovernorRateLimiter::direct(quota)
    }
}

/// Rate limit info for response headers
#[derive(Debug, Clone)]
pub struct RateLimitInfo {
    pub limit: u32,
    pub remaining: u32,
    pub reset_seconds: u64,
}

impl RateLimitInfo {
    /// Convert to HTTP headers
    pub fn to_headers(&self) -> Vec<(&'static str, String)> {
        vec![
            ("X-RateLimit-Limit", self.limit.to_string()),
            ("X-RateLimit-Remaining", self.remaining.to_string()),
            ("X-RateLimit-Reset", self.reset_seconds.to_string()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config {
            addr: "127.0.0.1:8080".parse().unwrap(),
            jwt_secret: "test".to_string(),
            database_url: "postgres://test@localhost/test".to_string(),
            rate_limit_free: 10,
            rate_limit_basic: 100,
            rate_limit_pro: 1000,
        }
    }

    #[tokio::test]
    async fn test_rate_limiter_allows_within_limit() {
        let limiter = RateLimiter::new(&test_config());

        // First request should be allowed
        assert!(limiter.check("test-ai", "free").await);
    }

    #[tokio::test]
    async fn test_tier_limits() {
        let config = test_config();
        let limiter = RateLimiter::new(&config);

        assert_eq!(limiter.get_limit("free"), 10);
        assert_eq!(limiter.get_limit("basic"), 100);
        assert_eq!(limiter.get_limit("pro"), 1000);
    }
}
