//! Federation Node — Teambook-to-Teambook P2P Daemon
//!
//! Binds a QUIC endpoint (via iroh), advertises on LAN (via mDNS),
//! accepts incoming connections, runs authenticated handshakes, and
//! syncs events with peers using cursor-tracked replication.
//!
//! # Usage
//!
//! ```sh
//! federation-node start --name "My Teambook"
//! federation-node start --name "My Teambook" --identity /path/to/identity.key
//! federation-node discover --timeout 30
//! federation-node status
//! ```
//!
//! # Identity
//!
//! On first run, generates and persists an Ed25519 keypair at
//! `~/.ai-foundation/federation/identity.key`. This keypair IS the
//! Teambook's permanent identity — never changes, even if renamed.

use federation::{
    FederationNode, PeerSession, QuicTransport, TeambookIdentity,
    ReplicationOrchestrator,
    InboxWriter, InboxState, AiRegistry,
    EventPushRequest, EventPushResponse, PresencePushRequest,
    EventPullRequest, EventPullResponse, SignedEvent,
    FederationMessage, FederationPayload,
    process_push_request, process_presence_request,
    send_message, recv_message, send_message_finish,
    discovery::mdns::{
        MdnsAdvertisement, advertise_service,
        DEFAULT_PORT, PROTOCOL_VERSION as MDNS_PROTO_VERSION,
    },
    discovery::{DiscoveryEvent, DiscoveryManager, DiscoveryConfig},
    gateway::{PeerRegistryConfig, FederationGateway, OutboundEventType},
    manifest::PermissionManifest,
    node_id_from_pubkey,
    session::PROTOCOL_VERSION,
};

// TeamEngram event log integration — for reading local events and serving pull requests
use teamengram::event::EventHeader;
use teamengram::event::event_type as te_event_type;
use teamengram::event_log::EventLogReader;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Mutex, Semaphore};
use tracing::{info, warn, error, debug};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Maximum outbound events buffered for peer catch-up on connect.
const MAX_OUTBOUND_BACKLOG: usize = 1024;

/// Maximum events allowed in a single push batch (inbound).
/// Ed25519 verify is ~8000/sec — 500 events ≈ 60ms of CPU, acceptable.
const MAX_EVENTS_PER_PUSH: usize = 500;

/// Maximum events returned per pull batch (server-side cap).
const MAX_PULL_BATCH_SIZE: usize = 500;

/// Timeout for post-handshake stream operations (send/recv).
const STREAM_TIMEOUT_SECS: u64 = 60;

/// Maximum concurrent bidi streams per peer (prevents resource exhaustion).
const MAX_CONCURRENT_STREAMS: usize = 16;

/// Maximum concurrent inbound connections being processed.
const MAX_CONCURRENT_CONNECTIONS: usize = 64;

/// Stream type byte — first byte on every bidi stream to distinguish protocol.
const STREAM_TYPE_PUSH: u8 = 0x01;
const STREAM_TYPE_PULL: u8 = 0x02;

/// An event queued for outbound federation push.
///
/// Contains pre-serialized FederationMessage bytes and metadata for the
/// outbound consent filter. Signing happens in the push loop, not at
/// injection time — so `event_bytes` are raw, unsigned CBOR.
#[derive(Debug, Clone)]
struct OutboundEvent {
    /// Monotonically increasing local sequence number.
    seq: u64,

    /// Serialized FederationMessage bytes (CBOR) — signed before push.
    event_bytes: Vec<u8>,

    /// Event type for outbound consent filter.
    event_type: OutboundEventType,

    /// AI ID that generated this event (for consent check).
    ai_id: String,
}

/// Active peer session (connected + handshaked).
#[allow(dead_code)]
struct ActivePeer {
    session: PeerSession,
    peer_name: String,
    peer_pubkey_hex: String,
}

/// Shared state accessible from all tasks.
#[allow(dead_code)]
struct NodeState {
    identity: Arc<TeambookIdentity>,
    local_node: FederationNode,
    gateway: FederationGateway,
    replication: Mutex<ReplicationOrchestrator>,
    peers: PeerRegistryConfig,
    active_peers: Mutex<HashMap<String, ActivePeer>>,
    inbox_state: InboxState,

    /// Broadcast channel for outbound events. Each per-peer outbound task
    /// subscribes via `outbound_tx.subscribe()`.
    outbound_tx: tokio::sync::broadcast::Sender<OutboundEvent>,

    /// Next outbound sequence number (monotonic, atomic for lock-free injection).
    outbound_seq: AtomicU64,

    /// Recent outbound events for peer catch-up on connect.
    /// Peers that connect late drain this backlog for events they missed.
    outbound_backlog: Mutex<VecDeque<OutboundEvent>>,

    /// Consent directory for outbound filter checks.
    consent_dir: PathBuf,

    /// Limits concurrent inbound connections being processed.
    /// Arc-wrapped because OwnedSemaphorePermit must be moved into spawned tasks.
    conn_semaphore: Arc<Semaphore>,

    /// Limits concurrent bidi streams per peer in the event loop.
    stream_semaphore: Semaphore,

    /// Path to the teamengram event log file for serving pull requests
    /// and watching for new outbound events.
    /// `None` if the event log doesn't exist (federation runs without local event log).
    event_log_path: Option<PathBuf>,
}

impl NodeState {
    /// Inject a local event into the outbound federation pipeline.
    ///
    /// Called when the local Teambook generates an event that should be
    /// pushed to connected peers. The event is:
    /// 1. Added to the backlog (for late-connecting peers to catch up)
    /// 2. Broadcast to all active per-peer outbound tasks
    ///
    /// Signing and consent filtering happen in the push loop, not here.
    async fn inject_event(
        &self,
        event_bytes: Vec<u8>,
        event_type: OutboundEventType,
        ai_id: String,
    ) {
        let seq = self.outbound_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let event = OutboundEvent {
            seq,
            event_bytes,
            event_type,
            ai_id,
        };

        // Add to backlog for late-connecting peers
        {
            let mut backlog = self.outbound_backlog.lock().await;
            backlog.push_back(event.clone());
            while backlog.len() > MAX_OUTBOUND_BACKLOG {
                backlog.pop_front();
            }
        }

        // Broadcast to all active per-peer outbound loops.
        // Err means no active receivers — that's fine (no peers connected).
        let _ = self.outbound_tx.send(event);
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging to stderr (so test harnesses can read it)
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("federation=info".parse()?),
        )
        .init();

