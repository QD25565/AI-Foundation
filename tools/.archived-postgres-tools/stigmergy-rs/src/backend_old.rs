//! PostgreSQL backend for stigmergy with connection pooling
//!
//! Features:
//! - Async tokio-postgres with deadpool connection pooling
//! - Thread-safe operations
//! - Automatic schema initialization
//! - Efficient queries with indexes
//! - Activity-based pheromone refresh

use crate::{DigitalPheromone, PheromoneType};
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use deadpool_postgres::{Config, ManagerConfig, Pool, RecyclingMethod, Runtime};
use tokio_postgres::NoTls;
use tracing::{debug, info};

/// PostgreSQL stigmergy backend with connection pooling
pub struct PostgreSQLBackend {
    pool: Pool,
}

impl PostgreSQLBackend {
    /// Create new backend with connection pooling
    pub async fn new(database_url: &str) -> Result<Self> {
        // Parse database URL
        let config = database_url.parse::<tokio_postgres::Config>()
            .context("Failed to parse database URL")?;

        // Configure connection pool
        let mut pool_config = Config::new();
        pool_config.dbname = config.get_dbname().map(|s| s.to_string());
        pool_config.host = config.get_hosts().first().map(|h| {
            match h {
                tokio_postgres::config::Host::Tcp(s) => s.clone(),
                tokio_postgres::config::Host::Unix(p) => p.to_string_lossy().to_string(),
            }
        });
        pool_config.port = config.get_ports().first().copied();
        pool_config.user = config.get_user().map(|s| s.to_string());
        pool_config.password = config.get_password().map(|p| String::from_utf8_lossy(p).to_string());

        pool_config.manager = Some(ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        });

        // Create pool
        let pool = pool_config.create_pool(Some(Runtime::Tokio1), NoTls)
            .context("Failed to create connection pool")?;

        info!("PostgreSQL connection pool created");

        let backend = Self { pool };
        backend.init_schema().await?;

