//! Observe CLI - High-Performance Observability Tools
//!
//! Provides logging, error tracking, and telemetry for AI tools.
//!
//! Usage:
//!   observe-cli error <message> [--context JSON]
//!   observe-cli errors [--limit N] [--ai-id ID]
//!   observe-cli telemetry <event> [--data JSON]
//!   observe-cli stats
//!   observe-cli fingerprint <error_message>

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use clap::{Parser, Subcommand};
use deadpool_postgres::{Config, Pool, Runtime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::env;
use tokio_postgres::NoTls;

// ============================================================================
// DATA STRUCTURES
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ErrorRecord {
    id: i64,
    fingerprint: String,
    message: String,
    context: Option<serde_json::Value>,
    ai_id: String,
    occurrence_count: i32,
    first_seen: DateTime<Utc>,
    last_seen: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TelemetryEvent {
    id: i64,
    event_type: String,
    data: Option<serde_json::Value>,
    ai_id: String,
    timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ObservabilityStats {
    total_errors: i64,
    unique_errors: i64,
    total_telemetry: i64,
    errors_last_24h: i64,
    top_errors: Vec<TopError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TopError {
    fingerprint: String,
    message_preview: String,
    count: i64,
}

// ============================================================================
// CLI
// ============================================================================

#[derive(Parser)]
#[command(name = "observe-cli")]
#[command(about = "High-performance observability: logging, errors, telemetry", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Log an error with automatic fingerprinting
    #[command(alias = "err", alias = "log-error", alias = "report", alias = "track-error")]
    Error {
        /// Error message (e.g., "Connection timeout")
        #[arg(value_name = "MESSAGE")]
        message: String,

        /// Additional context as JSON
        #[arg(long)]
        context: Option<String>,

        /// Stack trace or additional details
        #[arg(long)]
        trace: Option<String>,

        // Hidden flag fallback
        #[arg(long = "message", hide = true)]
        message_flag: Option<String>,
    },

    /// List recent errors
    #[command(alias = "errs", alias = "list-errors", alias = "show-errors", alias = "recent")]
    Errors {
        /// Maximum errors to show (e.g., 10, 50)
        #[arg(long, default_value = "10")]
        limit: i64,

        /// Filter by AI ID (e.g., cascade-230)
        #[arg(long)]
        ai_id: Option<String>,

        /// Show only unique errors (by fingerprint)
        #[arg(long)]
        unique: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Log a telemetry event
    #[command(alias = "tel", alias = "event", alias = "track", alias = "log-event")]
    Telemetry {
        /// Event type (e.g., session_start, tool_used)
        #[arg(value_name = "EVENT_TYPE")]
        event_type: String,

        /// Event data as JSON
        #[arg(long)]
        data: Option<String>,

        // Hidden flag fallback
        #[arg(long = "event-type", hide = true)]
        event_type_flag: Option<String>,
    },

    /// Show observability statistics
    #[command(alias = "info", alias = "status", alias = "summary", alias = "overview")]
    Stats {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Generate error fingerprint (for deduplication)
    #[command(alias = "fp", alias = "hash", alias = "dedupe", alias = "signature")]
    Fingerprint {
        /// Error message to fingerprint
        #[arg(value_name = "MESSAGE")]
        message: String,
    },

    /// Initialize observability database tables
    #[command(alias = "setup", alias = "create-tables", alias = "migrate")]
    Init,

    /// Aggregate errors (merge duplicates)
    #[command(alias = "merge", alias = "deduplicate", alias = "consolidate")]
    Aggregate,

    /// Clean up old records
    #[command(alias = "prune", alias = "purge", alias = "clear")]
    Cleanup {
        /// Days to keep (e.g., 30, 90)
        #[arg(value_name = "DAYS", default_value = "30")]
        days: i32,
    },
}

// ============================================================================
// DATABASE
// ============================================================================

fn get_postgres_url() -> String {
    env::var("POSTGRES_URL")
        .or_else(|_| env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgresql://ai_foundation:ai_foundation_pass@127.0.0.1:15432/ai_foundation".to_string())
}

fn get_ai_id() -> String {
    env::var("AI_ID").unwrap_or_else(|_| "unknown".to_string())
}

async fn create_pool() -> Result<Pool> {
    let url = get_postgres_url();

    let mut config = Config::new();

    let url = url.strip_prefix("postgresql://").unwrap_or(&url);
    let url = url.strip_prefix("postgres://").unwrap_or(url);

    let parts: Vec<&str> = url.split('@').collect();
    if parts.len() == 2 {
        // Parse user:password
        let user_pass: Vec<&str> = parts[0].split(':').collect();
        config.user = Some(user_pass[0].to_string());
        if user_pass.len() > 1 {
            config.password = Some(user_pass[1].to_string());
        }

        // Parse host:port/database
        let host_db: Vec<&str> = parts[1].split('/').collect();
        if !host_db.is_empty() {
            let host_port: Vec<&str> = host_db[0].split(':').collect();
            config.host = Some(host_port[0].to_string());
            if host_port.len() > 1 {
                config.port = host_port[1].parse().ok();
            }
        }
        if host_db.len() >= 2 {
            config.dbname = Some(host_db[1].to_string());
        }
    }

    if config.host.is_none() { config.host = Some("localhost".to_string()); }
    if config.port.is_none() { config.port = Some(15432); }
    if config.dbname.is_none() { config.dbname = Some("ai_foundation".to_string()); }
    if config.user.is_none() { config.user = Some("ai_foundation".to_string()); }
    if config.password.is_none() { config.password = Some("ai_foundation_pass".to_string()); }

    let pool = config.create_pool(Some(Runtime::Tokio1), NoTls)?;
    Ok(pool)
}

async fn init_tables(pool: &Pool) -> Result<()> {
    let client = pool.get().await?;

    // Error tracking table
    client.execute(
        "CREATE TABLE IF NOT EXISTS observability_errors (
            id BIGSERIAL PRIMARY KEY,
            fingerprint VARCHAR(64) NOT NULL,
            message TEXT NOT NULL,
            context JSONB,
            trace TEXT,
            ai_id VARCHAR(100) NOT NULL,
            occurrence_count INTEGER DEFAULT 1,
            first_seen TIMESTAMPTZ DEFAULT NOW(),
            last_seen TIMESTAMPTZ DEFAULT NOW()
        )",
        &[],
    ).await?;

    client.execute(
        "CREATE INDEX IF NOT EXISTS idx_errors_fingerprint ON observability_errors(fingerprint)",
        &[],
    ).await?;

    client.execute(
        "CREATE INDEX IF NOT EXISTS idx_errors_last_seen ON observability_errors(last_seen)",
        &[],
    ).await?;

    // Telemetry table
    client.execute(
        "CREATE TABLE IF NOT EXISTS observability_telemetry (
            id BIGSERIAL PRIMARY KEY,
            event_type VARCHAR(100) NOT NULL,
            data JSONB,
            ai_id VARCHAR(100) NOT NULL,
            timestamp TIMESTAMPTZ DEFAULT NOW()
        )",
        &[],
    ).await?;

    client.execute(
        "CREATE INDEX IF NOT EXISTS idx_telemetry_type ON observability_telemetry(event_type)",
        &[],
    ).await?;

    client.execute(
        "CREATE INDEX IF NOT EXISTS idx_telemetry_timestamp ON observability_telemetry(timestamp)",
        &[],
    ).await?;

    println!("[+] Observability tables initialized");
    Ok(())
}

// ============================================================================
// FINGERPRINTING
// ============================================================================

fn generate_fingerprint(message: &str) -> String {
    // Normalize the message for fingerprinting:
    // 1. Lowercase
    // 2. Remove numbers (line numbers, IDs, etc.)
    // 3. Remove extra whitespace
    // 4. Hash the result

    let normalized: String = message
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_ascii_digit())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    hex::encode(&hasher.finalize()[..8]) // First 16 hex chars (8 bytes)
}

// ============================================================================
// COMMANDS
// ============================================================================

async fn cmd_error(pool: &Pool, message: &str, context: Option<&str>, trace: Option<&str>) -> Result<()> {
    let client = pool.get().await?;
    let ai_id = get_ai_id();
    let fingerprint = generate_fingerprint(message);

    let context_json: Option<serde_json::Value> = context
        .map(|c| serde_json::from_str(c).unwrap_or(serde_json::json!({"raw": c})));

    // Try to update existing error, otherwise insert new
    let updated = client.execute(
        "UPDATE observability_errors
         SET occurrence_count = occurrence_count + 1,
             last_seen = NOW(),
             context = COALESCE($3, context)
         WHERE fingerprint = $1 AND ai_id = $2",
        &[&fingerprint, &ai_id, &context_json],
    ).await?;

    if updated == 0 {
        client.execute(
            "INSERT INTO observability_errors (fingerprint, message, context, trace, ai_id)
             VALUES ($1, $2, $3, $4, $5)",
            &[&fingerprint, &message, &context_json, &trace, &ai_id],
        ).await?;
        println!("[+] Error logged (new): {}", &fingerprint[..8]);
    } else {
        println!("[+] Error logged (existing): {}", &fingerprint[..8]);
    }

    Ok(())
}

async fn cmd_errors(pool: &Pool, limit: i64, ai_id: Option<&str>, unique: bool, json: bool) -> Result<()> {
    let client = pool.get().await?;

    let query = if unique {
        "SELECT DISTINCT ON (fingerprint)
            id, fingerprint, message, context, ai_id, occurrence_count, first_seen, last_seen
         FROM observability_errors
         WHERE ($1::VARCHAR IS NULL OR ai_id = $1)
         ORDER BY fingerprint, last_seen DESC
         LIMIT $2"
    } else {
        "SELECT id, fingerprint, message, context, ai_id, occurrence_count, first_seen, last_seen
         FROM observability_errors
         WHERE ($1::VARCHAR IS NULL OR ai_id = $1)
         ORDER BY last_seen DESC
         LIMIT $2"
    };

    let rows = client.query(query, &[&ai_id, &limit]).await?;

    let errors: Vec<ErrorRecord> = rows.iter().map(|row| {
        ErrorRecord {
            id: row.get(0),
            fingerprint: row.get(1),
            message: row.get(2),
            context: row.get(3),
            ai_id: row.get(4),
            occurrence_count: row.get(5),
            first_seen: row.get(6),
            last_seen: row.get(7),
        }
    }).collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&errors)?);
    } else {
        if errors.is_empty() {
            println!("No errors found");
            return Ok(());
        }

        println!("=== RECENT ERRORS ===");
        for err in &errors {
            let msg_preview = if err.message.len() > 60 {
                format!("{}...", &err.message[..57])
            } else {
                err.message.clone()
            };

            println!();
            println!("  [{}] {}", &err.fingerprint[..8], msg_preview);
            println!("    AI: {} | Count: {} | Last: {}",
                     err.ai_id,
                     err.occurrence_count,
                     err.last_seen.format("%Y-%m-%d %H:%M"));
        }
    }

    Ok(())
}