    info!(
        "AI-Foundation Federation Node v{} (protocol v{})",
        env!("CARGO_PKG_VERSION"),
        PROTOCOL_VERSION,
    );

    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    match command {
        "start" => start_node(&args[2..]).await?,
        "discover" => discover_peers(&args[2..]).await?,
        "status" => show_status().await?,
        "help" | "--help" | "-h" => print_help(),
        _ => {
            error!("Unknown command: {}", command);
            print_help();
        }
    }

    Ok(())
}

fn print_help() {
    println!(
        r#"
AI-Foundation Federation Node

USAGE:
    federation-node <COMMAND> [OPTIONS]

COMMANDS:
    start       Start the federation node daemon
    discover    Discover peers on the local network
    status      Show node identity and cursor state
    help        Show this help message

START OPTIONS:
    --name, -n <NAME>       Display name for this Teambook (default: hostname)
    --port, -p <PORT>       QUIC port (default: 31420)
    --no-mdns               Disable mDNS advertisement and discovery

DISCOVER OPTIONS:
    --timeout, -t <SECS>    Scan duration in seconds (default: 10)

ENVIRONMENT:
    RUST_LOG=federation=debug    Enable debug logging
"#
    );
}

// ---------------------------------------------------------------------------
// start — main daemon loop
// ---------------------------------------------------------------------------

async fn start_node(args: &[String]) -> anyhow::Result<()> {
    let mut port = DEFAULT_PORT;
    let mut name = hostname();
    let mut enable_mdns = true;

    // Parse args
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
            "--no-mdns" => {
                enable_mdns = false;
            }
            _ => {}
        }
        i += 1;
    }

    // 1. Load or generate persistent identity
    let identity = TeambookIdentity::load_or_generate().await?;
    let identity = Arc::new(identity);
    let pubkey_hex = identity.public_key_hex();
    let short_id = identity.short_id();
    let node_id = node_id_from_pubkey(identity.verifying_key());

    info!(short_id = %short_id, pubkey = %pubkey_hex, "Identity loaded");

    // 2. Create local FederationNode
    let local_node = FederationNode::new_local(&name, identity.signing_key());

    // 3. Load peer registry
    let peers = PeerRegistryConfig::load_or_default(&PeerRegistryConfig::default_path());
    info!(peer_count = peers.peers.len(), "Peer registry loaded");

    // 4. Create gateway (for signing events and outbound filtering)
    let manifest = PermissionManifest::load_or_default(
        &PermissionManifest::default_path(),
    );
    let gateway = FederationGateway::new(
        reconstruct_identity(&identity),
        &name,
        peers.clone(),
        manifest.clone(),
    );

    // 5. Create inbox state (for inbound event processing pipeline)
    let inbox_writer = InboxWriter::open(InboxWriter::default_path())
        .expect("Failed to open federation inbox — cannot receive events");
    let ai_registry = AiRegistry::new(
        pubkey_hex.clone(),
        short_id.clone(),
        name.clone(),
    );
    let inbox_state = InboxState::new(manifest, ai_registry, inbox_writer);
    info!("Federation inbox ready");

    // 6. Load replication orchestrator
    let replication = ReplicationOrchestrator::new(&pubkey_hex, None)?;

    // 7. Build outbound event channel and shared state
    let (outbound_tx, _) = tokio::sync::broadcast::channel::<OutboundEvent>(256);
    let consent_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ai-foundation")
        .join("federation")
        .join("consent");

    // 7b. Locate teamengram event log (if it exists)
    let event_log_path = {
        let path = teamengram::event_log::event_log_path(None);
        if path.exists() {
            info!(path = %path.display(), "TeamEngram event log found — federation replication enabled");
            Some(path)
        } else {
            warn!(path = %path.display(), "TeamEngram event log not found — pull requests will return empty");
            None
        }
    };

    let state = Arc::new(NodeState {
        identity: identity.clone(),
        local_node: local_node.clone(),
        gateway,
        replication: Mutex::new(replication),
        peers: peers.clone(),
        active_peers: Mutex::new(HashMap::new()),
        inbox_state,
        outbound_tx,
        outbound_seq: AtomicU64::new(0),
        outbound_backlog: Mutex::new(VecDeque::new()),
        consent_dir,
        conn_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS)),
        stream_semaphore: Semaphore::new(MAX_CONCURRENT_STREAMS),
        event_log_path: event_log_path.clone(),
    });

    // 8. Bind QUIC transport (Arc-wrapped for sharing with discovery auto-connect)
    let transport = Arc::new(QuicTransport::bind(&reconstruct_identity(&identity)).await?);
    let endpoint_id = transport.endpoint_id();
    info!(%endpoint_id, port = port, "QUIC transport online");

    // 9. Start mDNS advertising
    let _advertiser = if enable_mdns {
        let ad = MdnsAdvertisement::new(&node_id, &name, &pubkey_hex, port, MDNS_PROTO_VERSION);
        match advertise_service(&ad) {
            Ok(adv) => {
                info!("mDNS advertising as {} on port {}", ad.instance_name, port);
                Some(adv)
            }
            Err(e) => {
                warn!("mDNS advertising failed (continuing without): {e}");
                None
            }
        }
    } else {
        info!("mDNS disabled");
        None
    };

    // 10. Start mDNS discovery + auto-connect in background
    if enable_mdns {
        let config = DiscoveryConfig::default();
        let mut discovery = DiscoveryManager::new(&node_id, config);
        if let Err(e) = discovery.start().await {
            warn!("mDNS discovery failed to start: {e}");
        }

        let disc_state = state.clone();
        let disc_transport = transport.clone();
        tokio::spawn(async move {
            loop {
                match discovery.next_event().await {
                    Some(DiscoveryEvent::PeerFound(peer)) => {
                        let peer_id = peer
                            .node_id
                            .as_deref()
                            .unwrap_or("unknown")
                            .to_string();
                        let peer_name = peer
                            .display_name
                            .as_deref()
                            .unwrap_or("unnamed")
                            .to_string();
                        info!(
                            peer_id = %peer_id,
                            peer_name = %peer_name,
                            discovery = ?peer.discovery_type,
                            "Discovered peer"
                        );

                        // Auto-connect if we have their pubkey and aren't already connected
                        if let Some(ref pk) = peer.pubkey_hex {
                            let already_connected = {
                                let active = disc_state.active_peers.lock().await;
                                active.contains_key(pk)
                            };
                            if !already_connected {
                                info!(
                                    peer_name = %peer_name,
                                    "Auto-connecting to discovered peer"
                                );
                                let s = disc_state.clone();
                                let t = disc_transport.clone();
                                let p = peer.clone();
                                tokio::spawn(async move {
                                    handle_outgoing(s, t, p).await;
                                });
                            }
                        } else {
                            debug!(
                                peer_id = %peer_id,
                                "Discovered peer has no pubkey — cannot auto-connect"
                            );
                        }
                    }
                    Some(DiscoveryEvent::PeerLost { node_id, .. }) => {
                        warn!(node_id = ?node_id, "Peer lost");
                    }
                    Some(DiscoveryEvent::Error(e)) => {
                        warn!("Discovery error: {e}");
                    }
                    Some(_) => {}
                    None => break,
                }
            }
        });
    }

    // 10b. Start event log watcher (monitors local teamengram for outbound federation events)
    if let Some(ref elp) = event_log_path {
        let watcher_state = state.clone();
        let watcher_path = elp.clone();
        tokio::spawn(async move {
            run_event_log_watcher(watcher_state, watcher_path).await;
        });
        info!("Event log watcher started — local events will be pushed to peers");
    }

    info!(
        name = %name,
        short_id = %short_id,
        peers = peers.peers.len(),
        "Federation node running. Press Ctrl+C to stop."
    );

    // 11. Main loop: accept connections + handle Ctrl+C
    loop {
        tokio::select! {
            // Graceful shutdown
            _ = tokio::signal::ctrl_c() => {
                info!("Shutting down...");
                break;
            }

            // Accept incoming QUIC connections (rate-limited via semaphore)
            incoming = transport.accept() => {
                match incoming {
                    Some(incoming) => {
                        let state = state.clone();
                        // Acquire connection permit — limits concurrent connection handling
                        let permit = match Arc::clone(&state.conn_semaphore).try_acquire_owned() {
                            Ok(permit) => permit,
                            Err(_) => {
                                warn!("Connection limit reached ({}) — rejecting", MAX_CONCURRENT_CONNECTIONS);
                                // Drop the incoming connection (QUIC will send CONNECTION_CLOSE)
                                drop(incoming);
                                continue;
                            }
                        };
                        tokio::spawn(async move {
                            handle_incoming(state, incoming).await;
                            drop(permit); // Release on disconnect
                        });
                    }
                    None => {
                        info!("Transport closed");
                        break;
                    }
                }
            }
        }
    }

    // Shutdown sequence
    info!("Flushing replication cursors...");
    if let Err(e) = state.replication.lock().await.flush() {
        error!("Failed to flush cursors: {e}");
    }

    info!("Shutting down QUIC transport...");
    transport.shutdown().await?;

    info!("Federation node stopped.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Connection handler
// ---------------------------------------------------------------------------

async fn handle_incoming(state: Arc<NodeState>, incoming: iroh::endpoint::Incoming) {
    let conn = match incoming.await {
        Ok(conn) => conn,
        Err(e) => {
            warn!("Failed to accept connection: {e}");
            return;
        }
    };

    let remote_id = conn.remote_id();
    info!(%remote_id, "Incoming connection");

    // Run responder handshake
    let session = match PeerSession::accept(
        conn,
        &state.identity,
        &state.local_node,
        &state.peers,
    )
    .await
    {
        Ok(session) => session,
        Err(e) => {
            warn!(%remote_id, "Handshake failed: {e}");
            return;
        }
    };

    run_peer_session(state, session).await;
}

/// Initiate an outgoing connection to a discovered peer.
///
/// Parses the peer's Ed25519 public key from mDNS TXT records, establishes
/// a QUIC connection via iroh, runs the initiator handshake, then hands off
/// to the shared `run_peer_session` loop.
async fn handle_outgoing(
    state: Arc<NodeState>,
    transport: Arc<QuicTransport>,
    peer: federation::discovery::DiscoveredPeer,
) {
    let pubkey_hex = match peer.pubkey_hex {
        Some(ref pk) => pk.clone(),
        None => {
            warn!("Cannot connect — peer has no pubkey");
            return;
        }
    };
    let peer_name = peer
        .display_name
        .as_deref()
        .unwrap_or("unnamed")
        .to_string();

    // Parse Ed25519 public key → iroh EndpointId
    let pubkey_bytes = match hex::decode(&pubkey_hex) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            arr
        }
        _ => {
            warn!(peer = %peer_name, "Invalid pubkey hex — cannot connect");
            return;
        }
    };
    let endpoint_id = match iroh::PublicKey::from_bytes(&pubkey_bytes) {
        Ok(pk) => pk,
        Err(e) => {
            warn!(peer = %peer_name, "Invalid Ed25519 public key: {e}");
            return;
        }
    };

    info!(peer = %peer_name, "Connecting to discovered peer via QUIC");

    // Connect via QUIC transport
    let conn = match transport.connect(endpoint_id).await {
        Ok(conn) => conn,
        Err(e) => {
            warn!(peer = %peer_name, "QUIC connect failed: {e}");
            return;
        }
    };

    // Run initiator handshake
    let session = match PeerSession::connect(
        conn,
        &state.identity,
        &state.local_node,
        &state.peers,
    )
    .await
    {
        Ok(session) => session,
        Err(e) => {
            warn!(peer = %peer_name, "Outgoing handshake failed: {e}");
            return;
        }
    };

    run_peer_session(state, session).await;
}