        Ok(backend)
    }

    /// Initialize database schema
    async fn init_schema(&self) -> Result<()> {
        let client = self.pool.get().await
            .context("Failed to get connection from pool")?;

        // Create pheromones table
        client.execute(
            "CREATE TABLE IF NOT EXISTS pheromones (
                id SERIAL PRIMARY KEY,
                location VARCHAR(255) NOT NULL,
                pheromone_type VARCHAR(50) NOT NULL,
                intensity FLOAT NOT NULL,
                decay_rate FLOAT NOT NULL,
                agent_id VARCHAR(100) NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                expires_at TIMESTAMPTZ NOT NULL
            )",
            &[],
        ).await.context("Failed to create pheromones table")?;

        // Create indexes for fast queries
        client.execute(
            "CREATE INDEX IF NOT EXISTS idx_pheromones_location
             ON pheromones(location, pheromone_type)",
            &[],
        ).await.context("Failed to create location index")?;

        client.execute(
            "CREATE INDEX IF NOT EXISTS idx_pheromones_expires
             ON pheromones(expires_at)",
            &[],
        ).await.context("Failed to create expiration index")?;

        client.execute(
            "CREATE INDEX IF NOT EXISTS idx_pheromones_agent
             ON pheromones(agent_id)",
            &[],
        ).await.context("Failed to create agent index")?;

        info!("Schema initialized");
        Ok(())
    }

    /// Leave a pheromone trace
    pub async fn leave_trace(&self, pheromone: &DigitalPheromone) -> Result<bool> {
        let client = self.pool.get().await
            .context("Failed to get connection")?;

        // Calculate expiration
        let expires_at = if let Some(exp_seconds) = pheromone.expiration_seconds(0.01) {
            pheromone.created_at + Duration::seconds(exp_seconds as i64)
        } else {
            // No decay - default to 5 minutes
            pheromone.created_at + Duration::minutes(5)
        };

        // Insert pheromone
        let rows = client.execute(
            "INSERT INTO pheromones
             (location, pheromone_type, intensity, decay_rate, agent_id, created_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[
                &pheromone.location,
                &pheromone.pheromone_type.to_string(),
                &pheromone.intensity,
                &pheromone.decay_rate,
                &pheromone.agent_id,
                &pheromone.created_at,
                &expires_at,
            ],
        ).await.context("Failed to insert pheromone")?;

        debug!(
            "Pheromone deposited: {} at {} (expires in {}s)",
            pheromone.pheromone_type,
            pheromone.location,
            (expires_at - pheromone.created_at).num_seconds()
        );

        Ok(rows > 0)
    }

    /// Sense pheromones at a location
    pub async fn sense_environment(
        &self,
        location: &str,
        pheromone_type: Option<PheromoneType>,
        threshold: f64,
    ) -> Result<Vec<DigitalPheromone>> {
        let client = self.pool.get().await
            .context("Failed to get connection")?;

        let query = if let Some(ptype) = pheromone_type {
            client.query(
                "SELECT location, pheromone_type, intensity, decay_rate, agent_id, created_at
                 FROM pheromones
                 WHERE location = $1 AND pheromone_type = $2 AND expires_at > NOW()",
                &[&location, &ptype.to_string()],
            ).await
        } else {
            client.query(
                "SELECT location, pheromone_type, intensity, decay_rate, agent_id, created_at
                 FROM pheromones
                 WHERE location = $1 AND expires_at > NOW()",
                &[&location],
            ).await
        }.context("Failed to query pheromones")?;

        let now = Utc::now();
        let mut pheromones = Vec::new();

        for row in query {
            let location: String = row.get(0);
            let ptype_str: String = row.get(1);
            let intensity: f64 = row.get(2);
            let decay_rate: f64 = row.get(3);
            let agent_id: String = row.get(4);
            let created_at: DateTime<Utc> = row.get(5);

            let ptype = PheromoneType::from_str(&ptype_str)
                .ok_or_else(|| anyhow::anyhow!("Invalid pheromone type: {}", ptype_str))?;

            let pheromone = DigitalPheromone {
                location,
                pheromone_type: ptype,
                intensity,
                decay_rate,
                agent_id,
                created_at,
            };

            // Filter by current intensity
            if pheromone.current_intensity(Some(now)) >= threshold {
                pheromones.push(pheromone);
            }
        }

        Ok(pheromones)
    }

    /// Get total intensity at location
    pub async fn get_intensity(
        &self,
        location: &str,
        pheromone_type: Option<PheromoneType>,
    ) -> Result<f64> {
        let pheromones = self.sense_environment(location, pheromone_type, 0.01).await?;
        let now = Utc::now();
        Ok(pheromones.iter()
            .map(|p| p.current_intensity(Some(now)))
            .sum())
    }

    /// Refresh existing pheromone (activity-based refresh)
    pub async fn refresh_pheromone(
        &self,
        location: &str,
        pheromone_type: PheromoneType,
        agent_id: &str,
    ) -> Result<bool> {
        let client = self.pool.get().await
            .context("Failed to get connection")?;

        let rows = client.execute(
            "UPDATE pheromones
             SET created_at = NOW(),
                 expires_at = NOW() + INTERVAL '1 second' * (
                     CASE
                         WHEN decay_rate >= 1.0 THEN 0
                         WHEN decay_rate = 0.0 THEN 300
                         ELSE LOG(0.01 / intensity) / LOG(1 - decay_rate)
                     END
                 )
             WHERE location = $1
               AND pheromone_type = $2
               AND agent_id = $3
               AND expires_at > NOW()",
            &[&location, &pheromone_type.to_string(), &agent_id],
        ).await.context("Failed to refresh pheromone")?;

        if rows > 0 {
            debug!("Pheromone refreshed: {} at {} for {}", pheromone_type, location, agent_id);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Clear pheromones at location
    pub async fn clear_pheromone(
        &self,
        location: &str,
        pheromone_type: PheromoneType,
        agent_id: Option<&str>,
    ) -> Result<()> {
        let client = self.pool.get().await
            .context("Failed to get connection")?;

        if let Some(aid) = agent_id {
            client.execute(
                "DELETE FROM pheromones
                 WHERE location = $1 AND pheromone_type = $2 AND agent_id = $3",
                &[&location, &pheromone_type.to_string(), &aid],
            ).await
        } else {
            client.execute(
                "DELETE FROM pheromones
                 WHERE location = $1 AND pheromone_type = $2",
                &[&location, &pheromone_type.to_string()],
            ).await
        }.context("Failed to clear pheromone")?;

        Ok(())
    }

    /// Cleanup expired pheromones (garbage collection)
    pub async fn cleanup_expired(&self) -> Result<u64> {
        let client = self.pool.get().await
            .context("Failed to get connection")?;

        let rows = client.execute(
            "DELETE FROM pheromones WHERE expires_at < NOW()",
            &[],
        ).await.context("Failed to cleanup expired pheromones")?;

        if rows > 0 {
            info!("Cleaned up {} expired pheromones", rows);
        }

        Ok(rows)
    }
}
