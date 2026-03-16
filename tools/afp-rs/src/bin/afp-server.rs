//! AFP Server Binary
//!
//! Runs the AI-Foundation Protocol server for a teambook.
//!
//! Usage:
//!   afp-server --ai-id server-001 --teambook my-team
//!   afp-server --quic-port 31415 --ws-port 31416

use afp::{server::{AFPServer, ServerConfig}, DEFAULT_QUIC_PORT, DEFAULT_WS_PORT};
use clap::Parser;
use std::net::SocketAddr;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser, Debug)]
#[command(name = "afp-server")]
#[command(about = "AI-Foundation Protocol Server")]
#[command(version)]
struct Args {
    /// AI ID for this server
    #[arg(long, env = "AI_ID")]
    ai_id: Option<String>,

    /// Teambook name
    #[arg(long, short = 't', default_value = "default")]
    teambook: String,

    /// QUIC port
    #[arg(long, default_value_t = DEFAULT_QUIC_PORT)]
    quic_port: u16,

    /// WebSocket port
    #[arg(long, default_value_t = DEFAULT_WS_PORT)]
    ws_port: u16,

    /// Bind address
    #[arg(long, default_value = "0.0.0.0")]
    bind: String,

    /// PostgreSQL URL (optional, for persistence)
    #[arg(long, env = "POSTGRES_URL")]
    postgres_url: Option<String>,

    /// Redis URL (optional, for pub/sub)
    #[arg(long, env = "REDIS_URL")]
    redis_url: Option<String>,

    /// Verbose output
    #[arg(long, short = 'v')]
    verbose: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Setup logging
    let level = if args.verbose { Level::DEBUG } else { Level::INFO };
    FmtSubscriber::builder()
        .with_max_level(level)
        .with_target(false)
        .compact()
        .init();

    // Generate AI ID if not provided
    let ai_id = args.ai_id.unwrap_or_else(|| {
        let suffix: u32 = rand::random::<u32>() % 1000;
        format!("afp-server-{}", suffix)
    });

    info!("AI-Foundation Protocol Server");
    info!("AI ID: {}", ai_id);
    info!("Teambook: {}", args.teambook);

    // Create config
    let quic_addr: SocketAddr = format!("{}:{}", args.bind, args.quic_port).parse()?;
    let ws_addr: SocketAddr = format!("{}:{}", args.bind, args.ws_port).parse()?;

    let config = ServerConfig {
        quic_addr,
        ws_addr,
        teambook_name: args.teambook,
        teambook_id: uuid::Uuid::new_v4().to_string(),
        postgres_url: args.postgres_url,
        redis_url: args.redis_url,
    };

    // Create and run server
    let server = AFPServer::new(config, &ai_id).await?;

    info!("Starting server...");
    info!("QUIC: {}", quic_addr);
    info!("WebSocket: {}", ws_addr);

    // Handle Ctrl+C
    let server_ref = std::sync::Arc::new(server);
    let server_clone = server_ref.clone();

    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        info!("Shutdown signal received");
        server_clone.shutdown().await.ok();
    });

    server_ref.run().await?;

    info!("Server stopped");
    Ok(())
}
