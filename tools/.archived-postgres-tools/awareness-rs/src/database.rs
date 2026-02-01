//! Database connection pooling and management

use anyhow::Result;
use deadpool_postgres::{Config, Pool, Runtime};
use std::env;
use tokio_postgres::NoTls;

/// PostgreSQL connection pool
pub struct DatabasePool {
    pool: Pool,
}

impl DatabasePool {
    /// Create pool from environment variables
    pub async fn from_env() -> Result<Self> {
        let postgres_url = env::var("POSTGRES_URL")
            .unwrap_or_else(|_| {
                "postgresql://ai_foundation:ai_foundation_pass@127.0.0.1:15432/ai_foundation".to_string()
            });

        let mut config = Config::new();
        config.url = Some(postgres_url);
        let pool = config.create_pool(Some(Runtime::Tokio1), NoTls)?;

        Ok(Self { pool })
    }

    /// Get connection from pool
    pub async fn get(&self) -> Result<deadpool_postgres::Object> {
        Ok(self.pool.get().await?)
    }
}

impl Clone for DatabasePool {
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
        }
    }
}
