//! Metrics CLI - High-Performance Metrics Collection and Analysis
//!
//! Tracks tool usage patterns, token consumption, and identifies optimization opportunities.
//!
//! Usage:
//!   metrics-cli record <function> <module> <duration_ms> [tokens]
//!   metrics-cli summary [--days N]
//!   metrics-cli candidates [--threshold N]
//!   metrics-cli patterns [--limit N]
//!   metrics-cli stats

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use clap::{Parser, Subcommand};
use deadpool_postgres::{Config, Pool, Runtime};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use tokio_postgres::NoTls;
use uuid::Uuid;

// ============================================================================
// DATA STRUCTURES
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FunctionCallRecord {
    id: Uuid,
    function_name: String,
    module: String,
    duration_ms: f64,
    tokens_used: i32,
    success: bool,
    error: Option<String>,
    ai_id: String,
    timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MetricsSummary {
    total_calls: i64,
    total_tokens: i64,
    total_duration_ms: f64,
    avg_duration_ms: f64,
    success_rate: f64,
    unique_functions: i64,
    period_days: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FunctionStats {
    function_name: String,
    module: String,
    call_count: i64,
    total_tokens: i64,
    avg_duration_ms: f64,
    success_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConsolidationCandidate {
    pattern: String,
    occurrence_count: i64,
    potential_savings: String,
    recommendation: String,
}

// ============================================================================
// CLI
// ============================================================================

#[derive(Parser)]
#[command(name = "metrics-cli")]
#[command(about = "High-performance metrics collection and analysis", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Record a function call metric
    #[command(alias = "log", alias = "track", alias = "add", alias = "save")]
    Record {
        /// Function name (e.g., remember, recall)
        #[arg(value_name = "FUNCTION")]
        function: String,

        /// Module name (e.g., notebook, teambook)
        #[arg(value_name = "MODULE")]
        module: String,

        /// Duration in milliseconds (e.g., 150.5)
        #[arg(value_name = "DURATION_MS")]
        duration_ms: f64,

        /// Tokens used (e.g., 100)
        #[arg(value_name = "TOKENS", default_value = "0")]
        tokens: i32,

        /// Mark as failed
        #[arg(long)]
        failed: bool,

        /// Error message (if failed)
        #[arg(long)]
        error: Option<String>,

        // Hidden flag fallbacks
        #[arg(long = "function", hide = true)]
        function_flag: Option<String>,
        #[arg(long = "module", hide = true)]
        module_flag: Option<String>,
    },

    /// Show metrics summary
    #[command(alias = "overview", alias = "report", alias = "show", alias = "view")]
    Summary {
        /// Number of days to summarize (e.g., 7, 30)
        #[arg(long, default_value = "7")]
        days: i32,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Identify consolidation candidates (repeated patterns)
    #[command(alias = "optimize", alias = "suggestions", alias = "hints", alias = "improvements")]
    Candidates {
        /// Minimum occurrences to be a candidate (e.g., 3, 5)
        #[arg(long, default_value = "3")]
        threshold: i64,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show function call patterns
    #[command(alias = "top", alias = "frequent", alias = "popular", alias = "usage")]
    Patterns {
        /// Maximum patterns to show (e.g., 10, 20)
        #[arg(long, default_value = "10")]
        limit: i64,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show database statistics
    #[command(alias = "info", alias = "status", alias = "db")]
    Stats,

    /// Initialize metrics database tables
    #[command(alias = "setup", alias = "create-tables", alias = "migrate")]
    Init,

    /// Clear old metrics (older than N days)
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

    // Parse the URL manually for deadpool
    let mut config = Config::new();

    // Simple URL parsing: postgresql://user:pass@host:port/database
    let url = url.strip_prefix("postgresql://").unwrap_or(&url);
    let url = url.strip_prefix("postgres://").unwrap_or(url);

    // Split user:pass@host:port/database
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

    // Fallbacks
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

    client.execute(
        "CREATE TABLE IF NOT EXISTS metrics_function_calls (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            function_name VARCHAR(200) NOT NULL,
            module VARCHAR(200) NOT NULL,
            duration_ms DOUBLE PRECISION NOT NULL,
            tokens_used INTEGER DEFAULT 0,
            success BOOLEAN DEFAULT TRUE,
            error TEXT,
            ai_id VARCHAR(100) NOT NULL,
            timestamp TIMESTAMPTZ DEFAULT NOW()
        )",
        &[],
    ).await?;

    client.execute(
        "CREATE INDEX IF NOT EXISTS idx_metrics_timestamp ON metrics_function_calls(timestamp)",
        &[],
    ).await?;

    client.execute(
        "CREATE INDEX IF NOT EXISTS idx_metrics_function ON metrics_function_calls(function_name, module)",
        &[],
    ).await?;

    client.execute(
        "CREATE INDEX IF NOT EXISTS idx_metrics_ai ON metrics_function_calls(ai_id)",
        &[],
    ).await?;

    println!("[+] Metrics tables initialized");
    Ok(())
}

// ============================================================================
// COMMANDS
// ============================================================================

async fn cmd_record(
    pool: &Pool,
    function: &str,
    module: &str,
    duration_ms: f64,
    tokens: i32,
    failed: bool,
    error: Option<&str>,
) -> Result<()> {
    let client = pool.get().await?;
    let ai_id = get_ai_id();

    client.execute(
        "INSERT INTO metrics_function_calls
         (function_name, module, duration_ms, tokens_used, success, error, ai_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[&function, &module, &duration_ms, &tokens, &(!failed), &error, &ai_id],
    ).await?;

    println!("[+] Recorded: {}.{} ({}ms, {} tokens)", module, function, duration_ms as i64, tokens);
    Ok(())
}

async fn cmd_summary(pool: &Pool, days: i32, json: bool) -> Result<()> {
    let client = pool.get().await?;
    let since = Utc::now() - Duration::days(days as i64);

    let row = client.query_one(
        "SELECT
            COUNT(*)::BIGINT as total_calls,
            COALESCE(SUM(tokens_used)::BIGINT, 0) as total_tokens,
            COALESCE(SUM(duration_ms)::FLOAT8, 0) as total_duration,
            COALESCE(AVG(duration_ms)::FLOAT8, 0) as avg_duration,
            COALESCE(AVG(CASE WHEN success THEN 1.0 ELSE 0.0 END)::FLOAT8 * 100, 0) as success_rate,
            COUNT(DISTINCT function_name || '.' || module)::BIGINT as unique_functions
         FROM metrics_function_calls
         WHERE timestamp > $1",
        &[&since],
    ).await?;

    let summary = MetricsSummary {
        total_calls: row.get::<_, i64>(0),
        total_tokens: row.get::<_, i64>(1),
        total_duration_ms: row.get::<_, f64>(2),
        avg_duration_ms: row.get::<_, f64>(3),
        success_rate: row.get::<_, f64>(4),
        unique_functions: row.get::<_, i64>(5),
        period_days: days,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        println!("=== METRICS SUMMARY (last {} days) ===", days);
        println!("  Total calls:      {}", summary.total_calls);
        println!("  Total tokens:     {}", summary.total_tokens);
        println!("  Total duration:   {:.1}s", summary.total_duration_ms / 1000.0);
        println!("  Avg duration:     {:.1}ms", summary.avg_duration_ms);
        println!("  Success rate:     {:.1}%", summary.success_rate);
        println!("  Unique functions: {}", summary.unique_functions);
    }

    Ok(())
}

async fn cmd_candidates(pool: &Pool, threshold: i64, json: bool) -> Result<()> {
    let client = pool.get().await?;
    let since = Utc::now() - Duration::days(7);

    // Find functions called repeatedly in sequence (potential batch candidates)
    let rows = client.query(
        "WITH call_sequences AS (
            SELECT
                function_name || '.' || module as func,
                COUNT(*) as call_count,
                SUM(tokens_used) as total_tokens,
                AVG(duration_ms) as avg_duration
            FROM metrics_function_calls
            WHERE timestamp > $1
            GROUP BY function_name, module
            HAVING COUNT(*) >= $2
            ORDER BY COUNT(*) DESC
            LIMIT 10
        )
        SELECT func, call_count, total_tokens, avg_duration FROM call_sequences",
        &[&since, &threshold],
    ).await?;

    let candidates: Vec<ConsolidationCandidate> = rows.iter().map(|row| {
        let func: String = row.get(0);
        let count: i64 = row.get(1);
        let tokens: i64 = row.get(2);
        let avg_ms: f64 = row.get(3);

        ConsolidationCandidate {
            pattern: func.clone(),
            occurrence_count: count,
            potential_savings: format!("~{} tokens, ~{:.0}ms", tokens / 2, avg_ms * (count as f64) / 2.0),
            recommendation: if count > 10 {
                "Consider batching multiple calls".to_string()
            } else {
                "Monitor for patterns".to_string()
            },
        }
    }).collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&candidates)?);
    } else {
        if candidates.is_empty() {
            println!("No consolidation candidates found (threshold: {} calls)", threshold);
            return Ok(());
        }

        println!("=== CONSOLIDATION CANDIDATES ===");
        for candidate in &candidates {
            println!();
            println!("  Pattern: {}", candidate.pattern);
            println!("  Occurrences: {}", candidate.occurrence_count);
            println!("  Potential savings: {}", candidate.potential_savings);
            println!("  Recommendation: {}", candidate.recommendation);
        }
    }

    Ok(())
}

async fn cmd_patterns(pool: &Pool, limit: i64, json: bool) -> Result<()> {
    let client = pool.get().await?;
    let since = Utc::now() - Duration::days(7);

    let rows = client.query(
        "SELECT
            function_name,
            module,
            COUNT(*) as call_count,
            COALESCE(SUM(tokens_used), 0) as total_tokens,
            COALESCE(AVG(duration_ms), 0) as avg_duration,
            COALESCE(AVG(CASE WHEN success THEN 1.0 ELSE 0.0 END) * 100, 0) as success_rate
         FROM metrics_function_calls
         WHERE timestamp > $1
         GROUP BY function_name, module
         ORDER BY call_count DESC
         LIMIT $2",
        &[&since, &limit],
    ).await?;

    let stats: Vec<FunctionStats> = rows.iter().map(|row| {
        FunctionStats {
            function_name: row.get(0),
            module: row.get(1),
            call_count: row.get(2),
            total_tokens: row.get(3),
            avg_duration_ms: row.get(4),
            success_rate: row.get(5),
        }
    }).collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        if stats.is_empty() {
            println!("No function call patterns found");
            return Ok(());
        }

        println!("=== TOP FUNCTION PATTERNS (last 7 days) ===");
        println!("{:<30} {:<20} {:>8} {:>10} {:>10} {:>8}",
                 "Function", "Module", "Calls", "Tokens", "Avg(ms)", "Success");
        println!("{}", "-".repeat(90));

        for stat in &stats {
            let func_name = if stat.function_name.len() > 28 {
                format!("{}...", &stat.function_name[..25])
            } else {
                stat.function_name.clone()
            };

            let module_name = if stat.module.len() > 18 {
                format!("{}...", &stat.module[..15])
            } else {
                stat.module.clone()
            };

            println!("{:<30} {:<20} {:>8} {:>10} {:>10.1} {:>7.1}%",
                     func_name,
                     module_name,
                     stat.call_count,
                     stat.total_tokens,
                     stat.avg_duration_ms,
                     stat.success_rate);
        }
    }

    Ok(())
}

