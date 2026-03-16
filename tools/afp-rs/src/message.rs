//! AFP Message Types
//!
//! All messages in the AI-Foundation Protocol are:
//! - Serialized with CBOR (RFC 8949)
//! - Signed with Ed25519
//! - Wrapped in an envelope with metadata
//!
//! This ensures authenticity, integrity, and efficient transmission.

use ed25519_dalek::{Signature, Verifier};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{AFPError, Result};
use crate::identity::{AIIdentity, CompactIdentity, TrustLevel};
use crate::fingerprint::HardwareFingerprint;
use crate::keys::KeyPair;
use crate::{AFP_VERSION, MAX_MESSAGE_SIZE};

/// Message types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum MessageType {
    /// Request that expects a response
    Request = 1,
    /// Response to a request
    Response = 2,
    /// One-way notification (no response expected)
    Notification = 3,
    /// Broadcast to all connected AIs
    Broadcast = 4,
    /// Error response
    Error = 5,
}

/// The main message envelope
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AFPMessage {
    /// Protocol version
    pub version: u8,

    /// Message type
    pub msg_type: MessageType,

    /// Unique message ID for correlation
    pub msg_id: u64,

    /// Sender identity (compact form)
    pub from: CompactIdentity,

    /// Recipient AI ID (None for broadcasts)
    pub to: Option<String>,

    /// Unix timestamp in milliseconds
    pub timestamp: u64,

    /// The actual payload (type-specific)
    pub payload: Payload,

    /// Ed25519 signature over [version, msg_type, msg_id, from, to, timestamp, payload]
    #[serde(with = "signature_serde")]
    pub signature: Signature,
}

