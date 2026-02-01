//! Stigmergy server with async operations
//!
//! High-level API for stigmergic coordination with automatic backend selection

use crate::{backend::PostgreSQLBackend, DigitalPheromone, PheromoneType};
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Stigmergy server with PostgreSQL backend
pub struct StigmergyServer {
    backend: Arc<PostgreSQLBackend>,
}

impl StigmergyServer {
    /// Create new server with PostgreSQL backend
    pub async fn new(database_url: &str) -> Result<Self> {
        let backend = PostgreSQLBackend::new(database_url).await?;
        info!("Stigmergy server initialized with PostgreSQL backend");

        Ok(Self {
            backend: Arc::new(backend),
        })
    }

    /// Leave a pheromone trace
    pub async fn leave_trace(&self, pheromone: DigitalPheromone) -> Result<bool> {
        self.backend.leave_trace(&pheromone).await
    }

    /// Sense pheromones at a location
    pub async fn sense_environment(
        &self,
        location: &str,
        pheromone_type: Option<PheromoneType>,
    ) -> Result<Vec<DigitalPheromone>> {
        self.backend.sense_environment(location, pheromone_type, 0.01).await
    }

    /// Get total intensity at location
    pub async fn get_intensity(
        &self,
        location: &str,
        pheromone_type: Option<PheromoneType>,
    ) -> Result<f64> {
        self.backend.get_intensity(location, pheromone_type).await
    }

    /// Refresh pheromone (activity-based refresh)
    pub async fn refresh_pheromone(
        &self,
        location: &str,
        pheromone_type: PheromoneType,
        agent_id: &str,
    ) -> Result<bool> {
        self.backend.refresh_pheromone(location, pheromone_type, agent_id).await
    }

    /// Clear pheromones at location
    pub async fn clear_pheromone(
        &self,
        location: &str,
        pheromone_type: PheromoneType,
        agent_id: Option<&str>,
    ) -> Result<()> {
        self.backend.clear_pheromone(location, pheromone_type, agent_id).await
    }

    /// Cleanup expired pheromones
    pub async fn cleanup_expired(&self) -> Result<u64> {
        self.backend.cleanup_expired().await
    }

    /// Check if location is available (WORKING intensity < 0.8)
    pub async fn is_available(&self, location: &str) -> Result<bool> {
        let intensity = self.get_intensity(location, Some(PheromoneType::Working)).await?;
        Ok(intensity < 0.8)
    }
}

/// Agent wrapper for stigmergic coordination
pub struct StigmergicAgent {
    agent_id: String,
    server: Arc<StigmergyServer>,
    current_task: Arc<RwLock<Option<String>>>,
}

impl StigmergicAgent {
    /// Create new agent
    pub fn new(agent_id: String, server: Arc<StigmergyServer>) -> Self {
        Self {
            agent_id,
            server,
            current_task: Arc::new(RwLock::new(None)),
        }
    }

    /// Express interest in a location
    pub async fn express_interest(&self, location: &str) -> Result<()> {
        let pheromone = DigitalPheromone::new(
            location.to_string(),
            PheromoneType::Interest,
            0.5,
            0.2, // Fast decay (3s half-life)
            self.agent_id.clone(),
        );
        self.server.leave_trace(pheromone).await?;
        debug!("Agent {} expressed interest in {}", self.agent_id, location);
        Ok(())
    }

    /// Check if location is available
    pub async fn is_available(&self, location: &str) -> Result<bool> {
        self.server.is_available(location).await
    }

    /// Claim work at location
    pub async fn claim_work(&self, location: &str) -> Result<bool> {
        // Check if agent already has a task
        {
            let task = self.current_task.read().await;
            if task.is_some() {
                return Ok(false);
            }
        }

        // Check availability
        if !self.is_available(location).await? {
            return Ok(false);
        }

        // Claim work
        let pheromone = DigitalPheromone::new(
            location.to_string(),
            PheromoneType::Working,
            1.0,
            0.05, // Slow decay (14s half-life)
            self.agent_id.clone(),
        );

        let success = self.server.leave_trace(pheromone).await?;

        if success {
            let mut task = self.current_task.write().await;
            *task = Some(location.to_string());
            info!("Agent {} claimed work at {}", self.agent_id, location);
        }

        Ok(success)
    }

    /// Complete work at location
    pub async fn complete_work(&self, location: &str) -> Result<()> {
        // Mark as success
        let pheromone = DigitalPheromone::new(
            location.to_string(),
            PheromoneType::Success,
            1.0,
            0.02, // Slow decay (34s half-life)
            self.agent_id.clone(),
        );
        self.server.leave_trace(pheromone).await?;

        // Clear working pheromone
        self.server.clear_pheromone(
            location,
            PheromoneType::Working,
            Some(&self.agent_id),
        ).await?;

        // Clear current task
        let mut task = self.current_task.write().await;
        *task = None;

        info!("Agent {} completed work at {}", self.agent_id, location);
        Ok(())
    }

    /// Mark location as blocked
    pub async fn mark_blocked(&self, location: &str) -> Result<()> {
        let pheromone = DigitalPheromone::new(
            location.to_string(),
            PheromoneType::Blocked,
            2.0,
            0.01, // Very slow decay (69s half-life)
            self.agent_id.clone(),
        );
        self.server.leave_trace(pheromone).await?;
        info!("Agent {} marked {} as blocked", self.agent_id, location);
        Ok(())
    }

    /// Mark location as confusing (question pheromone)
    pub async fn mark_confusing(&self, location: &str) -> Result<()> {
        let pheromone = DigitalPheromone::new(
            location.to_string(),
            PheromoneType::Question,
            0.3,
            0.001, // Very slow decay (693s / ~11.5min half-life)
            self.agent_id.clone(),
        );
        self.server.leave_trace(pheromone).await?;
        debug!("Agent {} marked {} as confusing", self.agent_id, location);
        Ok(())
    }
}
