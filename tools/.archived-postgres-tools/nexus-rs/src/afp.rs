//! AFP Transport Handler for Nexus
//!
//! Handles incoming AFP connections and routes Nexus-specific payloads
//! (spaces, encounters, tools, friendships, activity) to the Nexus database.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{error, info, warn};

use afp::{
    AFPMessage, MessageType, Payload, AFPError, Result as AfpResult,
    identity::{AIIdentity, TrustLevel},
    keys::{FallbackStorage, KeyPair, KeyStorage},
    fingerprint::HardwareFingerprint,
    transport::{QuicServer, TransportServer, Transport},
    message::{SpaceInfo, EncounterInfo, ToolInfo, FriendInfo, FriendRequestInfo, ActivityInfo},
};

use crate::db::NexusDb;
use crate::{ToolFilter, ToolRating, Friendship, ActivityFilter};

/// AFP session for Nexus
#[allow(dead_code)]
struct NexusAfpSession {
    ai_id: String,
    fingerprint: HardwareFingerprint,
    trust_level: TrustLevel,
}

/// AFP handler configuration
pub struct AfpHandlerConfig {
    pub quic_port: u16,
    pub ai_id: String,
}

impl Default for AfpHandlerConfig {
    fn default() -> Self {
        Self {
            quic_port: 31421, // Different from AFP server's 31415
            ai_id: "nexus-server".to_string(),
        }
    }
}

/// AFP Handler for Nexus server
pub struct AfpHandler {
    config: AfpHandlerConfig,
    keypair: KeyPair,
    identity: AIIdentity,
    db: Arc<NexusDb>,
    #[allow(dead_code)]
    sessions: RwLock<HashMap<String, NexusAfpSession>>,
}

impl AfpHandler {
    /// Create a new AFP handler
    pub async fn new(config: AfpHandlerConfig, db: Arc<NexusDb>) -> AfpResult<Self> {
        let storage = FallbackStorage::default_chain(&config.ai_id);

        let keypair = if storage.exists(&config.ai_id) {
            storage.load(&config.ai_id)?
        } else {
            let _ = storage.generate_and_store(&config.ai_id)?;
            storage.load(&config.ai_id)?
        };

        let identity = AIIdentity::new(
            config.ai_id.clone(),
            keypair.public_key(),
            "nexus".to_string(),
        ).with_trust_level(TrustLevel::Owner);

        Ok(Self {
            config,
            keypair,
            identity,
            db,
            sessions: RwLock::new(HashMap::new()),
        })
    }

