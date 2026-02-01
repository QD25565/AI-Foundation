//! AI-Foundation STUN Server
//!
//! Lightweight STUN server for NAT traversal discovery.
//! Helps AIs discover their public IP address and port mapping.
//!
//! ## Protocol Support
//!
//! - RFC 5389: Session Traversal Utilities for NAT (STUN)
//! - RFC 8489: STUN (updated)
//!
//! ## Usage
//!
//! ```bash
//! stun-server --bind 0.0.0.0:3478
//! ```
//!
//! ## How It Works
//!
//! 1. Client sends STUN Binding Request
//! 2. Server receives request, notes client's source IP:port
//! 3. Server sends Binding Response with XOR-MAPPED-ADDRESS
//! 4. Client now knows its public IP:port as seen by the server

use clap::Parser;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::net::UdpSocket;
use tracing::{debug, error, info, warn, Level};

mod stun_handler;

use stun_handler::StunHandler;

/// AI-Foundation STUN Server
#[derive(Parser, Debug)]
#[command(name = "stun-server")]
#[command(about = "STUN Server for AI-Foundation NAT Traversal")]
#[command(version)]
struct Args {
    /// Bind address for UDP
    #[arg(short, long, default_value = "0.0.0.0:3478")]
    bind: SocketAddr,

    /// Secondary bind address (for NAT behavior discovery)
    #[arg(long)]
    bind_alt: Option<SocketAddr>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(short, long, default_value = "info")]
    log_level: String,

    /// Enable RFC 5780 NAT behavior discovery
    #[arg(long)]
    nat_discovery: bool,

    /// Print stats every N seconds (0 = disabled)
    #[arg(long, default_value = "60")]
    stats_interval: u64,
}

/// Server statistics
struct Stats {
    requests_total: AtomicU64,
    responses_total: AtomicU64,
    errors_total: AtomicU64,
    bytes_received: AtomicU64,
    bytes_sent: AtomicU64,
}

impl Stats {
    fn new() -> Self {
        Self {
            requests_total: AtomicU64::new(0),
            responses_total: AtomicU64::new(0),
            errors_total: AtomicU64::new(0),
            bytes_received: AtomicU64::new(0),
            bytes_sent: AtomicU64::new(0),
        }
    }

    fn record_request(&self, bytes: usize) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
        self.bytes_received.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    fn record_response(&self, bytes: usize) {
        self.responses_total.fetch_add(1, Ordering::Relaxed);
        self.bytes_sent.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    fn record_error(&self) {
        self.errors_total.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            requests_total: self.requests_total.load(Ordering::Relaxed),
            responses_total: self.responses_total.load(Ordering::Relaxed),
            errors_total: self.errors_total.load(Ordering::Relaxed),
            bytes_received: self.bytes_received.load(Ordering::Relaxed),
            bytes_sent: self.bytes_sent.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug)]
struct StatsSnapshot {
    requests_total: u64,
    responses_total: u64,
    errors_total: u64,
    bytes_received: u64,
    bytes_sent: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Initialize logging
    let level = match args.log_level.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    };

    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        .init();

    info!("AI-Foundation STUN Server v{}", env!("CARGO_PKG_VERSION"));
    info!("Binding to {}", args.bind);

    // Create UDP socket
    let socket = UdpSocket::bind(args.bind).await?;
    let socket = Arc::new(socket);

    // Get actual bound address (useful if port was 0)
    let local_addr = socket.local_addr()?;
    info!("Listening on {}", local_addr);

    // Optional secondary socket for NAT behavior discovery
    let alt_socket = if let Some(alt_bind) = args.bind_alt {
        info!("Secondary socket binding to {}", alt_bind);
        let sock = UdpSocket::bind(alt_bind).await?;
        info!("Secondary socket listening on {}", sock.local_addr()?);
        Some(Arc::new(sock))
    } else {
        None
    };

    // Initialize stats
    let stats = Arc::new(Stats::new());

    // Initialize STUN handler
    let handler = Arc::new(StunHandler::new(local_addr, alt_socket.as_ref().map(|s| {
        s.local_addr().unwrap_or(local_addr)
    })));

    // Spawn stats printer
    if args.stats_interval > 0 {
        let stats_clone = stats.clone();
        let interval = args.stats_interval;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval));
            loop {
                ticker.tick().await;
                let s = stats_clone.snapshot();
                info!(
                    "Stats | requests: {} | responses: {} | errors: {} | rx: {} bytes | tx: {} bytes",
                    s.requests_total, s.responses_total, s.errors_total,
                    s.bytes_received, s.bytes_sent
                );
            }
        });
    }

    // Main receive loop
    let mut buf = vec![0u8; 2048];

    info!("STUN server ready - awaiting Binding requests");

    loop {
        match socket.recv_from(&mut buf).await {
            Ok((len, src_addr)) => {
                stats.record_request(len);
                debug!("Received {} bytes from {}", len, src_addr);

                // Process STUN message
                match handler.handle_message(&buf[..len], src_addr) {
                    Ok(Some(response)) => {
                        // Send response
                        match socket.send_to(&response, src_addr).await {
                            Ok(sent) => {
                                stats.record_response(sent);
                                debug!("Sent {} bytes to {}", sent, src_addr);
                            }
                            Err(e) => {
                                stats.record_error();
                                warn!("Failed to send response to {}: {}", src_addr, e);
                            }
                        }
                    }
                    Ok(None) => {
                        // No response needed (e.g., indication)
                        debug!("No response for message from {}", src_addr);
                    }
                    Err(e) => {
                        stats.record_error();
                        debug!("Error handling message from {}: {}", src_addr, e);
                    }
                }
            }
            Err(e) => {
                stats.record_error();
                error!("Socket receive error: {}", e);
            }
        }
    }
}