async fn cmd_telemetry(pool: &Pool, event_type: &str, data: Option<&str>) -> Result<()> {
    let client = pool.get().await?;
    let ai_id = get_ai_id();

    let data_json: Option<serde_json::Value> = data
        .map(|d| serde_json::from_str(d).unwrap_or(serde_json::json!({"raw": d})));

    client.execute(
        "INSERT INTO observability_telemetry (event_type, data, ai_id) VALUES ($1, $2, $3)",
        &[&event_type, &data_json, &ai_id],
    ).await?;

    println!("[+] Telemetry logged: {}", event_type);
    Ok(())
}

async fn cmd_stats(pool: &Pool, json: bool) -> Result<()> {
    let client = pool.get().await?;
    let since_24h = Utc::now() - Duration::hours(24);

    let row = client.query_one(
        "SELECT
            (SELECT COUNT(*) FROM observability_errors) as total_errors,
            (SELECT COUNT(DISTINCT fingerprint) FROM observability_errors) as unique_errors,
            (SELECT COUNT(*) FROM observability_telemetry) as total_telemetry,
            (SELECT COUNT(*) FROM observability_errors WHERE last_seen > $1) as errors_24h",
        &[&since_24h],
    ).await?;

    let top_errors_rows = client.query(
        "SELECT fingerprint, message, SUM(occurrence_count) as total
         FROM observability_errors
         GROUP BY fingerprint, message
         ORDER BY total DESC
         LIMIT 5",
        &[],
    ).await?;

    let top_errors: Vec<TopError> = top_errors_rows.iter().map(|r| {
        let msg: String = r.get(1);
        TopError {
            fingerprint: r.get(0),
            message_preview: if msg.len() > 50 { format!("{}...", &msg[..47]) } else { msg },
            count: r.get(2),
        }
    }).collect();

    let stats = ObservabilityStats {
        total_errors: row.get(0),
        unique_errors: row.get(1),
        total_telemetry: row.get(2),
        errors_last_24h: row.get(3),
        top_errors,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        println!("=== OBSERVABILITY STATS ===");
        println!("  Total errors:     {}", stats.total_errors);
        println!("  Unique errors:    {}", stats.unique_errors);
        println!("  Errors (24h):     {}", stats.errors_last_24h);
        println!("  Telemetry events: {}", stats.total_telemetry);

        if !stats.top_errors.is_empty() {
            println!();
            println!("Top Errors:");
            for err in &stats.top_errors {
                println!("  [{}] {} ({}x)", &err.fingerprint[..8], err.message_preview, err.count);
            }
        }
    }

    Ok(())
}

