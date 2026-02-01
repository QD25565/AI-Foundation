//! Database Connection Pool
//!
//! PostgreSQL connection pooling using deadpool-postgres.
//! Provides efficient connection reuse for high-throughput API.

use deadpool_postgres::{Config, Manager, ManagerConfig, Pool, RecyclingMethod, Runtime};
use tokio_postgres::NoTls;

/// Type alias for the connection pool
pub type DbPool = Pool;

/// Type alias for a pooled connection
pub type DbConn = deadpool_postgres::Object;

/// Create a new database connection pool
pub async fn create_pool(database_url: &str) -> anyhow::Result<DbPool> {
    // Parse the URL to extract components
    let parsed = parse_database_url(database_url)?;

    let mut cfg = Config::new();
    cfg.host = Some(parsed.host);
    cfg.port = Some(parsed.port);
    cfg.dbname = Some(parsed.dbname);
    cfg.user = Some(parsed.user);
    cfg.password = Some(parsed.password);

    cfg.manager = Some(ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    });

    let pool = cfg.create_pool(Some(Runtime::Tokio1), NoTls)?;

    // Test the connection
    let conn = pool.get().await?;
    let row = conn.query_one("SELECT 1", &[]).await?;
    let _: i32 = row.get(0);

    tracing::info!("Database pool created successfully");
    Ok(pool)
}

struct ParsedUrl {
    host: String,
    port: u16,
    dbname: String,
    user: String,
    password: String,
}

fn parse_database_url(url: &str) -> anyhow::Result<ParsedUrl> {
    // postgres://user:pass@host:port/dbname
    let url = url.strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))
        .ok_or_else(|| anyhow::anyhow!("Invalid database URL scheme"))?;

    // Split user:pass@host:port/dbname
    let (auth, rest) = url.split_once('@')
        .ok_or_else(|| anyhow::anyhow!("Missing @ in database URL"))?;

    let (user, password) = auth.split_once(':')
        .ok_or_else(|| anyhow::anyhow!("Missing password in database URL"))?;

    let (host_port, dbname) = rest.split_once('/')
        .ok_or_else(|| anyhow::anyhow!("Missing database name in URL"))?;

    let (host, port_str) = host_port.split_once(':')
        .unwrap_or((host_port, "5432"));

    let port: u16 = port_str.parse()
        .map_err(|_| anyhow::anyhow!("Invalid port number"))?;

    Ok(ParsedUrl {
        host: host.to_string(),
        port,
        dbname: dbname.to_string(),
        user: user.to_string(),
        password: password.to_string(),
    })
}

/// Database queries for the gateway
pub mod queries {
    use super::DbConn;
    use crate::error::{ApiError, ApiResult};
    use chrono::{DateTime, Utc};
    use uuid::Uuid;

    // ============ API Keys ============

    pub async fn get_api_key(conn: &DbConn, key: &str) -> ApiResult<Option<ApiKeyRecord>> {
        let row = conn
            .query_opt(
                "SELECT id, ai_id, tier, created_at, last_used, revoked
                 FROM api_keys WHERE key_hash = $1",
                &[&hash_key(key)],
            )
            .await?;

        Ok(row.map(|r| ApiKeyRecord {
            id: r.get("id"),
            ai_id: r.get("ai_id"),
            tier: r.get("tier"),
            created_at: r.get("created_at"),
            last_used: r.get("last_used"),
            revoked: r.get("revoked"),
        }))
    }

    pub async fn create_api_key(
        conn: &DbConn,
        ai_id: &str,
        tier: &str,
    ) -> ApiResult<(Uuid, String)> {
        let key = generate_api_key();
        let key_hash = hash_key(&key);
        let id = Uuid::new_v4();

        conn.execute(
            "INSERT INTO api_keys (id, ai_id, key_hash, tier, created_at)
             VALUES ($1, $2, $3, $4, NOW())",
            &[&id, &ai_id, &key_hash, &tier],
        )
        .await?;

        Ok((id, key))
    }

    pub async fn update_api_key_last_used(conn: &DbConn, key: &str) -> ApiResult<()> {
        conn.execute(
            "UPDATE api_keys SET last_used = NOW() WHERE key_hash = $1",
            &[&hash_key(key)],
        )
        .await?;
        Ok(())
    }

    // ============ AI Registration ============

