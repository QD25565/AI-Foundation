//! AFP Server
//!
//! The main server that accepts connections, authenticates AIs,
//! and routes messages. Integrates with existing PostgreSQL/Redis
//! backends for persistent state.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, warn};

use crate::error::{AFPError, Result};
use crate::fingerprint::HardwareFingerprint;
use crate::identity::{AIIdentity, TrustLevel};
use crate::keys::{FallbackStorage, KeyPair, KeyStorage};
use crate::message::{AFPMessage, BanLevel, MessageType, Payload, PresenceInfo};
use crate::transport::{ConnectionState, QuicServer, Transport, TransportServer, WebSocketServer};
use crate::{DEFAULT_QUIC_PORT, DEFAULT_WS_PORT};

/// Connected AI session
struct Session {
    identity: AIIdentity,
    fingerprint: HardwareFingerprint,
    transport: Box<dyn Transport>,
    trust_level: TrustLevel,
}

/// Ban record
#[derive(Clone)]
struct BanRecord {
    level: BanLevel,
    reason: String,
    fingerprint_hash: Option<String>,
    ai_id: Option<String>,
}

/// AFP Server configuration
pub struct ServerConfig {
    pub quic_addr: SocketAddr,
    pub ws_addr: SocketAddr,
    pub teambook_name: String,
    pub teambook_id: String,
    pub postgres_url: Option<String>,
    pub redis_url: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            quic_addr: format!("0.0.0.0:{}", DEFAULT_QUIC_PORT).parse().unwrap(),
            ws_addr: format!("0.0.0.0:{}", DEFAULT_WS_PORT).parse().unwrap(),
            teambook_name: "default".to_string(),
            teambook_id: uuid::Uuid::new_v4().to_string(),
            postgres_url: None,
            redis_url: None,
        }
    }
}

/// AFP Server
pub struct AFPServer {
    config: ServerConfig,
    identity: AIIdentity,
    keypair: KeyPair,
    sessions: Arc<RwLock<HashMap<String, Session>>>,
    bans: Arc<RwLock<Vec<BanRecord>>>,
    running: Arc<RwLock<bool>>,
}

impl AFPServer {
    /// Create a new AFP server
    pub async fn new(config: ServerConfig, ai_id: &str) -> Result<Self> {
        // Initialize key storage and generate/load server identity
        let storage = FallbackStorage::default_chain(ai_id);

        let keypair = if storage.exists(ai_id) {
            storage.load(ai_id)?
        } else {
            let pubkey = storage.generate_and_store(ai_id)?;
            storage.load(ai_id)?
        };

        let identity = AIIdentity::new(
            ai_id.to_string(),
            keypair.public_key(),
            config.teambook_name.clone(),
        )
        .with_trust_level(TrustLevel::Owner);

        Ok(Self {
            config,
            identity,
            keypair,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            bans: Arc::new(RwLock::new(Vec::new())),
            running: Arc::new(RwLock::new(false)),
        })
    }

    /// Start the server (both QUIC and WebSocket)
    pub async fn run(&self) -> Result<()> {
        {
            let mut running = self.running.write().await;
            if *running {
                return Err(AFPError::ServerAlreadyRunning);
            }
            *running = true;
        }

        info!(
            "Starting AFP server for teambook '{}' ({})",
            self.config.teambook_name, self.config.teambook_id
        );
        info!("Server identity: {}", self.identity);

        // Start QUIC server
        let quic_sessions = self.sessions.clone();
        let quic_bans = self.bans.clone();
        let quic_running = self.running.clone();
        let quic_config = self.config.clone();
        let quic_identity = self.identity.clone();
        let quic_keypair = self.keypair.clone();

        let quic_handle = tokio::spawn(async move {
            if let Err(e) = run_quic_server(
                quic_config.quic_addr,
                quic_sessions,
                quic_bans,
                quic_running,
                quic_identity,
                quic_keypair,
                &quic_config.teambook_name,
                &quic_config.teambook_id,
            )
            .await
            {
                error!("QUIC server error: {}", e);
            }
        });

        // Start WebSocket server
        let ws_sessions = self.sessions.clone();
        let ws_bans = self.bans.clone();
        let ws_running = self.running.clone();
        let ws_config = self.config.clone();
        let ws_identity = self.identity.clone();
        let ws_keypair = self.keypair.clone();

        let ws_handle = tokio::spawn(async move {
            if let Err(e) = run_ws_server(
                ws_config.ws_addr,
                ws_sessions,
                ws_bans,
                ws_running,
                ws_identity,
                ws_keypair,
                &ws_config.teambook_name,
                &ws_config.teambook_id,
            )
            .await
            {
                error!("WebSocket server error: {}", e);
            }
        });

        // Wait for both servers
        let _ = tokio::join!(quic_handle, ws_handle);

        Ok(())
    }