fn cmd_fingerprint(message: &str) {
    let fp = generate_fingerprint(message);
    println!("Fingerprint: {}", fp);
    println!("Short:       {}", &fp[..8]);
}

async fn cmd_aggregate(pool: &Pool) -> Result<()> {
    let client = pool.get().await?;

    // Merge duplicate fingerprints
    let result = client.execute(
        "WITH duplicates AS (
            SELECT fingerprint, ai_id, MIN(id) as keep_id,
                   SUM(occurrence_count) as total_count,
                   MIN(first_seen) as first,
                   MAX(last_seen) as last
            FROM observability_errors
            GROUP BY fingerprint, ai_id
            HAVING COUNT(*) > 1
        )
        UPDATE observability_errors e
        SET occurrence_count = d.total_count,
            first_seen = d.first,
            last_seen = d.last
        FROM duplicates d
        WHERE e.id = d.keep_id",
        &[],
    ).await?;

    // Delete the duplicates
    let deleted = client.execute(
        "DELETE FROM observability_errors e
         WHERE EXISTS (
            SELECT 1 FROM observability_errors e2
            WHERE e2.fingerprint = e.fingerprint
              AND e2.ai_id = e.ai_id
              AND e2.id < e.id
         )",
        &[],
    ).await?;

    println!("[+] Aggregated {} errors, removed {} duplicates", result, deleted);
    Ok(())
}

