//! BCCS - Belief-Calibrated Consensus Seeking
//!
//! Tracks AI belief states and enables structured consensus building.
//! Divergence thresholds:
//! - ALIGNED: < 15% divergence
//! - DIVERGED_ACCEPTABLE: 15-40% divergence
//! - ESCALATED: > 40% divergence

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Divergence classification
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DivergenceLevel {
    Aligned,           // < 15% - proceed without discussion
    DivergedAcceptable, // 15-40% - note differences, proceed
    Escalated,         // > 40% - requires resolution
}

impl DivergenceLevel {
    pub fn from_divergence(divergence: f64) -> Self {
        if divergence < 0.15 {
            DivergenceLevel::Aligned
        } else if divergence < 0.40 {
            DivergenceLevel::DivergedAcceptable
        } else {
            DivergenceLevel::Escalated
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            DivergenceLevel::Aligned => "ALIGNED",
            DivergenceLevel::DivergedAcceptable => "DIVERGED_ACCEPTABLE",
            DivergenceLevel::Escalated => "ESCALATED",
        }
    }

    pub fn threshold(&self) -> &'static str {
        match self {
            DivergenceLevel::Aligned => "<15%",
            DivergenceLevel::DivergedAcceptable => "15-40%",
            DivergenceLevel::Escalated => ">40%",
        }
    }
}

/// Decision status
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DecisionStatus {
    Pending,   // Awaiting votes
    Approved,  // Consensus reached
    Rejected,  // Consensus against
    Escalated, // High divergence, needs discussion
    Closed,    // Manually closed
}

impl DecisionStatus {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "pending" => Some(DecisionStatus::Pending),
            "approved" => Some(DecisionStatus::Approved),
            "rejected" => Some(DecisionStatus::Rejected),
            "escalated" => Some(DecisionStatus::Escalated),
            "closed" => Some(DecisionStatus::Closed),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            DecisionStatus::Pending => "pending",
            DecisionStatus::Approved => "approved",
            DecisionStatus::Rejected => "rejected",
            DecisionStatus::Escalated => "escalated",
            DecisionStatus::Closed => "closed",
        }
    }
}

/// A belief state for a specific topic
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeliefState {
    pub id: i32,
    pub ai_id: String,
    pub task_id: String,
    pub belief_name: String,
    pub belief_value: f64,    // 0.0 to 1.0
    pub confidence: f64,      // 0.0 to 1.0
    pub rationale: Option<String>,
    pub assumptions: Option<String>,
    pub unknowns: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Comparison result between two AIs' beliefs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeliefComparison {
    pub task_id: String,
    pub ai_1: String,
    pub ai_2: String,
    pub comparisons: Vec<BeliefDiff>,
    pub overall_divergence: f64,
    pub level: DivergenceLevel,
}

/// Single belief difference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeliefDiff {
    pub belief_name: String,
    pub value_1: Option<f64>,
    pub value_2: Option<f64>,
    pub confidence_1: Option<f64>,
    pub confidence_2: Option<f64>,
    pub divergence: f64,
}

/// A decision requiring consensus
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub id: i32,
    pub task_id: String,
    pub decision: String,
    pub owner_ai: String,
    pub confidence: Option<f64>,
    pub rationale: Option<String>,
    pub status: DecisionStatus,
    pub vote_count: i32,
    pub approve_count: i32,
    pub created_at: DateTime<Utc>,
}

/// A vote on a decision
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vote {
    pub decision_id: i32,
    pub voter_ai: String,
    pub vote: String,  // "approve", "reject", "abstain"
    pub rationale: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// BCCS storage operations
pub struct BccsStorage<'a> {
    client: &'a tokio_postgres::Client,
}

impl<'a> BccsStorage<'a> {
    pub fn new(client: &'a tokio_postgres::Client) -> Self {
        Self { client }
    }