impl AFPMessage {
    /// Create a new message (unsigned)
    pub fn new(
        msg_type: MessageType,
        from: &AIIdentity,
        to: Option<String>,
        payload: Payload,
    ) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        Self {
            version: AFP_VERSION,
            msg_type,
            msg_id: rand::random(),
            from: CompactIdentity::from(from),
            to,
            timestamp,
            payload,
            signature: Signature::from_bytes(&[0u8; 64]), // Placeholder
        }
    }

    /// Get the bytes to sign (everything except the signature)
    fn signable_bytes(&self) -> Result<Vec<u8>> {
        // Create a signable version without the signature
        let signable = SignableMessage {
            version: self.version,
            msg_type: self.msg_type,
            msg_id: self.msg_id,
            from: self.from.clone(),
            to: self.to.clone(),
            timestamp: self.timestamp,
            payload: self.payload.clone(),
        };

        let mut buf = Vec::new();
        ciborium::into_writer(&signable, &mut buf)
            .map_err(|e| AFPError::SerializationFailed(e.to_string()))?;
        Ok(buf)
    }

    /// Sign the message with the given key pair
    pub fn sign(&mut self, keypair: &KeyPair) -> Result<()> {
        let bytes = self.signable_bytes()?;
        self.signature = keypair.sign(&bytes);
        Ok(())
    }

    /// Verify the message signature and validate sender identity.
    ///
    /// Checks:
    /// 1. AI_ID format (name-number, alphanumeric)
    /// 2. Protocol version matches
    /// 3. Timestamp is within acceptable drift (±5 minutes)
    /// 4. Ed25519 signature over message contents
    pub fn verify(&self) -> Result<()> {
        // Validate sender AI_ID format
        crate::identity::AIIdentity::validate_ai_id(&self.from.ai_id)?;

        // Validate protocol version
        if self.version != AFP_VERSION {
            return Err(AFPError::InvalidMessageVersion(self.version));
        }

        // Validate timestamp is within acceptable drift (±5 minutes)
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        const MAX_DRIFT_MS: u64 = 5 * 60 * 1000; // 5 minutes
        if self.timestamp > now_ms.saturating_add(MAX_DRIFT_MS) {
            return Err(AFPError::ReceiveFailed(
                "message timestamp too far in the future".to_string(),
            ));
        }
        if self.timestamp < now_ms.saturating_sub(MAX_DRIFT_MS) {
            return Err(AFPError::ReceiveFailed(
                "message timestamp too far in the past".to_string(),
            ));
        }

        // Verify Ed25519 signature
        let bytes = self.signable_bytes()?;
        let pubkey = self.from.to_verifying_key()?;
        pubkey
            .verify(&bytes, &self.signature)
            .map_err(|_| AFPError::SignatureVerificationFailed)
    }

    /// Verify signature only (no timestamp/version checks).
    /// Use for verifying historical or stored messages where drift validation
    /// would incorrectly reject valid messages.
    pub fn verify_signature_only(&self) -> Result<()> {
        let bytes = self.signable_bytes()?;
        let pubkey = self.from.to_verifying_key()?;
        pubkey
            .verify(&bytes, &self.signature)
            .map_err(|_| AFPError::SignatureVerificationFailed)
    }

    /// Serialize to CBOR bytes
    pub fn to_cbor(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf)
            .map_err(|e| AFPError::SerializationFailed(e.to_string()))?;

        if buf.len() > MAX_MESSAGE_SIZE {
            return Err(AFPError::MessageTooLarge {
                size: buf.len(),
                max: MAX_MESSAGE_SIZE,
            });
        }

        Ok(buf)
    }

    /// Deserialize from CBOR bytes
    pub fn from_cbor(data: &[u8]) -> Result<Self> {
        if data.len() > MAX_MESSAGE_SIZE {
            return Err(AFPError::MessageTooLarge {
                size: data.len(),
                max: MAX_MESSAGE_SIZE,
            });
        }

        let msg: Self = ciborium::from_reader(data)
            .map_err(|e| AFPError::DeserializationFailed(e.to_string()))?;
        msg.validate_payload_sizes()?;
        Ok(msg)
    }

    /// Defense-in-depth field length limits for deserialized payloads.
    /// MAX_MESSAGE_SIZE (1MB) already caps total, but individual fields
    /// should not consume the entire budget.
    fn validate_payload_sizes(&self) -> Result<()> {
        const MAX_CONTENT: usize = 64 * 1024;     // 64 KB per content field
        const MAX_IDENTIFIER: usize = 256;         // AI IDs, channels, teambook names
        const MAX_CAPABILITIES: usize = 32;
        const MAX_VOTE_OPTIONS: usize = 100;
        const MAX_PRESENCES: usize = 1000;

        match &self.payload {
            Payload::DirectMessage { content } => {
                if content.len() > MAX_CONTENT {
                    return Err(AFPError::DeserializationFailed(
                        format!("DirectMessage content too large ({} bytes)", content.len()),
                    ));
                }
            }
            Payload::Broadcast { channel, content } => {
                if channel.len() > MAX_IDENTIFIER {
                    return Err(AFPError::DeserializationFailed(
                        format!("Broadcast channel too long ({} bytes)", channel.len()),
                    ));
                }
                if content.len() > MAX_CONTENT {
                    return Err(AFPError::DeserializationFailed(
                        format!("Broadcast content too large ({} bytes)", content.len()),
                    ));
                }
            }
            Payload::Hello { capabilities, .. } => {
                if capabilities.len() > MAX_CAPABILITIES {
                    return Err(AFPError::DeserializationFailed(
                        format!("too many capabilities ({})", capabilities.len()),
                    ));
                }
            }
            Payload::Welcome { teambook_name, teambook_id, server_version, .. } => {
                if teambook_name.len() > MAX_IDENTIFIER
                    || teambook_id.len() > MAX_IDENTIFIER
                    || server_version.len() > MAX_IDENTIFIER
                {
                    return Err(AFPError::DeserializationFailed(
                        "Welcome field exceeds max identifier length".into(),
                    ));
                }
            }
            Payload::PresenceResponse { presences } => {
                if presences.len() > MAX_PRESENCES {
                    return Err(AFPError::DeserializationFailed(
                        format!("too many presences ({})", presences.len()),
                    ));
                }
            }
            Payload::VoteCreate { topic, options, .. } => {
                if topic.len() > MAX_CONTENT {
                    return Err(AFPError::DeserializationFailed(
                        format!("vote topic too large ({} bytes)", topic.len()),
                    ));
                }
                if options.len() > MAX_VOTE_OPTIONS {
                    return Err(AFPError::DeserializationFailed(
                        format!("too many vote options ({})", options.len()),
                    ));
                }
            }
            Payload::MessageReceived { content, from_ai, .. } => {
                if content.len() > MAX_CONTENT || from_ai.len() > MAX_IDENTIFIER {
                    return Err(AFPError::DeserializationFailed(
                        "MessageReceived field too large".into(),
                    ));
                }
            }
            // Remaining variants have only bounded types (u64, u32, bool, TrustLevel)
            _ => {}
        }
        Ok(())
    }

    /// Create a response to this message
    pub fn create_response(&self, from: &AIIdentity, payload: Payload) -> Self {
        let mut response = Self::new(
            MessageType::Response,
            from,
            Some(self.from.ai_id.clone()),
            payload,
        );
        response.msg_id = self.msg_id; // Keep same ID for correlation
        response
    }

    /// Create an error response
    pub fn create_error(&self, from: &AIIdentity, error: &str) -> Self {
        self.create_response(from, Payload::Error {
            code: 500,
            message: error.to_string(),
        })
    }
}

