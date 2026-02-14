//! Deep Net P2P Test Binary
//!
//! A simple peer-to-peer test tool for verifying QUIC transport and mesh connectivity.
//!
//! Usage:
//!   # Run as server (listener):
//!   p2p_test server --name "Node A" --port 31415
//!
//!   # Run as client (connector):
//!   p2p_test client --name "Node B" --target 127.0.0.1:31415
//!
//!   # Run with mDNS discovery:
//!   p2p_test discover --name "Node C"

use deepnet_core::{
    identity::NodeIdentity,
    message::{MessageEnvelope, MessagePayload, PresenceStatus, VectorClock},
    quic::QuicTransport,
    transport::{Connection, NodeAddress, Transport},
    discovery::{MdnsDiscovery, Discovery},
};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

/// Configuration
const DEFAULT_PORT: u16 = 31415;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for logs
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("deepnet=debug".parse().unwrap())
                .add_directive("p2p_test=debug".parse().unwrap()),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        return Ok(());
    }

    match args[1].as_str() {
        "server" => run_server(&args[2..]).await,
        "client" => run_client(&args[2..]).await,
        "discover" => run_discover(&args[2..]).await,
        "chat" => run_chat(&args[2..]).await,
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        _ => {
            eprintln!("Unknown command: {}", args[1]);
            print_usage();
            Ok(())
        }
    }
}

fn print_usage() {
    println!(r#"
Deep Net P2P Test Tool
======================

Usage:
  p2p_test <command> [options]

Commands:
  server    Run as a listening server
  client    Connect to a remote peer
  discover  Run mDNS discovery and list peers
  chat      Interactive chat mode (server + client)

Server options:
  --name <name>   Display name for this node (default: "Server")
  --port <port>   Port to listen on (default: 31415)

Client options:
  --name <name>   Display name for this node (default: "Client")
  --target <addr> Target address to connect to (e.g., 127.0.0.1:31415)

Discover options:
  --name <name>   Display name for this node
  --timeout <sec> How long to scan (default: 10)

Chat options:
  --name <name>   Display name for this node
  --port <port>   Port to listen on (default: 31415)
  --target <addr> Optional: target to connect to immediately

Examples:
  # Terminal 1: Run server
  p2p_test server --name "Alice"

  # Terminal 2: Connect and chat
  p2p_test client --name "Bob" --target 127.0.0.1:31415

  # Discover peers on LAN
  p2p_test discover --name "Scanner" --timeout 5
"#);
}

fn parse_args(args: &[String]) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let mut i = 0;
    while i < args.len() {
        if args[i].starts_with("--") && i + 1 < args.len() {
            let key = args[i][2..].to_string();
            let value = args[i + 1].clone();
            map.insert(key, value);
            i += 2;
        } else {
            i += 1;
        }
    }
    map
}

/// Run as a listening server
async fn run_server(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let opts = parse_args(args);
    let name = opts.get("name").cloned().unwrap_or_else(|| "Server".to_string());
    let port: u16 = opts.get("port").and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT);

    // Generate identity
    let identity = Arc::new(NodeIdentity::generate(name.clone()));
    println!("\n=== Deep Net Server ===");
    println!("  Name:    {}", name);
    println!("  Node ID: {}", identity.node_id().to_hex());
    println!("  Short:   {}", identity.node_id().short());
    println!("  Port:    {}", port);
    println!();

    // Create and start QUIC transport
    let bind_addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;
    let mut transport = QuicTransport::with_bind_addr(identity.clone(), bind_addr);
    transport.start()?;

    println!("Listening on {}...", transport.local_addr().unwrap_or(bind_addr));
    println!("Press Ctrl+C to stop.\n");

    // Accept connections
    let mut listener = transport.listen().await?;

    loop {
        println!("Waiting for connection...");
        match listener.accept().await {
            Ok(mut conn) => {
                let peer_id = conn.peer_id().short();
                println!("\n>>> Connection from peer: {}", peer_id);

                // Handle messages in a task
                tokio::spawn(async move {
                    loop {
                        match conn.recv().await {
                            Ok(msg) => {
                                handle_message(&msg);

                                // Echo back with acknowledgment
                                let reply = MessageEnvelope::broadcast(
                                    *conn.peer_id(),
                                    VectorClock::new(),
                                    format!("ACK: Message {} received", msg.id.to_hex()),
                                    None,
                                );
                                if let Err(e) = conn.send(&reply).await {
                                    eprintln!("Failed to send reply: {}", e);
                                    break;
                                }
                            }
                            Err(e) => {
                                eprintln!("Connection error: {}", e);
                                break;
                            }
                        }
                    }
                    println!("<<< Peer {} disconnected", peer_id);
                });
            }
            Err(e) => {
                eprintln!("Accept error: {}", e);
            }
        }
    }
}

