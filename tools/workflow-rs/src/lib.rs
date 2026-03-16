//! Workflow-RS - ACE Playbook Manager
//!
//! Implements Stanford ACE framework: Generate → Reflect → Curate
//! Converts playbook_manager.py (45.4 KB) to high-performance Rust
//!
//! Three playbook types:
//! 1. Personal Playbook - Individual AI learnings
//! 2. Team Playbook - Shared knowledge
//! 3. Session Playbook - Temporary context

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use pyo3::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

// ============= TYPES =============

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Strategy {
    pub id: String,
    pub title: String,
    pub context: String,
    pub approach: String,
    pub success_rate: f64,
    pub learned_from: Vec<String>,
    pub last_used: DateTime<Utc>,
    pub effectiveness: f64,
    pub use_count: u32,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Insight {
    pub id: String,
    pub discovery: String,
    pub evidence: Vec<String>,
    pub confidence: f64,
    pub learned_at: DateTime<Utc>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    pub id: String,
    pub situation: String,
    pub pattern: String,
    pub outcomes: Vec<String>,
    pub strength: f64,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybookEntries {
    pub strategies: Vec<Strategy>,
    pub insights: Vec<Insight>,
    pub patterns: Vec<Pattern>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybookMetadata {
    pub total_entries: usize,
    pub avg_effectiveness: f64,
    pub most_used: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playbook {
    pub version: String,
    pub owner: String,
    pub created: DateTime<Utc>,
    pub last_curated: DateTime<Utc>,
    pub entries: PlaybookEntries,
    pub metadata: PlaybookMetadata,
}

// ============= PERSONAL PLAYBOOK =============

pub struct PersonalPlaybook {
    ai_id: String,
    playbook_file: PathBuf,
}

impl PersonalPlaybook {
    pub fn new(ai_id: &str) -> Result<Self> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());

        let playbook_file = PathBuf::from(&home)
            .join(".ai-foundation")
            .join("notebook")
            .join("playbook.json");

        let mut pb = Self {
            ai_id: ai_id.to_string(),
            playbook_file,
        };

        pb.ensure_storage()?;
        Ok(pb)
    }

    fn ensure_storage(&mut self) -> Result<()> {
        if let Some(parent) = self.playbook_file.parent() {
            fs::create_dir_all(parent)?;
        }

        if !self.playbook_file.exists() {
            let initial = Playbook {
                version: "1.0".to_string(),
                owner: self.ai_id.clone(),
                created: Utc::now(),
                last_curated: Utc::now(),
                entries: PlaybookEntries {
                    strategies: Vec::new(),
                    insights: Vec::new(),
                    patterns: Vec::new(),
                },
                metadata: PlaybookMetadata {
                    total_entries: 0,
                    avg_effectiveness: 0.0,
                    most_used: None,
                },
            };

            let json = serde_json::to_string_pretty(&initial)?;
            fs::write(&self.playbook_file, json)?;
        }

        Ok(())
    }

    fn load_playbook(&self) -> Result<Playbook> {
        let contents = fs::read_to_string(&self.playbook_file)
            .context("Failed to read playbook")?;
        let playbook: Playbook = serde_json::from_str(&contents)
            .context("Failed to parse playbook JSON")?;
        Ok(playbook)
    }

    fn save_playbook(&self, playbook: &Playbook) -> Result<()> {
        let json = serde_json::to_string_pretty(playbook)?;
        fs::write(&self.playbook_file, json)?;
        Ok(())
    }

    // ========================================
    // GENERATE - Add new learnings
    // ========================================

    pub fn add_strategy(
        &self,
        title: &str,
        context: &str,
        approach: &str,
        learned_from: Option<Vec<String>>,
        tags: Option<Vec<String>>,
    ) -> Result<String> {
        let mut playbook = self.load_playbook()?;

        let strategy_id = format!("strat-{}", &Uuid::new_v4().to_string()[..8]);
        let strategy = Strategy {
            id: strategy_id.clone(),
            title: title.to_string(),
            context: context.to_string(),
            approach: approach.to_string(),
            success_rate: 0.0,
            learned_from: learned_from.unwrap_or_default(),
            last_used: Utc::now(),
            effectiveness: 0.0,
            use_count: 0,
            tags: tags.unwrap_or_default(),
            created_at: Utc::now(),
        };

        playbook.entries.strategies.push(strategy);
        playbook.metadata.total_entries =
            playbook.entries.strategies.len() +
            playbook.entries.insights.len() +
            playbook.entries.patterns.len();

        self.save_playbook(&playbook)?;

        Ok(strategy_id)
    }

    pub fn add_insight(
        &self,
        discovery: &str,
        evidence: Option<Vec<String>>,
        confidence: f64,
        tags: Option<Vec<String>>,
    ) -> Result<String> {
        let mut playbook = self.load_playbook()?;

        let insight_id = format!("insight-{}", &Uuid::new_v4().to_string()[..8]);
        let insight = Insight {
            id: insight_id.clone(),
            discovery: discovery.to_string(),
            evidence: evidence.unwrap_or_default(),
            confidence: confidence.max(0.0).min(1.0),
            learned_at: Utc::now(),
            tags: tags.unwrap_or_default(),
        };

        playbook.entries.insights.push(insight);
        playbook.metadata.total_entries =
            playbook.entries.strategies.len() +
            playbook.entries.insights.len() +
            playbook.entries.patterns.len();

        self.save_playbook(&playbook)?;

        Ok(insight_id)
    }

    pub fn add_pattern(
        &self,
        situation: &str,
        pattern: &str,
        outcomes: Option<Vec<String>>,
        strength: f64,
        tags: Option<Vec<String>>,
    ) -> Result<String> {
        let mut playbook = self.load_playbook()?;

        let pattern_id = format!("pattern-{}", &Uuid::new_v4().to_string()[..8]);
        let pat = Pattern {
            id: pattern_id.clone(),
            situation: situation.to_string(),
            pattern: pattern.to_string(),
            outcomes: outcomes.unwrap_or_default(),
            strength: strength.max(0.0).min(1.0),
            tags: tags.unwrap_or_default(),
            created_at: Utc::now(),
        };

        playbook.entries.patterns.push(pat);
        playbook.metadata.total_entries =
            playbook.entries.strategies.len() +
            playbook.entries.insights.len() +
            playbook.entries.patterns.len();

        self.save_playbook(&playbook)?;

        Ok(pattern_id)
    }

    // ========================================
    // REFLECT - Update effectiveness
    // ========================================

    pub fn record_strategy_outcome(&self, strategy_id: &str, success: bool) -> Result<()> {
        let mut playbook = self.load_playbook()?;

        if let Some(strategy) = playbook.entries.strategies.iter_mut().find(|s| s.id == strategy_id) {
            strategy.use_count += 1;
            let old_rate = strategy.success_rate;
            let n = strategy.use_count as f64;

            // Update success rate (running average)
            if success {
                strategy.success_rate = (old_rate * (n - 1.0) + 1.0) / n;
            } else {
                strategy.success_rate = (old_rate * (n - 1.0)) / n;
            }

            // Update effectiveness (exponential moving average)
            let alpha = 0.2;
            let new_score = if success { 1.0 } else { 0.0 };
            strategy.effectiveness = alpha * new_score + (1.0 - alpha) * strategy.effectiveness;

            strategy.last_used = Utc::now();

            self.save_playbook(&playbook)?;
        }

        Ok(())
    }

    // ========================================
    // CURATE - Prune low-value entries
    // ========================================

    pub fn curate_playbook(&self, min_effectiveness: f64, max_age_days: i64) -> Result<usize> {
        let mut playbook = self.load_playbook()?;

        let now = Utc::now();
        let cutoff = now - chrono::Duration::days(max_age_days);

        // Remove ineffective or old strategies
        let orig_strat_count = playbook.entries.strategies.len();
        playbook.entries.strategies.retain(|s| {
            let used_recently = s.use_count > 0 && s.last_used > cutoff;
            let is_effective = s.effectiveness >= min_effectiveness || s.use_count < 3;
            used_recently || is_effective
        });

        // Remove low-confidence insights
        let orig_insight_count = playbook.entries.insights.len();
        playbook.entries.insights.retain(|i| i.confidence >= min_effectiveness);

        // Remove weak patterns
        let orig_pattern_count = playbook.entries.patterns.len();
        playbook.entries.patterns.retain(|p| p.strength >= min_effectiveness);

        let removed =
            (orig_strat_count - playbook.entries.strategies.len()) +
            (orig_insight_count - playbook.entries.insights.len()) +
            (orig_pattern_count - playbook.entries.patterns.len());

        playbook.metadata.total_entries =
            playbook.entries.strategies.len() +
            playbook.entries.insights.len() +
            playbook.entries.patterns.len();

        playbook.last_curated = now;

        self.save_playbook(&playbook)?;

        Ok(removed)
    }

    // ========================================
    // QUERY - Retrieve entries
    // ========================================

    pub fn get_strategies(&self, tags: Option<Vec<String>>) -> Result<Vec<Strategy>> {
        let playbook = self.load_playbook()?;

        let strategies = if let Some(filter_tags) = tags {
            playbook.entries.strategies.into_iter()
                .filter(|s| filter_tags.iter().any(|t| s.tags.contains(t)))
                .collect()
        } else {
            playbook.entries.strategies
        };

        Ok(strategies)
    }

    pub fn get_insights(&self, tags: Option<Vec<String>>) -> Result<Vec<Insight>> {
        let playbook = self.load_playbook()?;

        let insights = if let Some(filter_tags) = tags {
            playbook.entries.insights.into_iter()
                .filter(|i| filter_tags.iter().any(|t| i.tags.contains(t)))
                .collect()
        } else {
            playbook.entries.insights
        };

        Ok(insights)
    }

    pub fn get_patterns(&self, tags: Option<Vec<String>>) -> Result<Vec<Pattern>> {
        let playbook = self.load_playbook()?;

        let patterns = if let Some(filter_tags) = tags {
            playbook.entries.patterns.into_iter()
                .filter(|p| filter_tags.iter().any(|t| p.tags.contains(t)))
                .collect()
        } else {
            playbook.entries.patterns
        };

        Ok(patterns)
    }

    pub fn get_summary(&self) -> Result<String> {
        let playbook = self.load_playbook()?;

        let summary = format!(
            "Playbook for {} | {} strategies, {} insights, {} patterns | Avg effectiveness: {:.2}",
            playbook.owner,
            playbook.entries.strategies.len(),
            playbook.entries.insights.len(),
            playbook.entries.patterns.len(),
            playbook.metadata.avg_effectiveness
        );

        Ok(summary)
    }
}

// ============= PYO3 BINDINGS =============

#[pyclass]
pub struct PersonalPlaybookPy {
    inner: PersonalPlaybook,
}

#[pymethods]
impl PersonalPlaybookPy {
    #[new]
    fn new(ai_id: &str) -> PyResult<Self> {
        let inner = PersonalPlaybook::new(ai_id)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
        Ok(Self { inner })
    }

    #[pyo3(signature = (title, context, approach, learned_from=None, tags=None))]
    fn add_strategy(
        &self,
        title: &str,
        context: &str,
        approach: &str,
        learned_from: Option<Vec<String>>,
        tags: Option<Vec<String>>,
    ) -> PyResult<String> {
        self.inner.add_strategy(title, context, approach, learned_from, tags)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    #[pyo3(signature = (discovery, confidence=0.8, evidence=None, tags=None))]
    fn add_insight(
        &self,
        discovery: &str,
        confidence: f64,
        evidence: Option<Vec<String>>,
        tags: Option<Vec<String>>,
    ) -> PyResult<String> {
        self.inner.add_insight(discovery, evidence, confidence, tags)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    #[pyo3(signature = (situation, pattern, strength, outcomes=None, tags=None))]
    fn add_pattern(
        &self,
        situation: &str,
        pattern: &str,
        strength: f64,
        outcomes: Option<Vec<String>>,
        tags: Option<Vec<String>>,
    ) -> PyResult<String> {
        self.inner.add_pattern(situation, pattern, outcomes, strength, tags)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    fn record_strategy_outcome(&self, strategy_id: &str, success: bool) -> PyResult<()> {
        self.inner.record_strategy_outcome(strategy_id, success)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    fn curate_playbook(&self, min_effectiveness: f64, max_age_days: i64) -> PyResult<usize> {
        self.inner.curate_playbook(min_effectiveness, max_age_days)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    #[pyo3(signature = (tags=None))]
    fn get_strategies(&self, tags: Option<Vec<String>>) -> PyResult<String> {
        self.inner.get_strategies(tags)
            .map(|strategies| serde_json::to_string(&strategies).unwrap())
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    #[pyo3(signature = (tags=None))]
    fn get_insights(&self, tags: Option<Vec<String>>) -> PyResult<String> {
        self.inner.get_insights(tags)
            .map(|insights| serde_json::to_string(&insights).unwrap())
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    #[pyo3(signature = (tags=None))]
    fn get_patterns(&self, tags: Option<Vec<String>>) -> PyResult<String> {
        self.inner.get_patterns(tags)
            .map(|patterns| serde_json::to_string(&patterns).unwrap())
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    fn get_summary(&self) -> PyResult<String> {
        self.inner.get_summary()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }
}

#[pymodule]
fn workflow_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PersonalPlaybookPy>()?;
    Ok(())
}
