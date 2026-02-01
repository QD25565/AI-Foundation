//! Federation Node CLI
//!
//! Run a federation node that participates in the Deep Net mesh.

use federation::{
    FederationNode, SharingPreferences, Endpoint,
    discovery::{DiscoveryConfig, DiscoveryManager},
};
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use std::net::SocketAddr;
use tracing::{info, warn, error};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Default QUIC port for federation
const DEFAULT_PORT: u16 = 31420;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_subscriber::EnvFilter::from_default_env()
            .add_directive("federation=info".parse()?))
        .init();

    info!("AI-Foundation Federation Node v{}", env!("CARGO_PKG_VERSION"));

    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    match command {
        "start" => start_node(&args[2..]).await?,
        "discover" => discover_peers(&args[2..]).await?,
        "connect" => connect_to_peer(&args[2..]).await?,
        "passkey" => generate_passkey(&args[2..]).await?,
        "help" | "--help" | "-h" => print_help(),
        _ => {
            error!("Unknown command: {}", command);
            print_help();
        }
    }

    Ok(())
}

fn print_help() {
    println!(r#"
AI-Foundation Federation Node

USAGE:
    federation-node <COMMAND> [OPTIONS]

COMMANDS:
    start       Start the federation node
    discover    Discover peers on the network
    connect     Connect to a specific peer
    passkey     Generate a pairing passkey
    help        Show this help message

EXAMPLES:
    federation-node start --port 31420 --name "My Node"
    federation-node discover --timeout 30
    federation-node connect quic://192.168.1.100:31420
    federation-node passkey --ttl 300

ENVIRONMENT:
    RUST_LOG=federation=debug    Enable debug logging
"#);
}

async fn start_node(args: &[String]) -> anyhow::Result<()> {
    // Parse arguments
    let mut port = DEFAULT_PORT;
    let mut name = "Federation Node".to_string();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" => {
                if i + 1 < args.len() {
                    port = args[i + 1].parse()?;
                    i += 1;
                }
            }
            "--name" | "-n" => {
                if i + 1 < args.len() {
                    name = args[i + 1].clone();
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    info!("Starting federation node: {}", name);
    info!("Listening on port: {}", port);

    // Generate identity
    let signing_key = SigningKey::generate(&mut OsRng);
    let node = FederationNode::new_local(&name, &signing_key);

    info!("Node ID: {}", node.node_id);
    info!("Trust Level: {:?}", node.trust_level);

    // Set up discovery
    let config = DiscoveryConfig::default();
    let mut discovery = DiscoveryManager::new(&node.node_id, config);

    info!("Starting discovery...");
    discovery.start().await?;

    // Main loop
    info!("Node is running. Press Ctrl+C to stop.");
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Shutting down...");
                break;
            }
            Some(event) = discovery.next_event() => {
                match event {
                    federation::discovery::DiscoveryEvent::PeerFound(peer) => {
                        info!("Discovered peer: {:?} via {:?}",
                            peer.node_id.as_deref().unwrap_or("unknown"),
                            peer.discovery_type);
                    }
                    federation::discovery::DiscoveryEvent::PeerLost { node_id, .. } => {
                        warn!("Lost peer: {:?}", node_id);
                    }
                    federation::discovery::DiscoveryEvent::Error(e) => {
                        warn!("Discovery error: {}", e);
                    }
                    _ => {}
                }
            }
        }
    }

    discovery.stop().await?;
    info!("Node stopped.");

    Ok(())
}

async fn discover_peers(args: &[String]) -> anyhow::Result<()> {
    let mut timeout_secs = 10u64;

    for i in 0..args.len() {
        if args[i] == "--timeout" || args[i] == "-t" {
            if i + 1 < args.len() {
                timeout_secs = args[i + 1].parse()?;
            }
        }
    }

    info!("Discovering peers for {} seconds...", timeout_secs);

    let config = DiscoveryConfig::default();
    let mut discovery = DiscoveryManager::new("discovery-scan", config);
    discovery.start().await?;

    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(timeout_secs);

    while tokio::time::Instant::now() < deadline {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                break;
            }
            Some(event) = discovery.next_event() => {
                if let federation::discovery::DiscoveryEvent::PeerFound(peer) = event {
                    println!("Found: {} - {} ({:?})",
                        peer.node_id.as_deref().unwrap_or("unknown"),
                        peer.display_name.as_deref().unwrap_or("unnamed"),
                        peer.discovery_type);
                }
            }
        }
    }

    discovery.stop().await?;

    let peers = discovery.known_peers();
    println!("\nDiscovered {} peers", peers.len());

    Ok(())
}

async fn connect_to_peer(args: &[String]) -> anyhow::Result<()> {
    if args.is_empty() {
        println!("Usage: federation-node connect <endpoint>");
        println!("  Examples:");
        println!("    federation-node connect quic://192.168.1.100:31420");
        println!("    federation-node connect mdns://my-node._ai-foundation._udp.local");
        return Ok(());
    }

    let endpoint_str = &args[0];
    info!("Connecting to: {}", endpoint_str);

    // Parse endpoint URL
    let endpoint = if endpoint_str.starts_with("quic://") {
        let addr_str = endpoint_str.strip_prefix("quic://").unwrap();
        let addr: SocketAddr = addr_str.parse()?;
        Endpoint::quic(addr)
    } else if endpoint_str.starts_with("mdns://") {
        let service = endpoint_str.strip_prefix("mdns://").unwrap();
        Endpoint::mdns(service)
    } else {
        anyhow::bail!("Unknown endpoint format: {}", endpoint_str);
    };

    info!("Endpoint: {}", endpoint.description());

    // TODO: Implement actual connection logic
    info!("Connection logic not yet implemented");

    Ok(())
}

async fn generate_passkey(args: &[String]) -> anyhow::Result<()> {
    use federation::discovery::passkey::{generate_passkey, EndpointInfo, format_passkey_display, DEFAULT_TTL};

    let mut ttl_secs = DEFAULT_TTL.as_secs();
    let mut port = DEFAULT_PORT;

    for i in 0..args.len() {
        match args[i].as_str() {
            "--ttl" => {
                if i + 1 < args.len() {
                    ttl_secs = args[i + 1].parse()?;
                }
            }
            "--port" => {
                if i + 1 < args.len() {
                    port = args[i + 1].parse()?;
                }
            }
            _ => {}
        }
    }

    // Generate identity for the passkey
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();
    let node_id = federation::node_id_from_pubkey(&verifying_key);
    let pubkey_hex = hex::encode(verifying_key.as_bytes());

    // Get local IP (simplified - would need better detection in production)
    let local_addr = format!("0.0.0.0:{}", port);
    let endpoint_info = EndpointInfo::quic(&local_addr, None);

    let ttl = std::time::Duration::from_secs(ttl_secs);
    let passkey = generate_passkey(&node_id, "Federation Node", endpoint_info, &pubkey_hex, ttl)?;

    println!("\n=== Federation Passkey ===");
    println!("Code: {}", format_passkey_display(&passkey.code));
    println!("Valid for: {} seconds", ttl_secs);
    println!("Expires: {:?}", passkey.remaining_time());
    println!("\nShare this code with the peer you want to connect to.");
    println!("They should run: federation-node connect passkey://{}", passkey.code);

    Ok(())
}