/// Shared peer session loop — runs after handshake (both inbound and outbound).
///
/// Registers the peer, starts bidirectional event exchange (inbound + outbound
/// loops via `tokio::select!`), and cleans up on disconnect.
async fn run_peer_session(state: Arc<NodeState>, session: PeerSession) {
    let peer_node = session.remote_node();
    let peer_pubkey_hex = hex::encode(peer_node.pubkey.as_bytes());
    let peer_name = peer_node.display_name.clone();
    let peer_node_id = peer_node.node_id.clone();

    info!(
        peer = %peer_node_id,
        name = %peer_name,
        "Peer connected and authenticated"
    );

    // Clone the QUIC connection for event loops.
    // iroh::Connection is Arc-based internally — clone is cheap.
    // The session is stored in active_peers for future use.
    let inbound_conn = session.connection().clone();
    let outbound_conn = session.connection().clone();

    // Register active peer
    {
        let mut active = state.active_peers.lock().await;
        active.insert(
            peer_pubkey_hex.clone(),
            ActivePeer {
                session,
                peer_name: peer_name.clone(),
                peer_pubkey_hex: peer_pubkey_hex.clone(),
            },
        );
    }

    // Subscribe to outbound event channel for this peer
    let outbound_rx = state.outbound_tx.subscribe();

    // Run both loops concurrently — when EITHER exits (peer disconnect),
    // the other is cancelled via tokio::select!
    info!(peer = %peer_node_id, "Starting bidirectional event exchange");
    tokio::select! {
        _ = run_peer_event_loop(
            &state, inbound_conn, &peer_pubkey_hex, &peer_name,
        ) => {
            debug!(peer = %peer_name, "Inbound loop ended");
        }
        _ = run_peer_outbound_loop(
            &state, outbound_conn, outbound_rx, &peer_pubkey_hex, &peer_name,
        ) => {
            debug!(peer = %peer_name, "Outbound loop ended");
        }
    }

    // Cleanup on disconnect
    {
        let mut active = state.active_peers.lock().await;
        active.remove(&peer_pubkey_hex);
    }
    info!(peer = %peer_node_id, name = %peer_name, "Peer disconnected — cleaned up");
}

