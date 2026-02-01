//! EVOLUTION - Multi-AI collaborative problem solving
//!
//! Multiple AIs contribute solutions, rank each other's work,
//! and collaboratively evolve towards the best approach.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Evolution session status
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum EvolutionStatus {
    Active,      // Accepting contributions
    Voting,      // Ranking phase
    Synthesizing, // Merging best ideas
    Complete,    // Final solution ready
    Abandoned,   // Closed without completion
}

impl EvolutionStatus {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "active" => Some(EvolutionStatus::Active),
            "voting" => Some(EvolutionStatus::Voting),
            "synthesizing" => Some(EvolutionStatus::Synthesizing),
            "complete" => Some(EvolutionStatus::Complete),
            "abandoned" => Some(EvolutionStatus::Abandoned),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            EvolutionStatus::Active => "active",
            EvolutionStatus::Voting => "voting",
            EvolutionStatus::Synthesizing => "synthesizing",
            EvolutionStatus::Complete => "complete",
            EvolutionStatus::Abandoned => "abandoned",
        }
    }
}

/// Evolution session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionSession {
    pub id: i32,
    pub goal: String,
    pub output_file: Option<String>,
    pub created_by: String,
    pub status: EvolutionStatus,
    pub contribution_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A contribution to an evolution session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contribution {
    pub id: i32,
    pub evo_id: i32,
    pub author_ai: String,
    pub content: String,
    pub approach: Option<String>,
    pub avg_score: f64,
    pub rank_count: i32,
    pub created_at: DateTime<Utc>,
}

/// A ranking of a contribution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ranking {
    pub contrib_id: i32,
    pub ranker_ai: String,
    pub score: f64,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Sort options for contributions
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ContributionSort {
    Ranked,  // By avg_score descending
    Recent,  // By created_at descending
    Author,  // By author_ai
}

impl ContributionSort {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "ranked" | "score" => Some(ContributionSort::Ranked),
            "recent" | "time" => Some(ContributionSort::Recent),
            "author" | "ai" => Some(ContributionSort::Author),
            _ => None,
        }
    }
}

/// Evolution storage operations
pub struct EvolutionStorage<'a> {
    client: &'a tokio_postgres::Client,
}

impl<'a> EvolutionStorage<'a> {
    pub fn new(client: &'a tokio_postgres::Client) -> Self {
        Self { client }
    }

    /// Initialize evolution tables
    pub async fn init_schema(&self) -> Result<()> {
        // Sessions table
        self.client.execute(
            "CREATE TABLE IF NOT EXISTS evolution_sessions (
                id SERIAL PRIMARY KEY,
                goal TEXT NOT NULL,
                output_file TEXT,
                created_by TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                contribution_count INT NOT NULL DEFAULT 0,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
            &[],
        ).await.context("Failed to create evolution_sessions table")?;

        // Contributions table
        self.client.execute(
            "CREATE TABLE IF NOT EXISTS evolution_contributions (
                id SERIAL PRIMARY KEY,
                evo_id INT NOT NULL REFERENCES evolution_sessions(id) ON DELETE CASCADE,
                author_ai TEXT NOT NULL,
                content TEXT NOT NULL,
                approach TEXT,
                avg_score REAL NOT NULL DEFAULT 0.0,
                rank_count INT NOT NULL DEFAULT 0,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
            &[],
        ).await.context("Failed to create evolution_contributions table")?;

        // Rankings table
        self.client.execute(
            "CREATE TABLE IF NOT EXISTS evolution_rankings (
                contrib_id INT NOT NULL REFERENCES evolution_contributions(id) ON DELETE CASCADE,
                ranker_ai TEXT NOT NULL,
                score REAL NOT NULL,
                reason TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (contrib_id, ranker_ai)
            )",
            &[],
        ).await.context("Failed to create evolution_rankings table")?;

        // Indexes
        self.client.execute(
            "CREATE INDEX IF NOT EXISTS idx_evolution_contributions_evo
             ON evolution_contributions(evo_id, avg_score DESC)",
            &[],
        ).await.ok();

        self.client.execute(
            "CREATE INDEX IF NOT EXISTS idx_evolution_sessions_status
             ON evolution_sessions(status, created_at DESC)",
            &[],
        ).await.ok();