async fn cmd_stats(pool: &Pool) -> Result<()> {
    let client = pool.get().await?;

    let row = client.query_one(
        "SELECT
            COUNT(*) as total_records,
            MIN(timestamp) as oldest,
            MAX(timestamp) as newest,
            COUNT(DISTINCT ai_id) as unique_ais
         FROM metrics_function_calls",
        &[],
    ).await?;

    let total: i64 = row.get(0);
    let oldest: Option<DateTime<Utc>> = row.get(1);
    let newest: Option<DateTime<Utc>> = row.get(2);
    let unique_ais: i64 = row.get(3);

    println!("=== METRICS DATABASE STATS ===");
    println!("  Total records: {}", total);
    println!("  Unique AIs:    {}", unique_ais);

    if let (Some(old), Some(new)) = (oldest, newest) {
        println!("  Oldest record: {}", old.format("%Y-%m-%d %H:%M UTC"));
        println!("  Newest record: {}", new.format("%Y-%m-%d %H:%M UTC"));

        let span = new.signed_duration_since(old);
        println!("  Time span:     {} days", span.num_days());
    }

    Ok(())
}

async fn cmd_cleanup(pool: &Pool, days: i32) -> Result<()> {
    let client = pool.get().await?;
    let cutoff = Utc::now() - Duration::days(days as i64);

    let result = client.execute(
        "DELETE FROM metrics_function_calls WHERE timestamp < $1",
        &[&cutoff],
    ).await?;

    println!("[+] Deleted {} old records (older than {} days)", result, days);
    Ok(())
}

// ============================================================================
// MAIN
// ============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let pool = create_pool().await
        .context("Failed to create database pool")?;

    match cli.command {
        Commands::Init => {
            init_tables(&pool).await?;
        }
        Commands::Record { function, module, duration_ms, tokens, failed, error, .. } => {
            cmd_record(&pool, &function, &module, duration_ms, tokens, failed, error.as_deref()).await?;
        }
        Commands::Summary { days, json } => {
            cmd_summary(&pool, days, json).await?;
        }
        Commands::Candidates { threshold, json } => {
            cmd_candidates(&pool, threshold, json).await?;
        }
        Commands::Patterns { limit, json } => {
            cmd_patterns(&pool, limit, json).await?;
        }
        Commands::Stats => {
            cmd_stats(&pool).await?;
        }
        Commands::Cleanup { days } => {
            cmd_cleanup(&pool, days).await?;
        }
    }

    Ok(())
}
