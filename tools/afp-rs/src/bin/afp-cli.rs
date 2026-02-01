//! AFP CLI
//!
//! Command-line tool for testing and interacting with AFP servers.
//!
//! Usage:
//!   afp-cli connect 192.168.1.100:31415
//!   afp-cli ping 192.168.1.100:31415
//!   afp-cli info

use afp::{
    client::AFPClient,
    fingerprint::HardwareFingerprint,
    identity::{generate_ai_id, AIIdentity},
    keys::{FallbackStorage, KeyStorage},
    DEFAULT_QUIC_PORT,
};
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser, Debug)]
#[command(name = "afp-cli")]
#[command(about = "AI-Foundation Protocol CLI")]
#[command(version)]
struct Args {
    /// AI ID for this client
    #[arg(long, env = "AI_ID")]
    ai_id: Option<String>,

    /// Use WebSocket instead of QUIC
    #[arg(long)]
    websocket: bool,

    /// Verbose output
    #[arg(long, short = 'v')]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Connect to a server and show connection info
    Connect {
        /// Server address (host:port)
        #[arg(value_name = "ADDRESS")]
        address: String,
    },

    /// Ping a server and measure latency
    Ping {
        /// Server address (host:port)
        #[arg(value_name = "ADDRESS")]
        address: String,

        /// Number of pings
        #[arg(short = 'c', default_value = "3")]
        count: u32,
    },

    /// Show local identity information
    Info,

    /// Generate a new identity
    Generate {
        /// Name prefix for the AI ID
        #[arg(default_value = "ai")]
        name: String,
    },

    /// Show hardware fingerprint
    Fingerprint,

    /// Delete stored identity
    Delete {
        /// AI ID to delete
        ai_id: String,

        /// Confirm deletion
        #[arg(long)]
        confirm: bool,
    },
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

    // Get or generate AI ID
    let ai_id = args.ai_id.unwrap_or_else(|| {
        // Try to load existing, or generate new
        let storage = FallbackStorage::default_chain("default");
        if storage.exists("default") {
            "default".to_string()
        } else {
            generate_ai_id("afp-cli")
        }
    });

    match args.command {
        Commands::Connect { address } => {
            let addr = parse_address(&address)?;
            println!("Connecting to {}...", addr);

            let mut client = AFPClient::new(&ai_id)?;
            client.connect(addr, args.websocket).await?;

            println!("CONNECTED");
            println!("AI ID: {}", client.ai_id());
            println!("Fingerprint: {}", client.fingerprint());
            println!("Trust Level: {:?}", client.trust_level());
            println!("Teambook: {}", client.teambook().unwrap_or("unknown"));

            client.disconnect().await?;
        }

        Commands::Ping { address, count } => {
            let addr = parse_address(&address)?;
            println!("Pinging {}...", addr);

            let mut client = AFPClient::new(&ai_id)?;
            client.connect(addr, args.websocket).await?;

            let mut latencies = Vec::new();
            for i in 0..count {
                match client.ping().await {
                    Ok(latency) => {
                        println!("Ping {}: {}ms", i + 1, latency);
                        latencies.push(latency);
                    }
                    Err(e) => {
                        println!("Ping {}: error - {}", i + 1, e);
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }

            if !latencies.is_empty() {
                let avg = latencies.iter().sum::<u64>() / latencies.len() as u64;
                let min = *latencies.iter().min().unwrap();
                let max = *latencies.iter().max().unwrap();
                println!();
                println!("Statistics: min={}ms avg={}ms max={}ms", min, avg, max);
            }

            client.disconnect().await?;
        }

        Commands::Info => {
            let storage = FallbackStorage::default_chain(&ai_id);

            if storage.exists(&ai_id) {
                let keypair = storage.load(&ai_id)?;
                let identity = AIIdentity::new(
                    ai_id.clone(),
                    keypair.public_key(),
                    "local".to_string(),
                );

                println!("IDENTITY INFO");
                println!("AI ID: {}", ai_id);
                println!("Fingerprint: {}", identity.fingerprint());
                println!("Public Key: {}", hex::encode(keypair.public_key().as_bytes()));
            } else {
                println!("No identity found for '{}'", ai_id);
                println!("Use 'afp-cli generate' to create one");
            }
        }

        Commands::Generate { name } => {
            let new_id = generate_ai_id(&name);
            let storage = FallbackStorage::default_chain(&new_id);

            let pubkey = storage.generate_and_store(&new_id)?;

            println!("IDENTITY GENERATED");
            println!("AI ID: {}", new_id);
            println!("Public Key: {}", hex::encode(pubkey.as_bytes()));
            println!();
            println!("Use with: afp-cli --ai-id {} connect <address>", new_id);
        }

        Commands::Fingerprint => {
            let fp = HardwareFingerprint::collect()?;

            println!("HARDWARE FINGERPRINT");
            println!("Hash: {}", fp.hash_hex());
            println!("Short: {}", fp.short_hash());
            println!();
            println!("Components:");
            println!("  CPU: {} ({} cores)", fp.cpu_brand, fp.cpu_cores);
            println!("  Memory: {} GB", fp.total_memory / 1024 / 1024 / 1024);
            println!("  Hostname: {}", fp.hostname);
            println!("  OS: {}", fp.os_info);
            if !fp.mac_addresses.is_empty() {
                println!("  MACs: {}", fp.mac_addresses.join(", "));
            }
            if !fp.disk_serials.is_empty() {
                println!("  Disks: {}", fp.disk_serials.join(", "));
            }
        }

        Commands::Delete { ai_id, confirm } => {
            if !confirm {
                println!("Error: Use --confirm to delete identity");
                println!("This will permanently delete the private key for '{}'", ai_id);
                return Ok(());
            }

            let storage = FallbackStorage::default_chain(&ai_id);
            if storage.exists(&ai_id) {
                storage.delete(&ai_id)?;
                println!("Identity '{}' deleted", ai_id);
            } else {
                println!("Identity '{}' not found", ai_id);
            }
        }
    }

    Ok(())
}

fn parse_address(addr: &str) -> anyhow::Result<SocketAddr> {
    // If no port specified, use default
    if !addr.contains(':') {
        format!("{}:{}", addr, DEFAULT_QUIC_PORT).parse().map_err(|e| anyhow::anyhow!("{}", e))
    } else {
        addr.parse().map_err(|e| anyhow::anyhow!("{}", e))
    }
}
