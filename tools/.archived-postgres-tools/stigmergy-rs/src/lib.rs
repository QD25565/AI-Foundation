//! Stigmergy-RS - High-Performance Digital Pheromone Coordination
//!
//! Rust implementation of stigmergic coordination for multi-AI systems.
//! Replaces 774 lines of Python with memory-safe, fast Rust code.
//!
//! Features:
//! - O(1) coordination via environmental traces
//! - PostgreSQL persistence with connection pooling
//! - Thread-safe operations
//! - Activity-based pheromone refresh
//! - Sub-millisecond latency
//!
//! Architecture:
//! - Core types: PheromoneType, DigitalPheromone
//! - Backend: PostgreSQL with deadpool connection pooling
//! - PyO3 bindings for Python integration
//! - Async/await with Tokio runtime

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

pub mod backend;
pub mod server;
pub mod pyo3_bindings;

/// Pheromone types for different coordination patterns
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PheromoneType {
    /// Exploration signal - "I'm looking at this"
    Interest,
    /// Active work claim - "I'm working on this"
    Working,
    /// Problem signal - "This is blocked"
    Blocked,
    /// Completion signal - "I finished this successfully"
    Success,
    /// Confusion signal - "This is confusing" (3+ reads in 60s)
    Question,
}

impl fmt::Display for PheromoneType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PheromoneType::Interest => write!(f, "interest"),
            PheromoneType::Working => write!(f, "working"),
            PheromoneType::Blocked => write!(f, "blocked"),
            PheromoneType::Success => write!(f, "success"),
            PheromoneType::Question => write!(f, "question"),
        }
    }
}

impl PheromoneType {
    /// Parse from string (case-insensitive)
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "interest" => Some(PheromoneType::Interest),
            "working" => Some(PheromoneType::Working),
            "blocked" => Some(PheromoneType::Blocked),
            "success" => Some(PheromoneType::Success),
            "question" => Some(PheromoneType::Question),
            _ => None,
        }
    }
}

/// Digital pheromone with exponential decay
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DigitalPheromone {
    /// Location identifier (e.g., "task:42", "file:/path/to/file.py")
    pub location: String,

    /// Type of pheromone
    pub pheromone_type: PheromoneType,

    /// Initial intensity (0.0-2.0)
    pub intensity: f64,

    /// Decay rate per second (0.0-1.0)
    /// - 0.0 = no decay (permanent)
    /// - 0.2 = fast decay (3s half-life)
    /// - 0.05 = slow decay (14s half-life)
    pub decay_rate: f64,

    /// Agent that deposited this pheromone
    pub agent_id: String,

    /// When pheromone was created/refreshed
    pub created_at: DateTime<Utc>,
}

impl DigitalPheromone {
    /// Create new pheromone
    pub fn new(
        location: String,
        pheromone_type: PheromoneType,
        intensity: f64,
        decay_rate: f64,
        agent_id: String,
    ) -> Self {
        Self {
            location,
            pheromone_type,
            intensity: intensity.clamp(0.0, 2.0),
            decay_rate: decay_rate.clamp(0.0, 1.0),
            agent_id,
            created_at: Utc::now(),
        }
    }

    /// Calculate current intensity based on exponential decay
    ///
    /// Formula: I(t) = I₀ * (1 - r)^t
    ///
    /// Where:
    /// - I(t) = current intensity
    /// - I₀ = initial intensity
    /// - r = decay rate
    /// - t = elapsed time in seconds
    pub fn current_intensity(&self, ref_time: Option<DateTime<Utc>>) -> f64 {
        let ref_time = ref_time.unwrap_or_else(Utc::now);
        let elapsed = (ref_time - self.created_at).num_seconds().max(0) as f64;

        if self.decay_rate >= 1.0 {
            // Instant expiry
            return 0.0;
        }

        if self.decay_rate == 0.0 {
            // No decay
            return self.intensity.clamp(0.0, 2.0);
        }

        // Exponential decay: I(t) = I₀ * (1 - r)^t
        let current = self.intensity * (1.0 - self.decay_rate).powf(elapsed);
        current.clamp(0.0, 2.0)
    }

    /// Check if pheromone is expired (intensity below threshold)
    pub fn is_expired(&self, threshold: f64) -> bool {
        self.current_intensity(None) < threshold
    }

    /// Calculate half-life in seconds
    ///
    /// Formula: t_half = log(0.5) / log(1 - r)
    pub fn half_life(&self) -> Option<f64> {
        if self.decay_rate == 0.0 {
            return None; // Infinite half-life
        }

        if self.decay_rate >= 1.0 {
            return Some(0.0); // Instant decay
        }

        // t_half = log(0.5) / log(1 - r)
        Some(0.5_f64.ln() / (1.0 - self.decay_rate).ln())
    }

    /// Calculate expiration time (when intensity reaches threshold)
    ///
    /// Formula: t = log(threshold/I₀) / log(1 - r)
    pub fn expiration_seconds(&self, threshold: f64) -> Option<f64> {
        if self.decay_rate == 0.0 {
            return None; // Never expires
        }

        if self.decay_rate >= 1.0 {
            return Some(0.0); // Immediate expiry
        }

        if threshold >= self.intensity {
            return Some(0.0); // Already below threshold
        }

        // t = log(threshold/I₀) / log(1 - r)
        let seconds = (threshold / self.intensity).ln() / (1.0 - self.decay_rate).ln();
        Some(seconds.max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pheromone_decay() {
        let mut pheromone = DigitalPheromone::new(
            "task:42".to_string(),
            PheromoneType::Working,
            1.0,
            0.05,
            "test-agent".to_string(),
        );

        // Initial intensity
        assert!((pheromone.current_intensity(None) - 1.0).abs() < 0.01);

        // After 14 seconds (half-life with decay_rate=0.05)
        pheromone.created_at = Utc::now() - chrono::Duration::seconds(14);
        assert!((pheromone.current_intensity(None) - 0.5).abs() < 0.1);
    }

    #[test]
    fn test_pheromone_half_life() {
        let pheromone = DigitalPheromone::new(
            "task:42".to_string(),
            PheromoneType::Working,
            1.0,
            0.05,
            "test-agent".to_string(),
        );

        let half_life = pheromone.half_life().unwrap();
        assert!((half_life - 13.51).abs() < 0.1);
    }

    #[test]
    fn test_no_decay_pheromone() {
        let mut pheromone = DigitalPheromone::new(
            "task:42".to_string(),
            PheromoneType::Blocked,
            2.0,
            0.0, // No decay
            "test-agent".to_string(),
        );

        // Wait 1000 seconds
        pheromone.created_at = Utc::now() - chrono::Duration::seconds(1000);

        // Should still be at full intensity
        assert!((pheromone.current_intensity(None) - 2.0).abs() < 0.01);
        assert!(pheromone.half_life().is_none());
    }
}