    /// Start the AFP listener
    pub async fn run(&self) -> AfpResult<()> {
        let addr: SocketAddr = format!("0.0.0.0:{}", self.config.quic_port)
            .parse()
            .map_err(|e| AFPError::ConnectionFailed(format!("Invalid address: {}", e)))?;

        let mut server = QuicServer::new();
        server.bind(addr).await?;
        info!("Nexus AFP listener on QUIC port {}", self.config.quic_port);

        loop {
            match server.accept().await {
                Ok(transport) => {
                    info!("AFP connection from {:?}", transport.remote_addr());
                    let db = self.db.clone();
                    let identity = self.identity.clone();
                    let keypair = self.keypair.clone();

                    tokio::spawn(async move {
                        if let Err(e) = handle_afp_connection(transport, db, identity, keypair).await {
                            warn!("AFP connection error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("AFP accept error: {}", e);
                }
            }
        }
    }
}

/// Handle an AFP connection
async fn handle_afp_connection(
    mut transport: Box<dyn Transport>,
    db: Arc<NexusDb>,
    server_identity: AIIdentity,
    server_keypair: KeyPair,
) -> AfpResult<()> {
    // Wait for Hello
    let hello = transport.recv().await?;
    hello.verify()?;

    let _fingerprint = match &hello.payload {
        Payload::Hello { fingerprint, .. } => fingerprint.clone(),
        _ => return Err(AFPError::HandshakeFailed("Expected Hello".into())),
    };

    let client_ai_id = hello.from.ai_id.clone();
    info!("Nexus AFP hello from {}", client_ai_id);

    // Send Welcome
    let mut welcome = AFPMessage::new(
        MessageType::Response,
        &server_identity,
        Some(client_ai_id.clone()),
        Payload::Welcome {
            trust_level: TrustLevel::Verified,
            teambook_name: "nexus".to_string(),
            teambook_id: "nexus-cyberspace".to_string(),
            server_version: env!("CARGO_PKG_VERSION").to_string(),
        },
    );
    welcome.msg_id = hello.msg_id;
    welcome.sign(&server_keypair)?;
    transport.send(&welcome).await?;

    // Message loop
    loop {
        let msg = match transport.recv().await {
            Ok(m) => m,
            Err(AFPError::ConnectionClosed) => {
                info!("{} disconnected from Nexus AFP", client_ai_id);
                break;
            }
            Err(e) => {
                warn!("Recv error from {}: {}", client_ai_id, e);
                break;
            }
        };

        if let Err(e) = msg.verify() {
            warn!("Invalid signature from {}: {}", client_ai_id, e);
            continue;
        }

        // Handle Nexus payloads
        let response_payload = handle_nexus_payload(&msg.payload, &client_ai_id, &db).await;

        let mut response = AFPMessage::new(
            MessageType::Response,
            &server_identity,
            Some(client_ai_id.clone()),
            response_payload,
        );
        response.msg_id = msg.msg_id;
        response.sign(&server_keypair)?;
        transport.send(&response).await?;
    }

    Ok(())
}

/// Handle a Nexus-specific payload and return the response
async fn handle_nexus_payload(payload: &Payload, ai_id: &str, db: &Arc<NexusDb>) -> Payload {
    match payload {
        // === SPACES ===
        Payload::SpacesList {} => {
            match db.get_spaces().await {
                Ok(spaces) => {
                    let infos: Vec<SpaceInfo> = spaces.iter().map(|s| SpaceInfo {
                        id: s.id.clone(),
                        name: s.name.clone(),
                        description: s.description.clone(),
                        space_type: format!("{:?}", s.space_type),
                        population: 0,
                    }).collect();
                    Payload::SpacesListResponse { spaces: infos }
                }
                Err(e) => error_payload(&e.to_string()),
            }
        }

        Payload::SpaceEnter { space_id } => {
            match db.enter_space(ai_id, space_id).await {
                Ok(_presence) => {
                    let pop = db.get_space_population(space_id).await.map(|p| p.total).unwrap_or(0) as u32;
                    Payload::SpaceEntered {
                        space_id: space_id.clone(),
                        population: pop,
                    }
                }
                Err(e) => error_payload(&e.to_string()),
            }
        }

        Payload::SpaceLeave {} => {
            // Need to track which space the AI is in - for now return generic response
            Payload::SpaceLeft { space_id: "unknown".to_string() }
        }

        Payload::SpacePopulation { space_id } => {
            match db.get_space_population(space_id).await {
                Ok(pop) => Payload::SpacePopulationResponse {
                    space_id: space_id.clone(),
                    count: pop.total as u32,
                    ais: pop.visible_ais.iter().map(|p| p.ai_id.clone()).collect(),
                },
                Err(e) => error_payload(&e.to_string()),
            }
        }

        // === ENCOUNTERS ===
        Payload::EncounterQuery { limit, since: _ } => {
            let lim = limit.unwrap_or(50) as usize;
            match db.get_encounters(ai_id, lim).await {
                Ok(encounters) => {
                    let infos: Vec<EncounterInfo> = encounters.iter().map(|e| {
                        let other = if e.ai_id_1 == ai_id { &e.ai_id_2 } else { &e.ai_id_1 };
                        EncounterInfo {
                            id: e.id.as_u128() as u64,
                            other_ai: other.clone(),
                            space_id: e.space_id.clone(),
                            encounter_type: format!("{:?}", e.encounter_type),
                            timestamp: e.occurred_at.timestamp() as u64,
                            message: None,
                        }
                    }).collect();
                    Payload::EncounterResponse { encounters: infos }
                }
                Err(e) => error_payload(&e.to_string()),
            }
        }

        // === TOOLS ===
        Payload::ToolSearch { query, category: _, min_rating, verified_only, limit } => {
            let filter = ToolFilter {
                query: query.clone(),
                category: None,
                min_rating: *min_rating,
                verified_only: *verified_only,
                limit: limit.map(|l| l as usize),
                ..Default::default()
            };

            match db.search_tools(&filter).await {
                Ok(tools) => {
                    let infos: Vec<ToolInfo> = tools.iter().map(|t| ToolInfo {
                        id: t.id.to_string(),
                        name: t.name.clone(),
                        display_name: t.display_name.clone(),
                        description: t.description.clone(),
                        category: format!("{:?}", t.category),
                        tags: t.tags.clone(),
                        version: t.version.clone(),
                        author: t.author.clone(),
                        source_url: t.source_url.clone(),
                        average_rating: t.average_rating,
                        rating_count: t.rating_count as u32,
                        install_count: t.install_count as u32,
                        verified: t.verified,
                    }).collect();
                    Payload::ToolSearchResponse { tools: infos }
                }
                Err(e) => error_payload(&e.to_string()),
            }
        }

        Payload::ToolGet { tool_id: _ } => {
            error_payload("Tool get by ID not implemented yet")
        }

        Payload::ToolRate { tool_id, rating, review } => {
            match tool_id.parse::<uuid::Uuid>() {
                Ok(uuid) => {
                    match ToolRating::new(uuid, ai_id, *rating as i32) {
                        Ok(mut r) => {
                            if let Some(rev) = review {
                                r = r.with_review(rev.clone());
                            }
                            match db.rate_tool(&r).await {
                                Ok(()) => Payload::ToolRated { tool_id: tool_id.clone(), new_average: 0.0 },
                                Err(e) => error_payload(&e.to_string()),
                            }
                        }
                        Err(e) => error_payload(&e),
                    }
                }
                Err(_) => error_payload("Invalid tool ID"),
            }
        }

        // === FRIENDSHIPS ===
        Payload::FriendRequest { target_ai, message } => {
            let mut friendship = Friendship::request(ai_id, target_ai);
            if let Some(msg) = message {
                friendship = friendship.with_note(msg.clone());
            }
            match db.send_friend_request(&friendship).await {
                Ok(()) => Payload::FriendRequestSent {
                    request_id: friendship.id.as_u128() as u64,
                    target_ai: target_ai.clone(),
                },
                Err(e) => error_payload(&e.to_string()),
            }
        }

        Payload::FriendAccept { request_id } => {
            // Convert u64 back to UUID (this is lossy but works for now)
            let uuid = uuid::Uuid::from_u128(*request_id as u128);
            match db.respond_to_friend_request(uuid, true).await {
                Ok(()) => Payload::FriendAccepted { friend_ai: "accepted".to_string() },
                Err(e) => error_payload(&e.to_string()),
            }
        }

        Payload::FriendReject { request_id } => {
            let uuid = uuid::Uuid::from_u128(*request_id as u128);
            match db.respond_to_friend_request(uuid, false).await {
                Ok(()) => Payload::FriendRejected { request_id: *request_id },
                Err(e) => error_payload(&e.to_string()),
            }
        }

        Payload::FriendsList { include_pending: _ } => {
            match db.get_friends(ai_id).await {
                Ok(friends) => {
                    let infos: Vec<FriendInfo> = friends.iter().map(|f| {
                        let other = if f.requester_id == ai_id { &f.addressee_id } else { &f.requester_id };
                        FriendInfo {
                            ai_id: other.clone(),
                            instance_id: String::new(),
                            nickname: None,
                            status: format!("{:?}", f.status),
                            since: f.requested_at.timestamp() as u64,
                        }
                    }).collect();
                    Payload::FriendsListResponse {
                        friends: infos,
                        pending_sent: vec![],
                        pending_received: vec![],
                    }
                }
                Err(e) => error_payload(&e.to_string()),
            }
        }

        // === ACTIVITY ===
        Payload::ActivityQuery { space_id, activity_type: _, limit, since: _ } => {
            let filter = ActivityFilter {
                ai_id: None,
                space_id: space_id.clone(),
                limit: limit.map(|l| l as usize),
                ..Default::default()
            };

            match db.get_activity_feed(&filter).await {
                Ok(activities) => {
                    let infos: Vec<ActivityInfo> = activities.iter().map(|a| ActivityInfo {
                        id: a.id.as_u128() as u64,
                        ai_id: a.ai_id.clone(),
                        activity_type: format!("{:?}", a.activity_type),
                        space_id: a.space_id.clone(),
                        content: a.description.clone().unwrap_or_default(),
                        timestamp: a.occurred_at.timestamp() as u64,
                    }).collect();
                    Payload::ActivityResponse { activities: infos }
                }
                Err(e) => error_payload(&e.to_string()),
            }
        }

        // Unknown/unsupported payload
        _ => error_payload("Unsupported Nexus payload type"),
    }
}

/// Create an error payload
fn error_payload(msg: &str) -> Payload {
    Payload::Error {
        code: 500,
        message: msg.to_string(),
    }
}
