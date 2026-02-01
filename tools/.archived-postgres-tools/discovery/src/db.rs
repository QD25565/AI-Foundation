//! Database operations for Discovery Registry

use deadpool_postgres::{Config, ManagerConfig, Pool, RecyclingMethod, Runtime};
use tokio_postgres::NoTls;

pub type DbPool = Pool;
pub type DbConn = deadpool_postgres::Object;

/// Create database connection pool
pub async fn create_pool(database_url: &str) -> anyhow::Result<DbPool> {
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

    // Test connection
    let conn = pool.get().await?;
    let _ = conn.query_one("SELECT 1", &[]).await?;

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
    let url = url
        .strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))
        .ok_or_else(|| anyhow::anyhow!("Invalid database URL scheme"))?;

    let (auth, rest) = url
        .split_once('@')
        .ok_or_else(|| anyhow::anyhow!("Missing @ in database URL"))?;

    let (user, password) = auth
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("Missing password in database URL"))?;

    let (host_port, dbname) = rest
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("Missing database name in URL"))?;

    let (host, port_str) = host_port.split_once(':').unwrap_or((host_port, "5432"));

    let port: u16 = port_str
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid port number"))?;

    Ok(ParsedUrl {
        host: host.to_string(),
        port,
        dbname: dbname.to_string(),
        user: user.to_string(),
        password: password.to_string(),
    })
}

/// Initialize database schema for discovery
pub async fn init_schema(pool: &DbPool) -> anyhow::Result<()> {
    let conn = pool.get().await?;

    // Teambooks table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS discovery_teambooks (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT,
            endpoint TEXT NOT NULL,
            public_key TEXT,
            is_public BOOLEAN DEFAULT true,
            member_count INTEGER DEFAULT 0,
            registered_at TIMESTAMPTZ DEFAULT NOW(),
            last_heartbeat TIMESTAMPTZ DEFAULT NOW(),
            metadata JSONB DEFAULT '{}'
        )",
        &[],
    )
    .await?;

    // AIs table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS discovery_ais (
            id TEXT PRIMARY KEY,
            display_name TEXT,
            teambook_id TEXT REFERENCES discovery_teambooks(id),
            status TEXT DEFAULT 'offline',
            capabilities JSONB DEFAULT '[]',
            registered_at TIMESTAMPTZ DEFAULT NOW(),
            last_seen TIMESTAMPTZ DEFAULT NOW(),
            metadata JSONB DEFAULT '{}'
        )",
        &[],
    )
    .await?;

    // Index for searching
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_teambooks_public ON discovery_teambooks(is_public) WHERE is_public = true",
        &[],
    )
    .await?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_ais_teambook ON discovery_ais(teambook_id)",
        &[],
    )
    .await?;

    Ok(())
}