// ---------------------------------------------------------------------------
// Inbound event loop — runs per-peer after handshake
// ---------------------------------------------------------------------------

/// Concurrent event loop for a single peer connection.
///
/// Uses `tokio::select!` to multiplex:
/// - Bidirectional streams → routed by stream type byte (push or pull)
/// - Unidirectional streams → presence updates (fire-and-forget)
///
/// **Security hardening:**
/// - Stream semaphore limits concurrent bidi streams (prevents resource exhaustion)
/// - Timeout on stream type byte read (prevents slow-read attacks)
///
/// Exits when the QUIC connection closes (peer disconnect, timeout, or error).
async fn run_peer_event_loop(
    state: &NodeState,
    conn: iroh::endpoint::Connection,
    peer_pubkey_hex: &str,
    peer_name: &str,
) {
    let timeout = tokio::time::Duration::from_secs(STREAM_TIMEOUT_SECS);

    loop {
        tokio::select! {
            // Bidirectional stream — read type byte to route
            result = conn.accept_bi() => {
                match result {
                    Ok((send, mut recv)) => {
                        // Acquire stream permit (limits concurrent processing)
                        let _permit = match state.stream_semaphore.try_acquire() {
                            Ok(permit) => permit,
                            Err(_) => {
                                warn!(peer = peer_name, "Stream limit reached — dropping stream");
                                continue;
                            }
                        };

                        // Read stream type byte with timeout
                        let mut type_buf = [0u8; 1];
                        let type_read = tokio::time::timeout(
                            timeout,
                            recv.read_exact(&mut type_buf),
                        ).await;
                        match type_read {
                            Ok(Ok(())) => {}
                            Ok(Err(e)) => {
                                warn!(peer = peer_name, "Failed to read stream type: {e}");
                                continue;
                            }
                            Err(_) => {
                                warn!(peer = peer_name, "Timeout reading stream type byte");
                                continue;
                            }
                        }

                        match type_buf[0] {
                            STREAM_TYPE_PUSH => {
                                if let Err(e) = handle_event_push_stream(
                                    state, send, recv, peer_pubkey_hex, peer_name,
                                ).await {
                                    warn!(peer = peer_name, "Event push handling error: {e}");
                                }
                            }
                            STREAM_TYPE_PULL => {
                                if let Err(e) = handle_event_pull_stream(
                                    state, send, recv, peer_pubkey_hex, peer_name,
                                ).await {
                                    warn!(peer = peer_name, "Event pull handling error: {e}");
                                }
                            }
                            other => {
                                warn!(peer = peer_name, stream_type = other, "Unknown bidi stream type");
                            }
                        }
                    }
                    Err(e) => {
                        info!(peer = peer_name, "Connection closed (bidi accept): {e}");
                        break;
                    }
                }
            }

            // Unidirectional stream — presence from peer (fire-and-forget)
            result = conn.accept_uni() => {
                match result {
                    Ok(recv) => {
                        if let Err(e) = handle_presence_stream(
                            state, recv, peer_pubkey_hex, peer_name,
                        ).await {
                            warn!(peer = peer_name, "Presence handling error: {e}");
                        }
                    }
                    Err(e) => {
                        info!(peer = peer_name, "Connection closed (uni accept): {e}");
                        break;
                    }
                }
            }
        }
    }
}

/// Process one inbound event push on a bidirectional QUIC stream.
///
/// 1. Read `EventPushRequest` (CBOR, length-prefixed) with timeout
/// 2. Validate batch size
/// 3. Pre-filter duplicates via `ReplicationOrchestrator`
/// 4. Validate + write to inbox.jsonl via `process_push_request`
/// 5. Update replication cursors
/// 6. Send `EventPushResponse` back to peer with timeout
async fn handle_event_push_stream(
    state: &NodeState,
    send: iroh::endpoint::SendStream,
    mut recv: iroh::endpoint::RecvStream,
    peer_pubkey_hex: &str,
    peer_name: &str,
) -> anyhow::Result<()> {
    let timeout = tokio::time::Duration::from_secs(STREAM_TIMEOUT_SECS);

    // 1. Read request with timeout
    let request_bytes = tokio::time::timeout(timeout, recv_message(&mut recv))
        .await
        .map_err(|_| anyhow::anyhow!("Timeout reading event push request"))??;
    let request: EventPushRequest = cbor_decode(&request_bytes)?;

    let event_count = request.events.len();
    let sender_head_seq = request.sender_head_seq;
    let sender_hlc = request.sender_hlc;

    // 2. Reject oversized batches before expensive validation
    if event_count > MAX_EVENTS_PER_PUSH {
        warn!(
            peer = peer_name,
            events = event_count,
            max = MAX_EVENTS_PER_PUSH,
            "Rejecting oversized push batch"
        );
        anyhow::bail!("Push batch exceeds maximum ({event_count} > {MAX_EVENTS_PER_PUSH})");
    }

    debug!(
        peer = peer_name,
        events = event_count,
        sender_head_seq,
        "Received event push"
    );

    // 2. Pre-filter duplicates through replication orchestrator
    let dup_count = {
        let replication = state.replication.lock().await;
        let (_new, dups) = replication.dedup_events(&request.events);
        dups
    };

    if dup_count > 0 {
        debug!(peer = peer_name, duplicates = dup_count, "Pre-filtered known duplicates");
    }

    // 3. Extract content IDs before request is consumed by process_push_request
    let event_content_ids: Vec<String> = request
        .events
        .iter()
        .map(|e| e.content_id.clone())
        .collect();

    // 4. Process through inbox validation pipeline
    //    (signature verify, manifest check, HLC drift, classify, write to JSONL)
    let response = process_push_request(&state.inbox_state, request);

    // 5. Compute accepted content IDs (events NOT in the error list)
    let error_indices: HashSet<usize> = response.errors.iter().map(|e| e.index).collect();
    let accepted_content_ids: Vec<String> = event_content_ids
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !error_indices.contains(i))
        .map(|(_, id)| id)
        .collect();

    // 6. Update replication cursors
    if response.accepted > 0 {
        let mut replication = state.replication.lock().await;
        replication.on_events_received(
            peer_pubkey_hex,
            peer_name,
            sender_head_seq,
            sender_hlc,
            accepted_content_ids,
        );
    }

    info!(
        peer = peer_name,
        accepted = response.accepted,
        rejected = response.rejected,
        duplicates = response.duplicates,
        "Event push processed"
    );

    // 7. Send response with timeout
    let response_bytes = cbor_encode(&response)?;
    tokio::time::timeout(timeout, send_message_finish(send, &response_bytes))
        .await
        .map_err(|_| anyhow::anyhow!("Timeout sending event push response"))??;

    Ok(())
}