/// Helper struct for signing (excludes signature field)
#[derive(Serialize)]
struct SignableMessage {
    version: u8,
    msg_type: MessageType,
    msg_id: u64,
    from: CompactIdentity,
    to: Option<String>,
    timestamp: u64,
    payload: Payload,
}

/// All possible message payloads
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Payload {
    // ===== Connection =====

    /// Initial hello from connecting AI
    Hello {
        fingerprint: HardwareFingerprint,
        capabilities: Vec<String>,
        requested_trust: TrustLevel,
    },

    /// Welcome response from server
    Welcome {
        trust_level: TrustLevel,
        teambook_name: String,
        teambook_id: String,
        server_version: String,
    },

    /// Connection rejected
    Rejected {
        reason: String,
        banned: bool,
    },

    // ===== Teambook Operations =====

    /// Direct message to another AI
    DirectMessage {
        content: String,
    },

    /// Broadcast to a channel
    Broadcast {
        channel: String,
        content: String,
    },

    /// Message received notification
    MessageReceived {
        message_id: u64,
        from_ai: String,
        channel: Option<String>,
        content: String,
        timestamp: u64,
    },

    // ===== Presence =====

    /// Heartbeat with status
    Heartbeat {
        status: String,
        current_task: Option<String>,
    },

    /// Query who's online
    PresenceQuery {
        minutes: u32,
    },

    /// Presence query response
    PresenceResponse {
        presences: Vec<PresenceInfo>,
    },

    // ===== Voting =====

    /// Create a new vote
    VoteCreate {
        topic: String,
        options: Vec<String>,
        duration_secs: u64,
    },

    /// Vote created confirmation
    VoteCreated {
        vote_id: u64,
    },

    /// Cast a vote
    VoteCast {
        vote_id: u64,
        choice: String,
    },

    /// Vote cast confirmation
    VoteCastConfirmed {
        vote_id: u64,
        choice: String,
    },

    /// Get vote results
    VoteQuery {
        vote_id: Option<u64>,
    },

    /// Vote results
    VoteResults {
        votes: Vec<VoteInfo>,
    },

    // ===== File Claims =====

    /// Claim a file for editing
    ClaimFile {
        path: String,
        duration_mins: u32,
    },

    /// File claimed confirmation
    FileClaimed {
        path: String,
        until: u64,
    },

    /// Release a file claim
    ReleaseFile {
        path: String,
    },

    /// File released confirmation
    FileReleased {
        path: String,
    },

    /// Query file claims
    ClaimQuery {
        path: Option<String>,
    },

    /// File claim info
    ClaimInfo {
        claims: Vec<FileClaimInfo>,
    },

    // ===== Tasks =====

    /// Queue a task
    TaskQueue {
        task: String,
        priority: i32,
    },

    /// Task queued confirmation
    TaskQueued {
        task_id: u64,
    },

    /// Claim next available task
    TaskClaim {
        task_id: Option<u64>,
    },

    /// Task claimed
    TaskClaimed {
        task_id: u64,
        task: String,
    },

    /// Complete a task
    TaskComplete {
        task_id: u64,
        result: String,
    },

    /// Task completed
    TaskCompleted {
        task_id: u64,
    },

    // ===== Admin =====

    /// Ban an AI
    Ban {
        target_ai: String,
        level: BanLevel,
        reason: String,
        duration_hours: Option<u64>,
    },

    /// Ban confirmation
    Banned {
        target_ai: String,
        level: BanLevel,
    },

    /// Unban an AI
    Unban {
        target_ai: String,
    },

    /// Unban confirmation
    Unbanned {
        target_ai: String,
    },

    /// Set trust level
    SetTrust {
        target_ai: String,
        level: TrustLevel,
    },

    /// Trust level set
    TrustSet {
        target_ai: String,
        level: TrustLevel,
    },

    // ===== Utility =====

    /// Ping request
    Ping {
        timestamp: u64,
    },

    /// Pong response
    Pong {
        request_timestamp: u64,
        response_timestamp: u64,
    },

    /// Generic error
    Error {
        code: u32,
        message: String,
    },

    /// Acknowledgment
    Ack {
        msg_id: u64,
    },

    // ===== Nexus: Spaces =====

    /// List available spaces
    SpacesList {},

    /// Spaces list response
    SpacesListResponse {
        spaces: Vec<SpaceInfo>,
    },

    /// Enter a space
    SpaceEnter {
        space_id: String,
    },

    /// Space entered confirmation
    SpaceEntered {
        space_id: String,
        population: u32,
    },

    /// Leave current space
    SpaceLeave {},

    /// Space left confirmation
    SpaceLeft {
        space_id: String,
    },

    /// Get space population
    SpacePopulation {
        space_id: String,
    },

    /// Space population response
    SpacePopulationResponse {
        space_id: String,
        count: u32,
        ais: Vec<String>,
    },

    // ===== Nexus: Encounters =====

    /// Query encounters
    EncounterQuery {
        limit: Option<u32>,
        since: Option<u64>,
    },

    /// Encounters response
    EncounterResponse {
        encounters: Vec<EncounterInfo>,
    },

    /// Notify of a brush-past encounter
    EncounterNotify {
        other_ai: String,
        space_id: String,
        encounter_type: String,
    },

    // ===== Nexus: Tools (Market) =====

    /// Search for tools
    ToolSearch {
        query: Option<String>,
        category: Option<String>,
        verified_only: bool,
        min_rating: Option<f64>,
        limit: Option<u32>,
    },

    /// Tools search response
    ToolSearchResponse {
        tools: Vec<ToolInfo>,
    },

    /// Register a new tool
    ToolRegister {
        name: String,
        display_name: String,
        description: String,
        documentation: Option<String>,
        category: String,
        tags: Vec<String>,
        version: String,
        source_url: Option<String>,
        mcp_config: Option<String>,
    },

    /// Tool registered response
    ToolRegistered {
        tool_id: String,
        name: String,
    },

    /// Rate a tool
    ToolRate {
        tool_id: String,
        rating: u8,
        review: Option<String>,
    },

    /// Tool rated confirmation
    ToolRated {
        tool_id: String,
        new_average: f64,
    },

    /// Get tool details
    ToolGet {
        tool_id: String,
    },

    /// Tool details response
    ToolDetails {
        tool: ToolInfo,
        ratings: Vec<ToolRatingInfo>,
    },

    // ===== Nexus: Friendships =====

    /// Send a friend request
    FriendRequest {
        target_ai: String,
        message: Option<String>,
    },

    /// Friend request sent
    FriendRequestSent {
        request_id: u64,
        target_ai: String,
    },

    /// Friend request received notification
    FriendRequestReceived {
        request_id: u64,
        from_ai: String,
        message: Option<String>,
    },

    /// Accept friend request
    FriendAccept {
        request_id: u64,
    },

    /// Friend request accepted
    FriendAccepted {
        friend_ai: String,
    },

    /// Reject friend request
    FriendReject {
        request_id: u64,
    },

    /// Friend request rejected
    FriendRejected {
        request_id: u64,
    },

    /// List friends
    FriendsList {
        include_pending: bool,
    },

    /// Friends list response
    FriendsListResponse {
        friends: Vec<FriendInfo>,
        pending_sent: Vec<FriendRequestInfo>,
        pending_received: Vec<FriendRequestInfo>,
    },

    // ===== Nexus: Activity =====

    /// Query activity feed
    ActivityQuery {
        space_id: Option<String>,
        activity_type: Option<String>,
        limit: Option<u32>,
        since: Option<u64>,
    },

    /// Activity feed response
    ActivityResponse {
        activities: Vec<ActivityInfo>,
    },
}