/// Run as a connecting client
async fn run_client(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let opts = parse_args(args);
    let name = opts.get("name").cloned().unwrap_or_else(|| "Client".to_string());
    let target = opts.get("target").cloned()
        .ok_or("Missing --target argument")?;

    // Generate identity
    let identity = Arc::new(NodeIdentity::generate(name.clone()));
    println!("\n=== Deep Net Client ===");
    println!("  Name:    {}", name);
    println!("  Node ID: {}", identity.node_id().to_hex());
    println!("  Short:   {}", identity.node_id().short());
    println!("  Target:  {}", target);
    println!();

    // Create QUIC transport (client-only, random port)
    let bind_addr: SocketAddr = "0.0.0.0:0".parse()?;
    let mut transport = QuicTransport::with_bind_addr(identity.clone(), bind_addr);
    transport.start()?;

    // Connect to target
    let target_addr: SocketAddr = target.parse()?;
    let addr = NodeAddress::Quic {
        addr: target_addr,
        server_name: Some("deepnet".to_string()),
    };

    println!("Connecting to {}...", target);
    let mut conn = transport.connect(&addr).await?;
    println!("Connected! Peer ID: {}", conn.peer_id().short());

    // Send a hello message
    let hello = MessageEnvelope::broadcast(
        identity.node_id(),
        VectorClock::new(),
        format!("Hello from {}!", name),
        Some("general".to_string()),
    );
    conn.send(&hello).await?;
    println!("Sent hello message: {}", hello.id.to_hex());

    // Send a presence update
    let presence = MessageEnvelope::presence(
        identity.node_id(),
        VectorClock::new(),
        PresenceStatus::Online,
    );
    conn.send(&presence).await?;
    println!("Sent presence update");

    // Wait for reply
    println!("\nWaiting for response...");
    match tokio::time::timeout(Duration::from_secs(5), conn.recv()).await {
        Ok(Ok(msg)) => {
            handle_message(&msg);
        }
        Ok(Err(e)) => {
            eprintln!("Receive error: {}", e);
        }
        Err(_) => {
            println!("Timeout waiting for response");
        }
    }

    // Interactive mode
    println!("\nEnter messages (or 'quit' to exit):");
    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();

    loop {
        print!("> ");
        // Note: this won't actually print before waiting due to buffering,
        // but it's for demonstration

        match lines.next_line().await {
            Ok(Some(line)) => {
                if line.trim() == "quit" || line.trim() == "exit" {
                    break;
                }

                let msg = MessageEnvelope::broadcast(
                    identity.node_id(),
                    VectorClock::new(),
                    line,
                    None,
                );

                if let Err(e) = conn.send(&msg).await {
                    eprintln!("Send error: {}", e);
                    break;
                }

                // Wait for response
                match tokio::time::timeout(Duration::from_secs(2), conn.recv()).await {
                    Ok(Ok(reply)) => handle_message(&reply),
                    Ok(Err(e)) => eprintln!("Receive error: {}", e),
                    Err(_) => {} // Timeout is ok for chat
                }
            }
            Ok(None) => break, // EOF
            Err(e) => {
                eprintln!("Read error: {}", e);
                break;
            }
        }
    }

    conn.close().await?;
    println!("Disconnected.");
    Ok(())
}

/// Run mDNS discovery
async fn run_discover(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let opts = parse_args(args);
    let name = opts.get("name").cloned().unwrap_or_else(|| "Scanner".to_string());
    let timeout: u64 = opts.get("timeout").and_then(|t| t.parse().ok()).unwrap_or(10);

    let identity = Arc::new(NodeIdentity::generate(name.clone()));
    println!("\n=== Deep Net Discovery ===");
    println!("  Name:    {}", name);
    println!("  Node ID: {}", identity.node_id().short());
    println!("  Timeout: {}s", timeout);
    println!();

    // Start mDNS discovery
    let mut mdns = MdnsDiscovery::new();
    mdns.start()?;

    // Announce ourselves
    mdns.announce(&identity.manifest).await?;
    println!("Announced on mDNS");

    // Scan for peers
    println!("Scanning for {} seconds...\n", timeout);

    for i in 0..timeout {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let nodes = mdns.discover().await?;
        println!("--- Scan {} ---", i + 1);

        if nodes.is_empty() {
            println!("  No peers found yet");
        } else {
            for node in &nodes {
                println!("  Found: {} ({})",
                    node.metadata.get("display_name").unwrap_or(&"Unknown".to_string()),
                    node.node_id.short()
                );
                for addr in &node.addresses {
                    println!("    Address: {:?}", addr);
                }
            }
        }
        println!();
    }

    mdns.unannounce().await?;
    mdns.stop();

    Ok(())
}