/// Process one inbound presence update on a unidirectional QUIC stream.
///
/// Fire-and-forget: reads the `PresencePushRequest`, processes it through
/// the inbox (writes to inbox.jsonl + updates AiRegistry), no response needed.
///
/// **Security:** Validates sender_short_id matches the authenticated peer's
/// identity. Timeout on recv prevents slow-read resource exhaustion.
async fn handle_presence_stream(
    state: &NodeState,
    mut recv: iroh::endpoint::RecvStream,
    peer_pubkey_hex: &str,
    peer_name: &str,
) -> anyhow::Result<()> {
    let timeout = tokio::time::Duration::from_secs(STREAM_TIMEOUT_SECS);

    let data = tokio::time::timeout(timeout, recv_message(&mut recv))
        .await
        .map_err(|_| anyhow::anyhow!("Timeout reading presence update"))??;
    let request: PresencePushRequest = cbor_decode(&data)?;

    debug!(
        peer = peer_name,
        ai_count = request.presences.len(),
        from = %request.sender_short_id,
        "Received presence update"
    );

    // Validate sender identity — short_id should derive from the peer's pubkey.
    // Log warning on mismatch but still process (presence is ephemeral, not worth
    // dropping the connection over — but the log alerts operators to misbehavior).
    let expected_short_id = &peer_pubkey_hex[..8.min(peer_pubkey_hex.len())];
    if !request.sender_short_id.contains(expected_short_id) {
        warn!(
            peer = peer_name,
            claimed = %request.sender_short_id,
            expected_prefix = %expected_short_id,
            peer_pubkey = %peer_pubkey_hex,
            "Presence sender_short_id doesn't match authenticated peer — possible spoofing"
        );
    }

    // Process through inbox EDU path (fire-and-forget)
    process_presence_request(&state.inbox_state, request).await;

    Ok(())
}

/// Serve an inbound event pull request on a bidirectional QUIC stream.
///
/// The peer is catching up after reconnect — they want events since their
/// last known sequence. We serve from our local event log (when integrated)
/// or return an empty response.
///
/// **Security:** Validates that `request.requester_pubkey` matches the
/// authenticated peer identity from the QUIC handshake. Caps pull limit
/// server-side to prevent excessive batch sizes.
async fn handle_event_pull_stream(
    state: &NodeState,
    send: iroh::endpoint::SendStream,
    mut recv: iroh::endpoint::RecvStream,
    peer_pubkey_hex: &str,
    peer_name: &str,
) -> anyhow::Result<()> {
    let timeout = tokio::time::Duration::from_secs(STREAM_TIMEOUT_SECS);

    // 1. Read pull request with timeout
    let request_bytes = tokio::time::timeout(timeout, recv_message(&mut recv))
        .await
        .map_err(|_| anyhow::anyhow!("Timeout reading pull request"))??;
    let mut request: EventPullRequest = cbor_decode(&request_bytes)?;

    // 2. Validate requester identity matches authenticated peer
    if request.requester_pubkey != peer_pubkey_hex {
        warn!(
            peer = peer_name,
            claimed = %request.requester_pubkey,
            actual = %peer_pubkey_hex,
            "Pull request identity mismatch — rejecting"
        );
        anyhow::bail!("Pull request requester_pubkey does not match authenticated peer");
    }

    // 3. Cap pull limit server-side (prevent excessive batch requests)
    if request.limit > MAX_PULL_BATCH_SIZE {
        debug!(
            peer = peer_name,
            requested = request.limit,
            capped = MAX_PULL_BATCH_SIZE,
            "Capping pull request limit"
        );
        request.limit = MAX_PULL_BATCH_SIZE;
    }

    debug!(
        peer = peer_name,
        since_seq = request.since_seq,
        limit = request.limit,
        "Received catchup pull request"
    );

    // 4. Serve the pull via orchestrator — reads from local teamengram event log
    let response = {
        let replication = state.replication.lock().await;
        let sender_hlc = state.gateway.clock.tick();
        replication.serve_pull_request(&request, sender_hlc, |since_seq, limit| {
            read_events_for_pull(&state.event_log_path, since_seq, limit, &state.gateway)
        })
    };

    info!(
        peer = peer_name,
        events = response.events.len(),
        head_seq = response.head_seq,
        has_more = response.has_more,
        "Serving catchup pull response"
    );

    // 5. Send response with timeout
    let response_bytes = cbor_encode(&response)?;
    tokio::time::timeout(timeout, send_message_finish(send, &response_bytes))
        .await
        .map_err(|_| anyhow::anyhow!("Timeout sending pull response"))??;

    Ok(())
}

// ---------------------------------------------------------------------------
// Outbound push loop — runs per-peer after handshake
// ---------------------------------------------------------------------------