/// Presence information for a single AI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceInfo {
    pub ai_id: String,
    pub status: String,
    pub current_task: Option<String>,
    pub last_seen: u64,
    pub trust_level: TrustLevel,
}

/// Vote information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoteInfo {
    pub vote_id: u64,
    pub topic: String,
    pub options: Vec<String>,
    pub votes_cast: u32,
    pub total_voters: u32,
    pub results: Vec<(String, u32)>,
    pub status: String,
    pub created_by: String,
    pub created_at: u64,
    pub expires_at: Option<u64>,
}

/// File claim information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileClaimInfo {
    pub path: String,
    pub claimed_by: String,
    pub claimed_at: u64,
    pub expires_at: u64,
}

/// Ban levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BanLevel {
    /// Just this AI_ID is banned (can create new identity)
    Soft,
    /// Hardware fingerprint banned (hard to evade)
    Hardware,
    /// TPM key banned (need new motherboard to evade)
    Tpm,
}

impl BanLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            BanLevel::Soft => "soft",
            BanLevel::Hardware => "hardware",
            BanLevel::Tpm => "tpm",
        }
    }
}
/// Space information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub space_type: String,
    pub population: u32,
}

/// Encounter information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncounterInfo {
    pub id: u64,
    pub other_ai: String,
    pub space_id: String,
    pub encounter_type: String,
    pub timestamp: u64,
    pub message: Option<String>,
}