async fn cmd_cleanup(pool: &Pool, days: i32) -> Result<()> {
    let client = pool.get().await?;
    let cutoff = Utc::now() - Duration::days(days as i64);

    let errors_deleted = client.execute(
        "DELETE FROM observability_errors WHERE last_seen < $1",
        &[&cutoff],
    ).await?;

    let telemetry_deleted = client.execute(
        "DELETE FROM observability_telemetry WHERE timestamp < $1",
        &[&cutoff],
    ).await?;

    println!("[+] Cleaned up: {} errors, {} telemetry events (older than {} days)",
             errors_deleted, telemetry_deleted, days);
    Ok(())
}

// ============================================================================
// MAIN
// ============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Fingerprint { message } => {
            cmd_fingerprint(message);
            return Ok(());
        }
        _ => {}
    }

    let pool = create_pool().await
        .context("Failed to create database pool")?;

    match cli.command {
        Commands::Init => {
            init_tables(&pool).await?;
        }
        Commands::Error { message, context, trace, .. } => {
            cmd_error(&pool, &message, context.as_deref(), trace.as_deref()).await?;
        }
        Commands::Errors { limit, ai_id, unique, json } => {
            cmd_errors(&pool, limit, ai_id.as_deref(), unique, json).await?;
        }
        Commands::Telemetry { event_type, data, .. } => {
            cmd_telemetry(&pool, &event_type, data.as_deref()).await?;
        }
        Commands::Stats { json } => {
            cmd_stats(&pool, json).await?;
        }
        Commands::Aggregate => {
            cmd_aggregate(&pool).await?;
        }
        Commands::Cleanup { days } => {
            cmd_cleanup(&pool, days).await?;
        }
        Commands::Fingerprint { .. } => unreachable!(),
    }

    Ok(())
}