/// Outbound event push loop for a single peer.
///
/// Two phases:
/// 1. **Backlog catch-up** — drain buffered events the peer missed (seq > their acked)
/// 2. **Live stream** — push events as they arrive from the broadcast channel
///
/// Exits when the QUIC connection closes, the broadcast channel closes, or
/// a push fails (indicating peer disconnect).
async fn run_peer_outbound_loop(
    state: &NodeState,
    conn: iroh::endpoint::Connection,
    mut outbound_rx: tokio::sync::broadcast::Receiver<OutboundEvent>,
    peer_pubkey_hex: &str,
    peer_name: &str,
) {
    // Phase 0: Catchup PULL — request events we missed from this peer.
    // Errors here don't kill the outbound loop — we can still push even if pull fails.
    match run_catchup_pull(state, &conn, peer_pubkey_hex, peer_name).await {
        Ok(accepted) => {
            if accepted > 0 {
                info!(peer = peer_name, accepted, "Catchup pull complete");
            } else {
                debug!(peer = peer_name, "Catchup pull: no new events from peer");
            }
        }
        Err(e) => {
            warn!(peer = peer_name, "Catchup pull failed (continuing with push): {e}");
        }
    }

    // Phase 1: Drain backlog for events this peer hasn't seen
    let since_seq = {
        let replication = state.replication.lock().await;
        replication.outbound_since_seq(peer_pubkey_hex)
    };

    {
        let backlog = state.outbound_backlog.lock().await;
        let catchup_count = backlog.iter().filter(|e| e.seq > since_seq).count();
        if catchup_count > 0 {
            info!(
                peer = peer_name,
                events = catchup_count,
                since_seq,
                "Starting backlog catch-up"
            );
        }
        for event in backlog.iter() {
            if event.seq > since_seq {
                if let Err(e) = push_single_event(
                    state, &conn, event, peer_pubkey_hex, peer_name,
                ).await {
                    warn!(peer = peer_name, "Backlog push failed: {e}");
                    return;
                }
            }
        }
    }

    debug!(peer = peer_name, "Backlog catch-up complete — switching to live stream");

    // Phase 2: Live stream — push events as they arrive
    loop {
        match outbound_rx.recv().await {
            Ok(event) => {
                if let Err(e) = push_single_event(
                    state, &conn, &event, peer_pubkey_hex, peer_name,
                ).await {
                    warn!(peer = peer_name, "Live push failed: {e}");
                    return;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                warn!(
                    peer = peer_name,
                    skipped = n,
                    "Outbound channel lagged — events dropped"
                );
                // Continue — next recv will get the latest
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                info!(peer = peer_name, "Outbound channel closed — shutting down");
                return;
            }
        }
    }
}

/// Push a single event to a peer over a new QUIC bidi stream.
///
/// Pipeline: consent filter → backoff check → sign → build request →
/// send on bidi stream → read response → update replication cursor.
async fn push_single_event(
    state: &NodeState,
    conn: &iroh::endpoint::Connection,
    event: &OutboundEvent,
    peer_pubkey_hex: &str,
    peer_name: &str,
) -> anyhow::Result<()> {
    // 1. Check consent filter (manifest ceiling + AI consent record)
    if !state.gateway.should_cross_boundary(
        event.event_type,
        &event.ai_id,
        &state.consent_dir,
    ) {
        debug!(
            peer = peer_name,
            event_type = ?event.event_type,
            ai_id = %event.ai_id,
            "Filtered by consent — not pushing"
        );
        return Ok(());
    }

    // 2. Check backoff (exponential: 1s, 2s, 4s, ..., capped at 5min)
    {
        let replication = state.replication.lock().await;
        if let Some(cursor) = replication.cursor(peer_pubkey_hex) {
            if cursor.consecutive_failures > 0 {
                let backoff = cursor.backoff_secs();
                debug!(
                    peer = peer_name,
                    backoff_secs = backoff,
                    failures = cursor.consecutive_failures,
                    "Peer in backoff — delaying push"
                );
                // Release lock before sleeping
                drop(replication);
                tokio::time::sleep(tokio::time::Duration::from_secs(backoff)).await;
            }
        }
    }

    // 3. Sign the event
    let signed = state.gateway.sign_event(event.event_bytes.clone());

    // 4. Build push request with current local head seq
    let local_head_seq = state.outbound_seq.load(Ordering::Relaxed);
    let request = state.gateway.build_event_push(vec![signed], local_head_seq);

    // 5. Open bidi stream, write type byte, send request, read response (with timeout)
    let timeout = tokio::time::Duration::from_secs(STREAM_TIMEOUT_SECS);
    let (mut send, mut recv) = conn.open_bi().await?;

    let send_result = tokio::time::timeout(timeout, async {
        send.write_all(&[STREAM_TYPE_PUSH]).await?;
        let request_bytes = cbor_encode(&request)?;
        send_message(&mut send, &request_bytes).await?;
        send.finish()?;
        Ok::<(), anyhow::Error>(())
    }).await;
    match send_result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(_) => anyhow::bail!("Timeout sending outbound push"),
    }

    let response_bytes = tokio::time::timeout(timeout, recv_message(&mut recv))
        .await
        .map_err(|_| anyhow::anyhow!("Timeout reading push response"))??;
    let response: EventPushResponse = cbor_decode(&response_bytes)?;

    // 6. Update replication cursor based on response
    {
        let mut replication = state.replication.lock().await;
        if response.rejected > 0 {
            replication.on_push_failed(peer_pubkey_hex, peer_name);
        } else {
            replication.on_push_acked(peer_pubkey_hex, peer_name, &response);
        }
    }

    debug!(
        peer = peer_name,
        accepted = response.accepted,
        rejected = response.rejected,
        duplicates = response.duplicates,
        "Outbound push complete"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Catchup pull — request events we missed from a peer on reconnect
// ---------------------------------------------------------------------------

/// Pull events from a peer that we missed while disconnected.
///
/// Loops until no more events available (`has_more == false`) or all events
/// are duplicates. Returns the total number of newly accepted events.
///
/// On error, returns `Err` — caller should log and continue with outbound
/// push (pull failure doesn't block pushing our events to the peer).
async fn run_catchup_pull(
    state: &NodeState,
    conn: &iroh::endpoint::Connection,
    peer_pubkey_hex: &str,
    peer_name: &str,
) -> anyhow::Result<usize> {
    let mut total_accepted = 0usize;
    let timeout = tokio::time::Duration::from_secs(STREAM_TIMEOUT_SECS);

    loop {
        let pull_request = {
            let replication = state.replication.lock().await;
            replication.build_catchup_pull(peer_pubkey_hex, Some(100))
        };

        debug!(
            peer = peer_name,
            since_seq = pull_request.since_seq,
            limit = pull_request.limit,
            "Sending catchup pull request"
        );

        // Open bidi stream with PULL type byte (with timeout on send + recv)
        let (mut send, mut recv) = conn.open_bi().await?;

        let send_result = tokio::time::timeout(timeout, async {
            send.write_all(&[STREAM_TYPE_PULL]).await?;
            let request_bytes = cbor_encode(&pull_request)?;
            send_message(&mut send, &request_bytes).await?;
            send.finish()?;
            Ok::<(), anyhow::Error>(())
        }).await;
        match send_result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => anyhow::bail!("Timeout sending catchup pull request"),
        }

        // Read response with timeout
        let response_bytes = tokio::time::timeout(timeout, recv_message(&mut recv))
            .await
            .map_err(|_| anyhow::anyhow!("Timeout reading catchup pull response"))??;
        let response: EventPullResponse = cbor_decode(&response_bytes)?;

        let has_more = response.has_more;
        let event_count = response.events.len();

        if event_count == 0 {
            debug!(peer = peer_name, "Catchup pull: already up to date");
            break;
        }

        info!(
            peer = peer_name,
            events = event_count,
            head_seq = response.head_seq,
            has_more,
            "Received catchup pull response"
        );

        // Filter through dedup — only process genuinely new events
        let new_events: Vec<SignedEvent> = {
            let replication = state.replication.lock().await;
            replication
                .process_pull_response(&response)
                .into_iter()
                .cloned()
                .collect()
        };

        if new_events.is_empty() {
            debug!(peer = peer_name, "All pulled events were duplicates");
            let mut replication = state.replication.lock().await;
            replication.on_pull_complete(peer_pubkey_hex, peer_name, &response, vec![]);
            if !has_more {
                break;
            }
            continue;
        }

        // Extract content IDs before events are consumed by process_push_request
        let content_ids: Vec<String> = new_events.iter().map(|e| e.content_id.clone()).collect();

        // Process through inbox validation pipeline (sig verify, manifest, HLC drift, write JSONL)
        // Construct a synthetic EventPushRequest — same validation as live pushes.
        let synthetic_push = EventPushRequest {
            events: new_events,
            sender_hlc: response.sender_hlc,
            sender_head_seq: response.head_seq,
        };
        let push_response = process_push_request(&state.inbox_state, synthetic_push);

        // Compute accepted content IDs (events NOT in the error list)
        let error_indices: HashSet<usize> =
            push_response.errors.iter().map(|e| e.index).collect();
        let accepted_content_ids: Vec<String> = content_ids
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !error_indices.contains(i))
            .map(|(_, id)| id)
            .collect();

        total_accepted += push_response.accepted;

        info!(
            peer = peer_name,
            accepted = push_response.accepted,
            rejected = push_response.rejected,
            duplicates = push_response.duplicates,
            "Catchup pull batch processed"
        );

        // Update replication cursor
        {
            let mut replication = state.replication.lock().await;
            replication.on_pull_complete(
                peer_pubkey_hex,
                peer_name,
                &response,
                accepted_content_ids,
            );
        }

        if !has_more {
            break;
        }
    }

    Ok(total_accepted)
}