    pub async fn register_ai(
        conn: &DbConn,
        ai_id: &str,
        display_name: Option<&str>,
        password_hash: &str,
    ) -> ApiResult<()> {
        conn.execute(
            "INSERT INTO registered_ais (ai_id, display_name, password_hash, created_at)
             VALUES ($1, $2, $3, NOW())
             ON CONFLICT (ai_id) DO NOTHING",
            &[&ai_id, &display_name, &password_hash],
        )
        .await?;
        Ok(())
    }

    pub async fn get_ai_password_hash(conn: &DbConn, ai_id: &str) -> ApiResult<Option<String>> {
        let row = conn
            .query_opt(
                "SELECT password_hash FROM registered_ais WHERE ai_id = $1",
                &[&ai_id],
            )
            .await?;
        Ok(row.map(|r| r.get("password_hash")))
    }

    // ============ Messages ============

    pub async fn send_dm(
        conn: &DbConn,
        from_ai: &str,
        to_ai: &str,
        content: &str,
    ) -> ApiResult<i64> {
        let row = conn
            .query_one(
                "INSERT INTO direct_messages (from_ai, to_ai, content, sent_at)
                 VALUES ($1, $2, $3, NOW())
                 RETURNING id",
                &[&from_ai, &to_ai, &content],
            )
            .await?;
        Ok(row.get("id"))
    }

    pub async fn get_dms(
        conn: &DbConn,
        ai_id: &str,
        limit: i64,
    ) -> ApiResult<Vec<DirectMessage>> {
        let rows = conn
            .query(
                "SELECT id, from_ai, to_ai, content, sent_at, read_at
                 FROM direct_messages
                 WHERE to_ai = $1
                 ORDER BY sent_at DESC
                 LIMIT $2",
                &[&ai_id, &limit],
            )
            .await?;

        Ok(rows
            .into_iter()
            .map(|r| DirectMessage {
                id: r.get("id"),
                from_ai: r.get("from_ai"),
                to_ai: r.get("to_ai"),
                content: r.get("content"),
                sent_at: r.get("sent_at"),
                read_at: r.get("read_at"),
            })
            .collect())
    }

    pub async fn send_broadcast(
        conn: &DbConn,
        from_ai: &str,
        content: &str,
        channel: &str,
    ) -> ApiResult<i64> {
        let row = conn
            .query_one(
                "INSERT INTO broadcasts (from_ai, content, channel, sent_at)
                 VALUES ($1, $2, $3, NOW())
                 RETURNING id",
                &[&from_ai, &content, &channel],
            )
            .await?;
        Ok(row.get("id"))
    }

    pub async fn get_broadcasts(
        conn: &DbConn,
        channel: Option<&str>,
        limit: i64,
    ) -> ApiResult<Vec<Broadcast>> {
        let rows = if let Some(ch) = channel {
            conn.query(
                "SELECT id, from_ai, content, channel, sent_at
                 FROM broadcasts
                 WHERE channel = $1
                 ORDER BY sent_at DESC
                 LIMIT $2",
                &[&ch, &limit],
            )
            .await?
        } else {
            conn.query(
                "SELECT id, from_ai, content, channel, sent_at
                 FROM broadcasts
                 ORDER BY sent_at DESC
                 LIMIT $1",
                &[&limit],
            )
            .await?
        };

        Ok(rows
            .into_iter()
            .map(|r| Broadcast {
                id: r.get("id"),
                from_ai: r.get("from_ai"),
                content: r.get("content"),
                channel: r.get("channel"),
                sent_at: r.get("sent_at"),
            })
            .collect())
    }

    // ============ Helper functions ============

    fn generate_api_key() -> String {
        format!("aif_{}", Uuid::new_v4().to_string().replace("-", ""))
    }

    fn hash_key(key: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    // ============ Record types ============

    #[derive(Debug)]
    pub struct ApiKeyRecord {
        pub id: Uuid,
        pub ai_id: String,
        pub tier: String,
        pub created_at: DateTime<Utc>,
        pub last_used: Option<DateTime<Utc>>,
        pub revoked: bool,
    }

    #[derive(Debug, serde::Serialize)]
    pub struct DirectMessage {
        pub id: i64,
        pub from_ai: String,
        pub to_ai: String,
        pub content: String,
        pub sent_at: DateTime<Utc>,
        pub read_at: Option<DateTime<Utc>>,
    }

    #[derive(Debug, serde::Serialize)]
    pub struct Broadcast {
        pub id: i64,
        pub from_ai: String,
        pub content: String,
        pub channel: String,
        pub sent_at: DateTime<Utc>,
    }
}