    /// Initialize BCCS tables
    pub async fn init_schema(&self) -> Result<()> {
        // Belief states table
        self.client.execute(
            "CREATE TABLE IF NOT EXISTS belief_states (
                id SERIAL PRIMARY KEY,
                ai_id TEXT NOT NULL,
                task_id TEXT NOT NULL,
                belief_name TEXT NOT NULL,
                belief_value REAL NOT NULL,
                confidence REAL NOT NULL DEFAULT 0.5,
                rationale TEXT,
                assumptions TEXT,
                unknowns TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                UNIQUE (ai_id, task_id, belief_name)
            )",
            &[],
        ).await.context("Failed to create belief_states table")?;

        // Decisions table
        self.client.execute(
            "CREATE TABLE IF NOT EXISTS bccs_decisions (
                id SERIAL PRIMARY KEY,
                task_id TEXT NOT NULL,
                decision TEXT NOT NULL,
                owner_ai TEXT NOT NULL,
                confidence REAL,
                rationale TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                vote_count INT NOT NULL DEFAULT 0,
                approve_count INT NOT NULL DEFAULT 0,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
            &[],
        ).await.context("Failed to create bccs_decisions table")?;

        // Votes table
        self.client.execute(
            "CREATE TABLE IF NOT EXISTS bccs_votes (
                decision_id INT NOT NULL REFERENCES bccs_decisions(id) ON DELETE CASCADE,
                voter_ai TEXT NOT NULL,
                vote TEXT NOT NULL,
                rationale TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (decision_id, voter_ai)
            )",
            &[],
        ).await.context("Failed to create bccs_votes table")?;

        // Indexes
        self.client.execute(
            "CREATE INDEX IF NOT EXISTS idx_belief_states_task
             ON belief_states(task_id, ai_id)",
            &[],
        ).await.ok();

        self.client.execute(
            "CREATE INDEX IF NOT EXISTS idx_bccs_decisions_task
             ON bccs_decisions(task_id, status)",
            &[],
        ).await.ok();

        Ok(())
    }

    // ==================== BELIEF STATE OPERATIONS ====================

    /// Record or update a belief state
    pub async fn record_belief(
        &self,
        ai_id: &str,
        task_id: &str,
        belief_name: &str,
        belief_value: f64,
        confidence: f64,
        rationale: Option<&str>,
        assumptions: Option<&str>,
        unknowns: Option<&str>,
    ) -> Result<BeliefState> {
        // Validate ranges
        if belief_value < 0.0 || belief_value > 1.0 {
            bail!("belief_value must be between 0.0 and 1.0");
        }
        if confidence < 0.0 || confidence > 1.0 {
            bail!("confidence must be between 0.0 and 1.0");
        }

        // Upsert belief
        let row = self.client.query_one(
            "INSERT INTO belief_states
             (ai_id, task_id, belief_name, belief_value, confidence, rationale, assumptions, unknowns)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (ai_id, task_id, belief_name) DO UPDATE
             SET belief_value = $4, confidence = $5, rationale = $6,
                 assumptions = $7, unknowns = $8, updated_at = NOW()
             RETURNING id, ai_id, task_id, belief_name, belief_value, confidence,
                       rationale, assumptions, unknowns, created_at, updated_at",
            &[
                &ai_id, &task_id, &belief_name,
                &(belief_value as f32), &(confidence as f32),
                &rationale, &assumptions, &unknowns
            ],
        ).await.context("Failed to record belief")?;

        Ok(self.row_to_belief(&row))
    }

    /// Get all beliefs for a task by an AI
    pub async fn get_beliefs(
        &self,
        ai_id: &str,
        task_id: &str,
    ) -> Result<Vec<BeliefState>> {
        let rows = self.client.query(
            "SELECT id, ai_id, task_id, belief_name, belief_value, confidence,
                    rationale, assumptions, unknowns, created_at, updated_at
             FROM belief_states
             WHERE ai_id = $1 AND task_id = $2
             ORDER BY belief_name",
            &[&ai_id, &task_id],
        ).await?;

        Ok(rows.iter().map(|r| self.row_to_belief(r)).collect())
    }

    /// Get a specific belief
    pub async fn get_belief(
        &self,
        ai_id: &str,
        task_id: &str,
        belief_name: &str,
    ) -> Result<Option<BeliefState>> {
        let row = self.client.query_opt(
            "SELECT id, ai_id, task_id, belief_name, belief_value, confidence,
                    rationale, assumptions, unknowns, created_at, updated_at
             FROM belief_states
             WHERE ai_id = $1 AND task_id = $2 AND belief_name = $3",
            &[&ai_id, &task_id, &belief_name],
        ).await?;

        Ok(row.map(|r| self.row_to_belief(&r)))
    }

    /// Compare beliefs between two AIs for a task
    pub async fn compare_beliefs(
        &self,
        task_id: &str,
        ai_1: &str,
        ai_2: &str,
    ) -> Result<BeliefComparison> {
        // Get beliefs for both AIs
        let beliefs_1 = self.get_beliefs(ai_1, task_id).await?;
        let beliefs_2 = self.get_beliefs(ai_2, task_id).await?;

        // Create lookup maps
        let map_1: std::collections::HashMap<&str, &BeliefState> =
            beliefs_1.iter().map(|b| (b.belief_name.as_str(), b)).collect();
        let map_2: std::collections::HashMap<&str, &BeliefState> =
            beliefs_2.iter().map(|b| (b.belief_name.as_str(), b)).collect();

        // Get all unique belief names
        let mut all_names: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for b in &beliefs_1 { all_names.insert(&b.belief_name); }
        for b in &beliefs_2 { all_names.insert(&b.belief_name); }

        // Calculate differences
        let mut comparisons = Vec::new();
        let mut total_divergence = 0.0;
        let mut count = 0;

        for name in all_names {
            let b1 = map_1.get(name);
            let b2 = map_2.get(name);

            let divergence = match (b1, b2) {
                (Some(a), Some(b)) => {
                    // Weighted divergence: value diff * avg confidence
                    let value_diff = (a.belief_value - b.belief_value).abs();
                    let avg_conf = (a.confidence + b.confidence) / 2.0;
                    value_diff * avg_conf
                },
                (Some(_), None) | (None, Some(_)) => {
                    // One AI has belief, other doesn't - moderate divergence
                    0.5
                },
                (None, None) => 0.0, // Shouldn't happen
            };

            comparisons.push(BeliefDiff {
                belief_name: name.to_string(),
                value_1: b1.map(|b| b.belief_value),
                value_2: b2.map(|b| b.belief_value),
                confidence_1: b1.map(|b| b.confidence),
                confidence_2: b2.map(|b| b.confidence),
                divergence,
            });

            total_divergence += divergence;
            count += 1;
        }

        let overall_divergence = if count > 0 { total_divergence / count as f64 } else { 0.0 };
        let level = DivergenceLevel::from_divergence(overall_divergence);

        Ok(BeliefComparison {
            task_id: task_id.to_string(),
            ai_1: ai_1.to_string(),
            ai_2: ai_2.to_string(),
            comparisons,
            overall_divergence,
            level,
        })
    }

    // ==================== DECISION OPERATIONS ====================

    /// Record a decision requiring consensus
    pub async fn record_decision(
        &self,
        task_id: &str,
        decision: &str,
        owner_ai: &str,
        confidence: Option<f64>,
        rationale: Option<&str>,
    ) -> Result<Decision> {
        let conf_f32 = confidence.map(|c| c as f32);

        let row = self.client.query_one(
            "INSERT INTO bccs_decisions (task_id, decision, owner_ai, confidence, rationale)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING id, task_id, decision, owner_ai, confidence, rationale, status,
                       vote_count, approve_count, created_at",
            &[&task_id, &decision, &owner_ai, &conf_f32, &rationale],
        ).await.context("Failed to record decision")?;

        Ok(self.row_to_decision(&row))
    }

    /// Vote on a decision
    pub async fn vote_on_decision(
        &self,
        decision_id: i32,
        voter_ai: &str,
        vote: &str,
        rationale: Option<&str>,
    ) -> Result<Decision> {
        // Validate vote value
        let vote_lower = vote.to_lowercase();
        if !["approve", "reject", "abstain"].contains(&vote_lower.as_str()) {
            bail!("Vote must be 'approve', 'reject', or 'abstain'");
        }

        // Check decision exists and is pending
        let decision = self.get_decision(decision_id).await?
            .ok_or_else(|| anyhow::anyhow!("Decision not found"))?;

        if decision.status != DecisionStatus::Pending {
            bail!("Decision is not pending (status: {})", decision.status.as_str());
        }

        // Can't vote on your own decision
        if decision.owner_ai == voter_ai {
            bail!("Cannot vote on your own decision");
        }

        // Upsert vote
        self.client.execute(
            "INSERT INTO bccs_votes (decision_id, voter_ai, vote, rationale)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (decision_id, voter_ai) DO UPDATE
             SET vote = $3, rationale = $4, created_at = NOW()",
            &[&decision_id, &voter_ai, &vote_lower, &rationale],
        ).await?;

        // Update vote counts
        let counts = self.client.query_one(
            "SELECT COUNT(*)::INT, COUNT(*) FILTER (WHERE vote = 'approve')::INT
             FROM bccs_votes WHERE decision_id = $1",
            &[&decision_id],
        ).await?;

        let vote_count: i32 = counts.get(0);
        let approve_count: i32 = counts.get(1);

        self.client.execute(
            "UPDATE bccs_decisions SET vote_count = $2, approve_count = $3 WHERE id = $1",
            &[&decision_id, &vote_count, &approve_count],
        ).await?;

        // Return updated decision
        self.get_decision(decision_id).await?
            .ok_or_else(|| anyhow::anyhow!("Decision not found after update"))
    }

    /// Get a decision by ID
    pub async fn get_decision(&self, decision_id: i32) -> Result<Option<Decision>> {
        let row = self.client.query_opt(
            "SELECT id, task_id, decision, owner_ai, confidence, rationale, status,
                    vote_count, approve_count, created_at
             FROM bccs_decisions WHERE id = $1",
            &[&decision_id],
        ).await?;

        Ok(row.map(|r| self.row_to_decision(&r)))
    }

    /// Get votes for a decision
    pub async fn get_votes(&self, decision_id: i32) -> Result<Vec<Vote>> {
        let rows = self.client.query(
            "SELECT decision_id, voter_ai, vote, rationale, created_at
             FROM bccs_votes WHERE decision_id = $1 ORDER BY created_at",
            &[&decision_id],
        ).await?;

        Ok(rows.iter().map(|row| Vote {
            decision_id: row.get(0),
            voter_ai: row.get(1),
            vote: row.get(2),
            rationale: row.get(3),
            created_at: row.get(4),
        }).collect())
    }

    /// List decisions for a task
    pub async fn list_decisions(
        &self,
        task_id: &str,
        pending_only: bool,
    ) -> Result<Vec<Decision>> {
        let rows = if pending_only {
            self.client.query(
                "SELECT id, task_id, decision, owner_ai, confidence, rationale, status,
                        vote_count, approve_count, created_at
                 FROM bccs_decisions
                 WHERE task_id = $1 AND status = 'pending'
                 ORDER BY created_at DESC",
                &[&task_id],
            ).await?
        } else {
            self.client.query(
                "SELECT id, task_id, decision, owner_ai, confidence, rationale, status,
                        vote_count, approve_count, created_at
                 FROM bccs_decisions
                 WHERE task_id = $1
                 ORDER BY created_at DESC",
                &[&task_id],
            ).await?
        };

        Ok(rows.iter().map(|r| self.row_to_decision(r)).collect())
    }

    /// Resolve a decision based on votes
    pub async fn resolve_decision(
        &self,
        decision_id: i32,
        required_votes: i32,
        approval_threshold: f64,
    ) -> Result<DecisionStatus> {
        let decision = self.get_decision(decision_id).await?
            .ok_or_else(|| anyhow::anyhow!("Decision not found"))?;

        if decision.status != DecisionStatus::Pending {
            return Ok(decision.status);
        }

        if decision.vote_count < required_votes {
            return Ok(DecisionStatus::Pending); // Not enough votes yet
        }

        let approval_rate = decision.approve_count as f64 / decision.vote_count as f64;

        let new_status = if approval_rate >= approval_threshold {
            DecisionStatus::Approved
        } else {
            DecisionStatus::Rejected
        };

        self.client.execute(
            "UPDATE bccs_decisions SET status = $2 WHERE id = $1",
            &[&decision_id, &new_status.as_str()],
        ).await?;

        Ok(new_status)
    }

    /// Run full BCCS protocol between two AIs
    pub async fn run_bccs(
        &self,
        task_id: &str,
        ai_1: &str,
        ai_2: &str,
    ) -> Result<BeliefComparison> {
        // Compare beliefs
        let comparison = self.compare_beliefs(task_id, ai_1, ai_2).await?;

        // If escalated, could auto-create a decision point
        if comparison.level == DivergenceLevel::Escalated {
            // Log the escalation (future: could auto-create dialogue session)
            self.client.execute(
                "INSERT INTO bccs_decisions (task_id, decision, owner_ai, rationale, status)
                 VALUES ($1, $2, $3, $4, 'escalated')
                 ON CONFLICT DO NOTHING",
                &[
                    &task_id,
                    &format!("BCCS Escalation: {} vs {}", ai_1, ai_2),
                    &ai_1,
                    &format!("Divergence: {:.1}% (threshold: >40%)", comparison.overall_divergence * 100.0),
                ],
            ).await.ok();
        }

        Ok(comparison)
    }

    fn row_to_belief(&self, row: &tokio_postgres::Row) -> BeliefState {
        BeliefState {
            id: row.get(0),
            ai_id: row.get(1),
            task_id: row.get(2),
            belief_name: row.get(3),
            belief_value: row.get::<_, f32>(4) as f64,
            confidence: row.get::<_, f32>(5) as f64,
            rationale: row.get(6),
            assumptions: row.get(7),
            unknowns: row.get(8),
            created_at: row.get(9),
            updated_at: row.get(10),
        }
    }

    fn row_to_decision(&self, row: &tokio_postgres::Row) -> Decision {
        let status_str: String = row.get(6);
        Decision {
            id: row.get(0),
            task_id: row.get(1),
            decision: row.get(2),
            owner_ai: row.get(3),
            confidence: row.get::<_, Option<f32>>(4).map(|c| c as f64),
            rationale: row.get(5),
            status: DecisionStatus::from_str(&status_str).unwrap_or(DecisionStatus::Pending),
            vote_count: row.get(7),
            approve_count: row.get(8),
            created_at: row.get(9),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_divergence_level() {
        assert_eq!(DivergenceLevel::from_divergence(0.10), DivergenceLevel::Aligned);
        assert_eq!(DivergenceLevel::from_divergence(0.25), DivergenceLevel::DivergedAcceptable);
        assert_eq!(DivergenceLevel::from_divergence(0.50), DivergenceLevel::Escalated);
    }

    #[test]
    fn test_decision_status() {
        assert_eq!(DecisionStatus::from_str("pending"), Some(DecisionStatus::Pending));
        assert_eq!(DecisionStatus::Approved.as_str(), "approved");
    }
}