        Ok(())
    }

    /// Start a new evolution session
    pub async fn start_evolution(
        &self,
        goal: &str,
        created_by: &str,
        output_file: Option<&str>,
    ) -> Result<EvolutionSession> {
        let row = self.client.query_one(
            "INSERT INTO evolution_sessions (goal, created_by, output_file)
             VALUES ($1, $2, $3)
             RETURNING id, goal, output_file, created_by, status, contribution_count,
                       created_at, updated_at",
            &[&goal, &created_by, &output_file],
        ).await.context("Failed to create evolution session")?;

        Ok(self.row_to_session(&row))
    }

    /// Add a contribution to an evolution session
    pub async fn contribute(
        &self,
        evo_id: i32,
        author_ai: &str,
        content: &str,
        approach: Option<&str>,
    ) -> Result<Contribution> {
        // Check session exists and is active
        let session = self.get_session(evo_id).await?
            .ok_or_else(|| anyhow::anyhow!("Evolution session not found"))?;

        if session.status != EvolutionStatus::Active {
            bail!("Session is not accepting contributions (status: {})", session.status.as_str());
        }

        // Insert contribution
        let row = self.client.query_one(
            "INSERT INTO evolution_contributions (evo_id, author_ai, content, approach)
             VALUES ($1, $2, $3, $4)
             RETURNING id, evo_id, author_ai, content, approach, avg_score, rank_count, created_at",
            &[&evo_id, &author_ai, &content, &approach],
        ).await.context("Failed to add contribution")?;

        // Update contribution count
        self.client.execute(
            "UPDATE evolution_sessions
             SET contribution_count = contribution_count + 1, updated_at = NOW()
             WHERE id = $1",
            &[&evo_id],
        ).await?;

        Ok(self.row_to_contribution(&row))
    }

    /// Rank a contribution
    pub async fn rank_contribution(
        &self,
        contrib_id: i32,
        ranker_ai: &str,
        score: f64,
        reason: Option<&str>,
    ) -> Result<f64> {
        // Validate score range
        if score < 0.0 || score > 10.0 {
            bail!("Score must be between 0.0 and 10.0");
        }

        // Check contribution exists
        let contrib_row = self.client.query_opt(
            "SELECT evo_id, author_ai FROM evolution_contributions WHERE id = $1",
            &[&contrib_id],
        ).await?;

        let (evo_id, author_ai): (i32, String) = match contrib_row {
            Some(row) => (row.get(0), row.get(1)),
            None => bail!("Contribution not found"),
        };

        // Can't rank your own contribution
        if author_ai == ranker_ai {
            bail!("Cannot rank your own contribution");
        }

        // Check session status
        let session = self.get_session(evo_id).await?
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

        if session.status != EvolutionStatus::Active && session.status != EvolutionStatus::Voting {
            bail!("Session is not accepting rankings (status: {})", session.status.as_str());
        }

        // Upsert ranking
        self.client.execute(
            "INSERT INTO evolution_rankings (contrib_id, ranker_ai, score, reason)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (contrib_id, ranker_ai) DO UPDATE
             SET score = $3, reason = $4, created_at = NOW()",
            &[&contrib_id, &ranker_ai, &(score as f32), &reason],
        ).await?;

        // Recalculate average score
        let avg_row = self.client.query_one(
            "SELECT AVG(score)::REAL, COUNT(*)::INT FROM evolution_rankings WHERE contrib_id = $1",
            &[&contrib_id],
        ).await?;

        let new_avg: f32 = avg_row.get::<_, Option<f32>>(0).unwrap_or(0.0);
        let rank_count: i32 = avg_row.get(1);

        // Update contribution with new average
        self.client.execute(
            "UPDATE evolution_contributions SET avg_score = $2, rank_count = $3 WHERE id = $1",
            &[&contrib_id, &new_avg, &rank_count],
        ).await?;

        Ok(new_avg as f64)
    }

    /// Get contributions for a session
    pub async fn get_contributions(
        &self,
        evo_id: i32,
        sort: ContributionSort,
        limit: i32,
    ) -> Result<Vec<Contribution>> {
        let order_clause = match sort {
            ContributionSort::Ranked => "avg_score DESC, rank_count DESC",
            ContributionSort::Recent => "created_at DESC",
            ContributionSort::Author => "author_ai ASC, created_at DESC",
        };

        let query = format!(
            "SELECT id, evo_id, author_ai, content, approach, avg_score, rank_count, created_at
             FROM evolution_contributions
             WHERE evo_id = $1
             ORDER BY {}
             LIMIT $2",
            order_clause
        );

        let rows = self.client.query(&query, &[&evo_id, &(limit as i64)]).await?;

        Ok(rows.iter().map(|r| self.row_to_contribution(r)).collect())
    }

    /// Get a specific contribution with its rankings
    pub async fn get_contribution_with_rankings(
        &self,
        contrib_id: i32,
    ) -> Result<(Contribution, Vec<Ranking>)> {
        // Get contribution
        let contrib_row = self.client.query_opt(
            "SELECT id, evo_id, author_ai, content, approach, avg_score, rank_count, created_at
             FROM evolution_contributions WHERE id = $1",
            &[&contrib_id],
        ).await?
            .ok_or_else(|| anyhow::anyhow!("Contribution not found"))?;

        let contrib = self.row_to_contribution(&contrib_row);

        // Get rankings
        let ranking_rows = self.client.query(
            "SELECT contrib_id, ranker_ai, score, reason, created_at
             FROM evolution_rankings WHERE contrib_id = $1 ORDER BY score DESC",
            &[&contrib_id],
        ).await?;

        let rankings: Vec<Ranking> = ranking_rows.iter().map(|row| Ranking {
            contrib_id: row.get(0),
            ranker_ai: row.get(1),
            score: row.get::<_, f32>(2) as f64,
            reason: row.get(3),
            created_at: row.get(4),
        }).collect();

        Ok((contrib, rankings))
    }

    /// Get session by ID
    pub async fn get_session(&self, evo_id: i32) -> Result<Option<EvolutionSession>> {
        let row = self.client.query_opt(
            "SELECT id, goal, output_file, created_by, status, contribution_count,
                    created_at, updated_at
             FROM evolution_sessions WHERE id = $1",
            &[&evo_id],
        ).await?;

        Ok(row.map(|r| self.row_to_session(&r)))
    }

    /// List evolution sessions
    pub async fn list_sessions(
        &self,
        active_only: bool,
        limit: i32,
    ) -> Result<Vec<EvolutionSession>> {
        let rows = if active_only {
            self.client.query(
                "SELECT id, goal, output_file, created_by, status, contribution_count,
                        created_at, updated_at
                 FROM evolution_sessions
                 WHERE status IN ('active', 'voting')
                 ORDER BY updated_at DESC
                 LIMIT $1",
                &[&(limit as i64)],
            ).await?
        } else {
            self.client.query(
                "SELECT id, goal, output_file, created_by, status, contribution_count,
                        created_at, updated_at
                 FROM evolution_sessions
                 ORDER BY updated_at DESC
                 LIMIT $1",
                &[&(limit as i64)],
            ).await?
        };

        Ok(rows.iter().map(|r| self.row_to_session(r)).collect())
    }

    /// Transition session to voting phase
    pub async fn start_voting(&self, evo_id: i32, ai_id: &str) -> Result<bool> {
        let session = self.get_session(evo_id).await?
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

        // Only creator can transition
        if session.created_by != ai_id {
            bail!("Only the session creator can transition to voting");
        }

        if session.status != EvolutionStatus::Active {
            bail!("Session must be active to start voting");
        }

        if session.contribution_count < 2 {
            bail!("Need at least 2 contributions to start voting");
        }

        let result = self.client.execute(
            "UPDATE evolution_sessions SET status = 'voting', updated_at = NOW() WHERE id = $1",
            &[&evo_id],
        ).await?;

        Ok(result > 0)
    }

    /// Complete a session with final synthesis
    pub async fn complete_session(
        &self,
        evo_id: i32,
        ai_id: &str,
        output_file: Option<&str>,
    ) -> Result<bool> {
        let session = self.get_session(evo_id).await?
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

        if session.created_by != ai_id {
            bail!("Only the session creator can complete the session");
        }

        let result = self.client.execute(
            "UPDATE evolution_sessions
             SET status = 'complete', output_file = COALESCE($2, output_file), updated_at = NOW()
             WHERE id = $1",
            &[&evo_id, &output_file],
        ).await?;

        Ok(result > 0)
    }

    /// Abandon a session
    pub async fn abandon_session(&self, evo_id: i32, ai_id: &str) -> Result<bool> {
        let session = self.get_session(evo_id).await?
            .ok_or_else(|| anyhow::anyhow!("Session not found"))?;

        if session.created_by != ai_id {
            bail!("Only the session creator can abandon the session");
        }

        let result = self.client.execute(
            "UPDATE evolution_sessions SET status = 'abandoned', updated_at = NOW() WHERE id = $1",
            &[&evo_id],
        ).await?;

        Ok(result > 0)
    }

    /// Get top-ranked contributions (for synthesis)
    pub async fn get_top_contributions(&self, evo_id: i32, top_n: i32) -> Result<Vec<Contribution>> {
        let rows = self.client.query(
            "SELECT id, evo_id, author_ai, content, approach, avg_score, rank_count, created_at
             FROM evolution_contributions
             WHERE evo_id = $1 AND rank_count > 0
             ORDER BY avg_score DESC, rank_count DESC
             LIMIT $2",
            &[&evo_id, &(top_n as i64)],
        ).await?;

        Ok(rows.iter().map(|r| self.row_to_contribution(r)).collect())
    }

    fn row_to_session(&self, row: &tokio_postgres::Row) -> EvolutionSession {
        let status_str: String = row.get(4);
        EvolutionSession {
            id: row.get(0),
            goal: row.get(1),
            output_file: row.get(2),
            created_by: row.get(3),
            status: EvolutionStatus::from_str(&status_str).unwrap_or(EvolutionStatus::Active),
            contribution_count: row.get(5),
            created_at: row.get(6),
            updated_at: row.get(7),
        }
    }

    fn row_to_contribution(&self, row: &tokio_postgres::Row) -> Contribution {
        Contribution {
            id: row.get(0),
            evo_id: row.get(1),
            author_ai: row.get(2),
            content: row.get(3),
            approach: row.get(4),
            avg_score: row.get::<_, f32>(5) as f64,
            rank_count: row.get(6),
            created_at: row.get(7),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evolution_status() {
        assert_eq!(EvolutionStatus::from_str("active"), Some(EvolutionStatus::Active));
        assert_eq!(EvolutionStatus::Complete.as_str(), "complete");
    }

    #[test]
    fn test_contribution_sort() {
        assert_eq!(ContributionSort::from_str("ranked"), Some(ContributionSort::Ranked));
        assert_eq!(ContributionSort::from_str("score"), Some(ContributionSort::Ranked));
        assert_eq!(ContributionSort::from_str("recent"), Some(ContributionSort::Recent));
    }
}