    /// Stop the server
    pub async fn shutdown(&self) -> Result<()> {
        let mut running = self.running.write().await;
        *running = false;
        info!("Server shutdown requested");
        Ok(())
    }

    /// Get server info
    pub fn info(&self) -> ServerInfo {
        ServerInfo {
            teambook_name: self.config.teambook_name.clone(),
            teambook_id: self.config.teambook_id.clone(),
            quic_addr: self.config.quic_addr,
            ws_addr: self.config.ws_addr,
            server_ai_id: self.identity.ai_id.clone(),
        }
    }
}

/// Server info for display
pub struct ServerInfo {
    pub teambook_name: String,
    pub teambook_id: String,
    pub quic_addr: SocketAddr,
    pub ws_addr: SocketAddr,
    pub server_ai_id: String,
}

impl Clone for ServerConfig {
    fn clone(&self) -> Self {
        Self {
            quic_addr: self.quic_addr,
            ws_addr: self.ws_addr,
            teambook_name: self.teambook_name.clone(),
            teambook_id: self.teambook_id.clone(),
            postgres_url: self.postgres_url.clone(),
            redis_url: self.redis_url.clone(),
        }
    }
}

// Note: KeyPair impl Clone is in keys.rs

/// Run QUIC server
async fn run_quic_server(
    addr: SocketAddr,
    sessions: Arc<RwLock<HashMap<String, Session>>>,
    bans: Arc<RwLock<Vec<BanRecord>>>,
    running: Arc<RwLock<bool>>,
    server_identity: AIIdentity,
    server_keypair: KeyPair,
    teambook_name: &str,
    teambook_id: &str,
) -> Result<()> {
    let mut server = QuicServer::new();
    server.bind(addr).await?;
    info!("QUIC server listening on {}", addr);

    while *running.read().await {
        // Accept with timeout to check running flag
        let accept_result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            server.accept(),
        )
        .await;

        match accept_result {
            Ok(Ok(mut transport)) => {
                info!(
                    "QUIC connection from {:?}",
                    transport.remote_addr()
                );

                // Handle connection in separate task
                let sessions = sessions.clone();
                let bans = bans.clone();
                let server_identity = server_identity.clone();
                let server_keypair = server_keypair.clone();
                let teambook_name = teambook_name.to_string();
                let teambook_id = teambook_id.to_string();

                tokio::spawn(async move {
                    if let Err(e) = handle_connection(
                        transport,
                        sessions,
                        bans,
                        server_identity,
                        server_keypair,
                        &teambook_name,
                        &teambook_id,
                    )
                    .await
                    {
                        warn!("Connection handler error: {}", e);
                    }
                });
            }
            Ok(Err(e)) => {
                warn!("Accept error: {}", e);
            }
            Err(_) => {
                // Timeout, just continue to check running flag
            }
        }
    }

    server.shutdown().await?;
    info!("QUIC server stopped");
    Ok(())
}

/// Run WebSocket server
async fn run_ws_server(
    addr: SocketAddr,
    sessions: Arc<RwLock<HashMap<String, Session>>>,
    bans: Arc<RwLock<Vec<BanRecord>>>,
    running: Arc<RwLock<bool>>,
    server_identity: AIIdentity,
    server_keypair: KeyPair,
    teambook_name: &str,
    teambook_id: &str,
) -> Result<()> {
    let mut server = WebSocketServer::new();
    server.bind(addr).await?;
    info!("WebSocket server listening on {}", addr);

    while *running.read().await {
        let accept_result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            server.accept(),
        )
        .await;

        match accept_result {
            Ok(Ok(transport)) => {
                info!(
                    "WebSocket connection from {:?}",
                    transport.remote_addr()
                );

                let sessions = sessions.clone();
                let bans = bans.clone();
                let server_identity = server_identity.clone();
                let server_keypair = server_keypair.clone();
                let teambook_name = teambook_name.to_string();
                let teambook_id = teambook_id.to_string();

                tokio::spawn(async move {
                    if let Err(e) = handle_connection(
                        transport,
                        sessions,
                        bans,
                        server_identity,
                        server_keypair,
                        &teambook_name,
                        &teambook_id,
                    )
                    .await
                    {
                        warn!("Connection handler error: {}", e);
                    }
                });
            }
            Ok(Err(e)) => {
                warn!("Accept error: {}", e);
            }
            Err(_) => {
                // Timeout
            }
        }
    }

    server.shutdown().await?;
    info!("WebSocket server stopped");
    Ok(())
}