// ---------------------------------------------------------------------------
// TeamEngram Event Log Integration
// ---------------------------------------------------------------------------

/// Classify a teamengram event type into a federation outbound category.
///
/// Returns `None` for event types that should never cross the federation boundary
/// (file claims, raw tool ops, votes, locks, etc.). Only the categories confirmed
/// in QD's taxonomy are eligible: presence, broadcasts, task completions, dialogue ends.
fn classify_event_for_federation(event_type: u16) -> Option<OutboundEventType> {
    match event_type {
        te_event_type::PRESENCE_UPDATE => Some(OutboundEventType::Presence),
        te_event_type::BROADCAST => Some(OutboundEventType::Broadcast { cross_team: false }),
        te_event_type::TASK_COMPLETE => Some(OutboundEventType::TaskComplete),
        te_event_type::DIALOGUE_END => Some(OutboundEventType::DialogueEnd),
        _ => None,
    }
}

/// Read signed events from the local teamengram event log for serving pull requests.
///
/// Opens the event log, seeks past `since_seq`, reads up to `limit` federation-eligible
/// events, wraps each in a `FederationMessage::EventRelay`, signs it, and returns
/// the batch with a `has_more` flag.
///
/// Returns `(vec![], false)` if the event log is unavailable or the peer is caught up.
fn read_events_for_pull(
    event_log_path: &Option<PathBuf>,
    since_seq: u64,
    limit: usize,
    gateway: &FederationGateway,
) -> (Vec<SignedEvent>, bool) {
    let path = match event_log_path {
        Some(p) => p,
        None => return (vec![], false),
    };

    let mut reader = match EventLogReader::open_at_path(path) {
        Ok(r) => r,
        Err(e) => {
            warn!("Failed to open event log for pull request: {e}");
            return (vec![], false);
        }
    };

    // Seek past the requested sequence.
    // seek_to_sequence(X) positions so next read returns event with seq == X.
    // We want events AFTER since_seq, so seek to since_seq and skip it.
    if since_seq > 0 {
        match reader.seek_to_sequence(since_seq) {
            Ok(()) => {
                // Skip the since_seq event itself — we want events strictly after it
                let _ = reader.try_read_raw();
            }
            Err(_) => {
                // Requested sequence not found — peer is ahead of us or log compacted
                return (vec![], false);
            }
        }
    }

    let mut events = Vec::new();
    let mut scanned = 0usize;

    while scanned < limit {
        match reader.try_read_raw() {
            Ok(Some(raw_bytes)) => {
                scanned += 1;

                if raw_bytes.len() < 64 {
                    continue; // Corrupted event — skip
                }

                // Parse header for metadata (no full deserialization needed)
                let header_bytes: [u8; 64] = raw_bytes[..64].try_into().unwrap();
                let header = EventHeader::from_bytes(&header_bytes);

                // Only relay federation-eligible event types
                if classify_event_for_federation(header.event_type).is_none() {
                    continue;
                }

                // Wrap in FederationMessage with EventRelay payload
                let payload = FederationPayload::EventRelay {
                    raw_event: raw_bytes,
                    event_type: header.event_type,
                    source_ai: header.source_ai_str().to_string(),
                    origin_seq: header.sequence,
                };

                let msg = FederationMessage::new(
                    &gateway.identity.short_id(),
                    payload,
                    gateway.identity.signing_key(),
                );

                let cbor_bytes = match msg.to_bytes() {
                    Ok(b) => b,
                    Err(e) => {
                        warn!("Failed to serialize EventRelay for pull: {e}");
                        continue;
                    }
                };

                let signed = gateway.sign_event(cbor_bytes);
                events.push(signed);
            }
            Ok(None) => break,  // End of log
            Err(e) => {
                warn!("Error reading event log during pull: {e}");
                break;
            }
        }
    }

    let has_more = reader.has_more();
    (events, has_more)
}