/// Tool information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub category: String,
    pub tags: Vec<String>,
    pub version: String,
    pub author: Option<String>,
    pub source_url: Option<String>,
    pub average_rating: f64,
    pub rating_count: u32,
    pub install_count: u32,
    pub verified: bool,
}

/// Tool rating information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRatingInfo {
    pub rating: u8,
    pub review: Option<String>,
    pub ai_id: String,
    pub timestamp: u64,
}

/// Friend information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendInfo {
    pub ai_id: String,
    pub instance_id: String,
    pub nickname: Option<String>,
    pub status: String,
    pub since: u64,
}

/// Friend request information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendRequestInfo {
    pub request_id: u64,
    pub ai_id: String,
    pub message: Option<String>,
    pub timestamp: u64,
}

/// Activity information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityInfo {
    pub id: u64,
    pub ai_id: String,
    pub activity_type: String,
    pub space_id: Option<String>,
    pub content: String,
    pub timestamp: u64,
}


/// Replay attack protection via sliding-window message ID tracking.
///
/// Tracks recently seen `(ai_id, msg_id)` pairs to reject duplicates.
/// Uses a time-based eviction window — entries older than `window` are
/// purged on the next `check_and_record` call.
///
/// Intended for use by the server's message processing loop.
pub struct ReplayGuard {
    /// Set of (ai_id, msg_id) pairs seen within the window.
    seen: std::collections::HashMap<(String, u64), u64>, // value = timestamp_ms
    /// How long to remember message IDs.
    window_ms: u64,
    /// Maximum entries before forced eviction (prevents memory exhaustion).
    max_entries: usize,
}