/// Handle a single connection
async fn handle_connection(
    mut transport: Box<dyn Transport>,
    sessions: Arc<RwLock<HashMap<String, Session>>>,
    bans: Arc<RwLock<Vec<BanRecord>>>,
    server_identity: AIIdentity,
    server_keypair: KeyPair,
    teambook_name: &str,
    teambook_id: &str,
) -> Result<()> {
    // Wait for Hello message
    let hello_msg = transport.recv().await?;

    // Verify signature
    hello_msg.verify()?;

    // Extract Hello payload
    let (fingerprint, capabilities, requested_trust) = match hello_msg.payload {
        Payload::Hello {
            fingerprint,
            capabilities,
            requested_trust,
        } => (fingerprint, capabilities, requested_trust),
        _ => {
            return Err(AFPError::HandshakeFailed(
                "Expected Hello message".to_string(),
            ));
        }
    };

    let client_ai_id = hello_msg.from.ai_id.clone();
    let client_pubkey = hello_msg.from.to_verifying_key()?;

    info!("Hello from {} (fingerprint: {})", client_ai_id, fingerprint.short_hash());

    // Check bans
    {
        let bans = bans.read().await;
        for ban in bans.iter() {
            // Check AI_ID ban
            if let Some(banned_id) = &ban.ai_id {
                if banned_id == &client_ai_id {
                    let mut reject = AFPMessage::new(
                        MessageType::Response,
                        &server_identity,
                        Some(client_ai_id.clone()),
                        Payload::Rejected {
                            reason: ban.reason.clone(),
                            banned: true,
                        },
                    );
                    reject.sign(&server_keypair)?;
                    transport.send(&reject).await?;
                    return Err(AFPError::Banned {
                        reason: ban.reason.clone(),
                    });
                }
            }

            // Check fingerprint ban
            if let Some(banned_hash) = &ban.fingerprint_hash {
                if banned_hash == &fingerprint.hash_hex() {
                    let mut reject = AFPMessage::new(
                        MessageType::Response,
                        &server_identity,
                        Some(client_ai_id.clone()),
                        Payload::Rejected {
                            reason: ban.reason.clone(),
                            banned: true,
                        },
                    );
                    reject.sign(&server_keypair)?;
                    transport.send(&reject).await?;
                    return Err(AFPError::Banned {
                        reason: ban.reason.clone(),
                    });
                }
            }
        }
    }

    // Determine trust level (for now, start at Verified)
    let trust_level = TrustLevel::Verified;

    // Send Welcome
    let mut welcome = AFPMessage::new(
        MessageType::Response,
        &server_identity,
        Some(client_ai_id.clone()),
        Payload::Welcome {
            trust_level,
            teambook_name: teambook_name.to_string(),
            teambook_id: teambook_id.to_string(),
            server_version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );
    welcome.msg_id = hello_msg.msg_id;
    welcome.sign(&server_keypair)?;
    transport.send(&welcome).await?;

    info!("{} authenticated at trust level {:?}", client_ai_id, trust_level);

    // Create identity
    let client_identity = AIIdentity::new(
        client_ai_id.clone(),
        client_pubkey,
        teambook_name.to_string(),
    )
    .with_trust_level(trust_level);

    // Store session
    {
        let mut sessions = sessions.write().await;
        sessions.insert(
            client_ai_id.clone(),
            Session {
                identity: client_identity,
                fingerprint,
                transport,
                trust_level,
            },
        );
    }

    // Main message loop would go here
    // For now, just keep connection alive

    Ok(())
}