/// Interactive chat mode
async fn run_chat(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let opts = parse_args(args);
    let name = opts.get("name").cloned().unwrap_or_else(|| "ChatNode".to_string());
    let port: u16 = opts.get("port").and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT);
    let target = opts.get("target").cloned();

    let identity = Arc::new(NodeIdentity::generate(name.clone()));
    println!("\n=== Deep Net Chat ===");
    println!("  Name:    {}", name);
    println!("  Node ID: {}", identity.node_id().short());
    println!("  Port:    {}", port);
    if let Some(ref t) = target {
        println!("  Target:  {}", t);
    }
    println!();

    // Start QUIC transport
    let bind_addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;
    let mut transport = QuicTransport::with_bind_addr(identity.clone(), bind_addr);
    transport.start()?;

    println!("Listening on port {}", port);

    // If we have a target, connect to it
    if let Some(target_str) = target {
        let target_addr: SocketAddr = target_str.parse()?;
        let addr = NodeAddress::Quic {
            addr: target_addr,
            server_name: Some("deepnet".to_string()),
        };

        println!("Connecting to {}...", target_str);
        let mut conn = transport.connect(&addr).await?;
        println!("Connected! Peer: {}", conn.peer_id().short());

        // Send hello
        let hello = MessageEnvelope::broadcast(
            identity.node_id(),
            VectorClock::new(),
            format!("[{}] joined the chat", name),
            None,
        );
        conn.send(&hello).await?;

        // Start chat loop
        chat_loop(&identity, &mut conn).await?;
    } else {
        // Server mode - accept connections
        let mut listener = transport.listen().await?;
        println!("Waiting for connection...");

        let mut conn = listener.accept().await?;
        println!("Connected! Peer: {}", conn.peer_id().short());

        // Start chat loop
        chat_loop(&identity, &mut conn).await?;
    }

    Ok(())
}

async fn chat_loop(
    identity: &Arc<NodeIdentity>,
    conn: &mut Box<dyn Connection>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\nChat started! Type messages and press Enter.");
    println!("Commands: /ping, /metrics, /quit\n");

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(32);

    // Spawn task to read stdin
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let stdin = BufReader::new(tokio::io::stdin());
        let mut lines = stdin.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if tx_clone.send(line).await.is_err() {
                break;
            }
        }
    });

    loop {
        tokio::select! {
            // Handle incoming messages
            result = conn.recv() => {
                match result {
                    Ok(msg) => {
                        if let MessagePayload::Broadcast(ref b) = msg.payload {
                            println!("[{}]: {}", msg.origin.short(), b.content);
                        } else {
                            handle_message(&msg);
                        }
                    }
                    Err(e) => {
                        eprintln!("Connection error: {}", e);
                        break;
                    }
                }
            }

            // Handle user input
            Some(line) = rx.recv() => {
                let line = line.trim();

                match line {
                    "/quit" | "/exit" => {
                        println!("Goodbye!");
                        break;
                    }
                    "/ping" => {
                        match conn.ping().await {
                            Ok(rtt) => println!("Pong! RTT: {}ms", rtt),
                            Err(e) => eprintln!("Ping failed: {}", e),
                        }
                    }
                    "/metrics" => {
                        let m = conn.metrics();
                        println!("Connection metrics:");
                        println!("  Latency: {}ms", m.latency_ms);
                        println!("  Bandwidth: {:?}", m.bandwidth);
                        println!("  Encrypted: {}", m.encrypted);
                    }
                    _ if !line.is_empty() => {
                        let msg = MessageEnvelope::broadcast(
                            identity.node_id(),
                            VectorClock::new(),
                            line.to_string(),
                            None,
                        );
                        if let Err(e) = conn.send(&msg).await {
                            eprintln!("Send error: {}", e);
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    conn.close().await?;
    Ok(())
}

fn handle_message(msg: &MessageEnvelope) {
    match &msg.payload {
        MessagePayload::Broadcast(b) => {
            println!("[{}] Broadcast: {}", msg.origin.short(), b.content);
        }
        MessagePayload::Presence(p) => {
            println!("[{}] Presence: {:?}", p.node_id.short(), p.status);
        }
        MessagePayload::DirectMessage(dm) => {
            println!("[{}→{}] DM: {} bytes",
                msg.origin.short(),
                dm.to.short(),
                dm.content.len()
            );
        }
        MessagePayload::Ping { nonce } => {
            println!("[{}] Ping: {}", msg.origin.short(), nonce);
        }
        MessagePayload::Pong { nonce } => {
            println!("[{}] Pong: {}", msg.origin.short(), nonce);
        }
        MessagePayload::Ack { message_id } => {
            println!("[{}] ACK: {}", msg.origin.short(), message_id.to_hex());
        }
        _ => {
            println!("[{}] Other message type", msg.origin.short());
        }
    }
}