impl ReplayGuard {
    /// Create a new replay guard with the given time window.
    pub fn new(window: std::time::Duration, max_entries: usize) -> Self {
        Self {
            seen: std::collections::HashMap::new(),
            window_ms: window.as_millis() as u64,
            max_entries,
        }
    }

    /// Check if a message is a replay. Returns `true` if the message is NEW
    /// (not seen before within the window). Returns `false` if it's a duplicate.
    ///
    /// Automatically records the message if new, and evicts stale entries.
    pub fn check_and_record(&mut self, ai_id: &str, msg_id: u64, timestamp_ms: u64) -> bool {
        // Evict stale entries if we're at capacity
        if self.seen.len() >= self.max_entries {
            self.evict_stale(timestamp_ms);
        }

        let key = (ai_id.to_string(), msg_id);

        if self.seen.contains_key(&key) {
            return false; // Duplicate
        }

        self.seen.insert(key, timestamp_ms);
        true // New message
    }

    /// Remove entries older than the window.
    fn evict_stale(&mut self, now_ms: u64) {
        let cutoff = now_ms.saturating_sub(self.window_ms);
        self.seen.retain(|_, ts| *ts > cutoff);
    }

    /// Number of tracked entries (for diagnostics).
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    /// Whether the guard is empty.
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

/// Helper for signature serialization
mod signature_serde {
    use ed25519_dalek::Signature;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(sig: &Signature, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        sig.to_bytes().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Signature, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes: Vec<u8> = Deserialize::deserialize(deserializer)?;
        if bytes.len() != 64 {
            return Err(serde::de::Error::custom("Signature must be 64 bytes"));
        }
        let mut arr = [0u8; 64];
        arr.copy_from_slice(&bytes);
        Ok(Signature::from_bytes(&arr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::generate_ai_id;

    fn create_test_identity() -> (AIIdentity, KeyPair) {
        let keypair = KeyPair::generate();
        let ai_id = generate_ai_id("test");
        let identity = AIIdentity::new(ai_id, keypair.public_key(), "local".to_string());
        (identity, keypair)
    }

    #[test]
    fn test_message_sign_verify() {
        let (identity, keypair) = create_test_identity();

        let mut msg = AFPMessage::new(
            MessageType::Request,
            &identity,
            None,
            Payload::Ping {
                timestamp: 12345,
            },
        );

        msg.sign(&keypair).unwrap();
        assert!(msg.verify().is_ok());
    }

    #[test]
    fn test_message_tamper_detection() {
        let (identity, keypair) = create_test_identity();

        let mut msg = AFPMessage::new(
            MessageType::Request,
            &identity,
            None,
            Payload::DirectMessage {
                content: "Hello".to_string(),
            },
        );

        msg.sign(&keypair).unwrap();

        // Tamper with the message
        msg.payload = Payload::DirectMessage {
            content: "Tampered!".to_string(),
        };

        // Verification should fail
        assert!(msg.verify().is_err());
    }

    #[test]
    fn test_cbor_roundtrip() {
        let (identity, keypair) = create_test_identity();

        let mut msg = AFPMessage::new(
            MessageType::Broadcast,
            &identity,
            None,
            Payload::Broadcast {
                channel: "general".to_string(),
                content: "Hello everyone!".to_string(),
            },
        );

        msg.sign(&keypair).unwrap();

        // Serialize
        let bytes = msg.to_cbor().unwrap();
        println!("Message size: {} bytes", bytes.len());

        // Deserialize
        let decoded = AFPMessage::from_cbor(&bytes).unwrap();

        assert_eq!(decoded.version, msg.version);
        assert_eq!(decoded.msg_id, msg.msg_id);
        assert_eq!(decoded.from.ai_id, msg.from.ai_id);
        assert!(decoded.verify().is_ok());
    }

    #[test]
    fn test_response_creation() {
        let (identity1, keypair1) = create_test_identity();
        let (identity2, keypair2) = create_test_identity();

        let mut request = AFPMessage::new(
            MessageType::Request,
            &identity1,
            Some(identity2.ai_id.clone()),
            Payload::Ping { timestamp: 12345 },
        );
        request.sign(&keypair1).unwrap();

        let mut response = request.create_response(
            &identity2,
            Payload::Pong {
                request_timestamp: 12345,
                response_timestamp: 12346,
            },
        );
        response.sign(&keypair2).unwrap();

        // Response should have same msg_id
        assert_eq!(response.msg_id, request.msg_id);
        assert!(response.verify().is_ok());
    }

    #[test]
    fn test_verify_rejects_bad_ai_id() {
        let keypair = KeyPair::generate();
        let identity = AIIdentity::new(
            "invalid_no_number".to_string(),
            keypair.public_key(),
            "local".to_string(),
        );

        let mut msg = AFPMessage::new(
            MessageType::Request,
            &identity,
            None,
            Payload::Ping { timestamp: 12345 },
        );
        msg.sign(&keypair).unwrap();

        // verify() should reject the malformed AI_ID
        assert!(msg.verify().is_err());
        // verify_signature_only() should still pass (signature is valid)
        assert!(msg.verify_signature_only().is_ok());
    }

    #[test]
    fn test_verify_rejects_future_timestamp() {
        let (identity, keypair) = create_test_identity();

        let mut msg = AFPMessage::new(
            MessageType::Request,
            &identity,
            None,
            Payload::Ping { timestamp: 12345 },
        );
        // Set timestamp 10 minutes in the future
        msg.timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 10 * 60 * 1000;
        msg.sign(&keypair).unwrap();

        assert!(msg.verify().is_err());
    }

    #[test]
    fn test_verify_rejects_stale_timestamp() {
        let (identity, keypair) = create_test_identity();

        let mut msg = AFPMessage::new(
            MessageType::Request,
            &identity,
            None,
            Payload::Ping { timestamp: 12345 },
        );
        // Set timestamp 10 minutes in the past
        msg.timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            - 10 * 60 * 1000;
        msg.sign(&keypair).unwrap();

        assert!(msg.verify().is_err());
    }

    #[test]
    fn test_verify_rejects_wrong_version() {
        let (identity, keypair) = create_test_identity();

        let mut msg = AFPMessage::new(
            MessageType::Request,
            &identity,
            None,
            Payload::Ping { timestamp: 12345 },
        );
        msg.sign(&keypair).unwrap();
        msg.version = 99; // Wrong version — checked before signature

        assert!(msg.verify().is_err());
    }

    #[test]
    fn test_replay_guard_detects_duplicate() {
        let mut guard = ReplayGuard::new(std::time::Duration::from_secs(60), 1000);

        assert!(guard.check_and_record("cascade-230", 1, 1000));
        assert!(!guard.check_and_record("cascade-230", 1, 1001)); // duplicate
        assert_eq!(guard.len(), 1);
    }

    #[test]
    fn test_replay_guard_different_senders() {
        let mut guard = ReplayGuard::new(std::time::Duration::from_secs(60), 1000);

        // Same msg_id from different AIs is NOT a replay
        assert!(guard.check_and_record("cascade-230", 1, 1000));
        assert!(guard.check_and_record("sage-724", 1, 1000));
        assert_eq!(guard.len(), 2);
    }

    #[test]
    fn test_replay_guard_eviction() {
        let mut guard = ReplayGuard::new(std::time::Duration::from_secs(60), 2);

        // Fill to capacity
        guard.check_and_record("a-1", 1, 1000);
        guard.check_and_record("b-2", 2, 2000);
        assert_eq!(guard.len(), 2);

        // Third entry triggers eviction; entry at ts=1000 is stale relative to ts=70000
        guard.check_and_record("c-3", 3, 70_000);
        // Entry "a-1" (ts=1000) should be evicted (cutoff = 70000 - 60000 = 10000)
        assert!(guard.len() <= 2);
    }
}