/// Background task that watches the local teamengram event log for new events
/// and injects federation-eligible ones into the outbound push pipeline.
///
/// ZERO POLLING. Blocks on FederationWakeReceiver — an OS-native wait primitive
/// (Named Event on Windows, POSIX semaphore on Linux). The sequencer signals
/// this after writing each event to the master log. Wake latency: ~1μs.
///
/// Starts from the current head (no history replay). Only presence, broadcasts,
/// task completions, and dialogue conclusions are forwarded — everything else
/// stays local.
async fn run_event_log_watcher(state: Arc<NodeState>, event_log_path: PathBuf) {
    let mut reader = match EventLogReader::open_at_path(&event_log_path) {
        Ok(r) => r,
        Err(e) => {
            error!("Event log watcher: failed to open {}: {e}", event_log_path.display());
            return;
        }
    };

    // Start from current head — don't replay historical events
    let head = reader.head_sequence();
    if head > 0 {
        match reader.seek_to_sequence(head) {
            Ok(()) => {
                // Consume the head event so we start watching from head+1
                let _ = reader.try_read_raw();
            }
            Err(e) => {
                error!("Event log watcher: failed to seek to head seq {head}: {e}");
                return;
            }
        }
    }

    // Initialize replication orchestrator with current head
    {
        let mut replication = state.replication.lock().await;
        replication.set_local_head_seq(head);
    }

    // Bridge OS-level blocking wake into async: dedicated thread signals a channel.
    // The sequencer calls signal_federation() after every event written to the master log.
    // FederationWakeReceiver::wait() blocks on WaitForSingleObject(INFINITE) / sem_wait —
    // zero CPU, ~1μs wake latency.
    let (wake_tx, mut wake_rx) = tokio::sync::mpsc::channel::<()>(1);

    std::thread::Builder::new()
        .name("federation-wake".into())
        .spawn(move || {
            let receiver = match teamengram::wake::FederationWakeReceiver::new(None) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Federation wake thread: failed to create receiver: {e}");
                    return;
                }
            };
            loop {
                receiver.wait(); // Blocks on OS primitive — zero CPU
                if wake_tx.blocking_send(()).is_err() {
                    break; // Channel closed — federation shutting down
                }
            }
        })
        .expect("failed to spawn federation-wake thread");

    info!(head_seq = head, "Event log watcher initialized — blocking on OS wake event (zero polling)");

    loop {
        // Await signal from the dedicated wake thread. Zero CPU while waiting.
        if wake_rx.recv().await.is_none() {
            info!("Event log watcher: wake channel closed, shutting down");
            break;
        }

        // Refresh mmap to see events written by sequencer
        if let Err(e) = reader.refresh() {
            warn!("Event log watcher: refresh failed: {e}");
            continue;
        }

        // Drain all new events
        while reader.has_more() {
            match reader.try_read_raw() {
                Ok(Some(raw_bytes)) => {
                    if raw_bytes.len() < 64 {
                        continue;
                    }

                    let header_bytes: [u8; 64] = raw_bytes[..64].try_into().unwrap();
                    let header = EventHeader::from_bytes(&header_bytes);

                    // Only federate eligible event types
                    let outbound_type = match classify_event_for_federation(header.event_type) {
                        Some(t) => t,
                        None => continue,
                    };

                    let source_ai = header.source_ai_str().to_string();

                    // Wrap in FederationMessage with EventRelay payload
                    let payload = FederationPayload::EventRelay {
                        raw_event: raw_bytes,
                        event_type: header.event_type,
                        source_ai: source_ai.clone(),
                        origin_seq: header.sequence,
                    };

                    let msg = FederationMessage::new(
                        &state.gateway.identity.short_id(),
                        payload,
                        state.gateway.identity.signing_key(),
                    );

                    let cbor_bytes = match msg.to_bytes() {
                        Ok(b) => b,
                        Err(e) => {
                            warn!("Event log watcher: failed to serialize EventRelay: {e}");
                            continue;
                        }
                    };

                    // Inject into outbound federation pipeline
                    state.inject_event(cbor_bytes, outbound_type, source_ai).await;

                    // Update local head seq for replication orchestrator
                    {
                        let mut replication = state.replication.lock().await;
                        replication.set_local_head_seq(header.sequence);
                    }

                    debug!(
                        seq = header.sequence,
                        event_type = header.event_type,
                        source = header.source_ai_str(),
                        "Event log watcher: injected event into federation pipeline"
                    );
                }
                Ok(None) => break,
                Err(e) => {
                    warn!("Event log watcher: error reading event: {e}");
                    break;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// CBOR helpers
// ---------------------------------------------------------------------------

fn cbor_encode<T: serde::Serialize>(value: &T) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    ciborium::into_writer(value, &mut buf)?;
    Ok(buf)
}

fn cbor_decode<T: serde::de::DeserializeOwned>(data: &[u8]) -> anyhow::Result<T> {
    Ok(ciborium::from_reader(data)?)
}

// ---------------------------------------------------------------------------
// discover — LAN peer scan
// ---------------------------------------------------------------------------

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
    let mut found = 0usize;

    while tokio::time::Instant::now() < deadline {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            Some(event) = discovery.next_event() => {
                if let DiscoveryEvent::PeerFound(peer) = event {
                    found += 1;
                    println!(
                        "  {} — {} ({:?}) signal:{:?}",
                        peer.node_id.as_deref().unwrap_or("unknown"),
                        peer.display_name.as_deref().unwrap_or("unnamed"),
                        peer.discovery_type,
                        peer.signal_strength,
                    );
                }
            }
        }
    }

    discovery.stop().await?;
    println!("\nDiscovered {} peers", found);

    Ok(())
}

// ---------------------------------------------------------------------------
// status — show identity and cursor state
// ---------------------------------------------------------------------------

async fn show_status() -> anyhow::Result<()> {
    let identity = TeambookIdentity::load_or_generate().await?;
    let pubkey_hex = identity.public_key_hex();
    let short_id = identity.short_id();
    let node_id = node_id_from_pubkey(identity.verifying_key());

    println!("=== Federation Node Status ===");
    println!("Node ID:     {}", node_id);
    println!("Short ID:    {}", short_id);
    println!("Public Key:  {}", pubkey_hex);

    // Peer registry
    let peers = PeerRegistryConfig::load_or_default(&PeerRegistryConfig::default_path());
    println!("\nRegistered Peers: {}", peers.peers.len());
    for peer in &peers.peers {
        println!(
            "  {} — {} ({})",
            peer.short_id(),
            peer.name,
            if peer.trusted { "trusted" } else { "untrusted" },
        );
    }

    // Replication cursors
    let orch = ReplicationOrchestrator::new(&pubkey_hex, None)?;
    let statuses = orch.peer_statuses();
    if statuses.is_empty() {
        println!("\nReplication Cursors: none (no sync history)");
    } else {
        println!("\nReplication Cursors:");
        for s in &statuses {
            println!(
                "  {} — in:{} out:{} lag:{} failures:{} dedup:{}",
                s.peer_name,
                s.inbound_head_seq,
                s.outbound_acked_seq,
                s.outbound_lag,
                s.consecutive_failures,
                s.dedup_cache_size,
            );
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Reconstruct a TeambookIdentity from the same key bytes.
///
/// `TeambookIdentity` doesn't implement `Clone` by design — copying secret key
/// material should be explicit. This function creates a second instance from
/// the same 32-byte Ed25519 seed.
fn reconstruct_identity(identity: &TeambookIdentity) -> TeambookIdentity {
    TeambookIdentity::from_secret_bytes(identity.secret_key_bytes())
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "federation-node".to_string())
}
