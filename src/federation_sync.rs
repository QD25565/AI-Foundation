//! Federation Event Sync — Push/Pull Protocol Between Teambooks
//!
//! Two sync modes:
//!
//! **Push (primary):** When a local event is created, sign it and push
//! to all registered peers via `POST /api/federation/events`.
//!
//! **Pull (catch-up):** On startup or after suspected missed events,
//! pull from each peer via `GET /api/federation/events?since=<seq>`.
//!
//! Every received event is:
//! 1. Verified (Ed25519 signature + content hash)
//! 2. Deduplicated (content hash lookup)
//! 3. HLC-updated (receive merges remote timestamp into local clock)
//! 4. Appended to local event log (via CLI subprocess)
//!
//! Fail loud: invalid signatures, unknown peers, and malformed events
//! are rejected immediately with specific error codes.

use crate::crypto::SignedEvent;
use crate::federation::{FederationState, PeerInfo, PeerStatus};
use crate::hlc::HlcTimestamp;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

// ---------------------------------------------------------------------------
// Sync Protocol Messages
// ---------------------------------------------------------------------------

/// Batch of signed events pushed between Teambooks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventPushRequest {
    /// Signed events to deliver
    pub events: Vec<SignedEvent>,

    /// HLC timestamp of the sending Teambook (for clock sync)
    pub sender_hlc: HlcTimestamp,

    /// The sender's latest local sequence number (so receiver knows our position)
    pub sender_head_seq: u64,
}

/// Response to a push request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventPushResponse {
    /// Number of new events accepted
    pub accepted: usize,

    /// Number of events skipped (already seen / duplicates)
    pub duplicates: usize,

    /// Number of events rejected (invalid signature, unknown peer, etc.)
    pub rejected: usize,

    /// Specific errors for rejected events (index -> reason)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<SyncError>,

    /// Receiver's current HLC (for sender's clock sync)
    pub receiver_hlc: HlcTimestamp,

    /// Receiver's current head sequence (so sender knows our position)
    pub receiver_head_seq: u64,
}

/// Request to pull events since a given sequence number.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventPullRequest {
    /// Pull events with sequence > this value
    pub since_seq: u64,

    /// Maximum number of events to return (default: 100)
    #[serde(default = "default_pull_limit")]
    pub limit: usize,

    /// Requester's public key (for authentication)
    pub requester_pubkey: String,
}

fn default_pull_limit() -> usize {
    100
}

/// Response to a pull request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventPullResponse {
    /// Signed events since the requested sequence
    pub events: Vec<SignedEvent>,

    /// The sender's current head sequence
    pub head_seq: u64,

    /// Whether there are more events beyond this batch
    pub has_more: bool,

    /// Sender's current HLC
    pub sender_hlc: HlcTimestamp,
}

/// Error details for a rejected event during sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncError {
    /// Index of the event in the batch
    pub index: usize,

    /// Content hash of the rejected event (if available)
    pub content_id: String,

    /// What went wrong
    pub reason: SyncRejectReason,
}

/// Specific reasons an event can be rejected during sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncRejectReason {
    /// Ed25519 signature verification failed
    InvalidSignature,
    /// SHA-256 content hash doesn't match event bytes
    ContentHashMismatch,
    /// Event came from an unregistered peer
    UnknownPeer,
    /// HLC timestamp exceeds drift bound
    ExcessiveDrift,
    /// Event bytes are malformed or empty
    MalformedEvent,
}

impl std::fmt::Display for SyncRejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSignature => write!(f, "invalid Ed25519 signature"),
            Self::ContentHashMismatch => write!(f, "content hash mismatch"),
            Self::UnknownPeer => write!(f, "unknown peer"),
            Self::ExcessiveDrift => write!(f, "HLC drift exceeded"),
            Self::MalformedEvent => write!(f, "malformed event"),
        }
    }
}

// ---------------------------------------------------------------------------
// Inbound Sync (Receiving Events)
// ---------------------------------------------------------------------------

/// Process a batch of pushed events from a peer.
///
/// For each event:
/// 1. Verify signature + content hash
/// 2. Check peer is registered
/// 3. Deduplicate by content hash
/// 4. Accept or reject
///
/// Returns a response with counts of accepted/duplicated/rejected events.
pub async fn process_push(
    federation: &FederationState,
    request: &EventPushRequest,
) -> EventPushResponse {
    let mut accepted = 0usize;
    let mut duplicates = 0usize;
    let mut rejected = 0usize;
    let mut errors = Vec::new();

    // Update HLC with sender's timestamp
    if let Err(e) = federation.clock.receive(&request.sender_hlc) {
        warn!("Sender HLC drift: {}", e);
        // Don't reject the whole batch — individual events are still verifiable
    }

    for (i, event) in request.events.iter().enumerate() {
        let content_id_hex = event.content_id_hex();

        // 1. Check for empty/malformed event bytes
        if event.event_bytes.is_empty() {
            errors.push(SyncError {
                index: i,
                content_id: content_id_hex,
                reason: SyncRejectReason::MalformedEvent,
            });
            rejected += 1;
            continue;
        }

        // 2. Verify signature and content hash
        if let Err(e) = event.verify() {
            let reason = match e {
                crate::crypto::SignedEventError::InvalidSignature => {
                    SyncRejectReason::InvalidSignature
                }
                crate::crypto::SignedEventError::ContentHashMismatch => {
                    SyncRejectReason::ContentHashMismatch
                }
            };
            warn!(
                event = %content_id_hex,
                origin = %event.origin_short_id(),
                error = %e,
                "Rejected event"
            );
            errors.push(SyncError {
                index: i,
                content_id: content_id_hex,
                reason,
            });
            rejected += 1;
            continue;
        }

        // 3. Check peer is registered
        if !federation.is_known_peer(&event.origin_pubkey).await {
            warn!(
                origin = %event.origin_short_id(),
                "Event from unknown peer"
            );
            errors.push(SyncError {
                index: i,
                content_id: content_id_hex,
                reason: SyncRejectReason::UnknownPeer,
            });
            rejected += 1;
            continue;
        }

        // 4. Deduplication
        if !federation.is_new_event(&content_id_hex).await {
            duplicates += 1;
            continue;
        }

        // 5. Accept the event
        federation.mark_event_seen(content_id_hex.clone()).await;

        // Touch the peer so we know they're alive
        federation
            .touch_peer(&event.origin_pubkey_hex())
            .await;

        accepted += 1;

        debug!(
            event = %content_id_hex,
            origin = %event.origin_short_id(),
            "Accepted federation event"
        );
    }

    if accepted > 0 || rejected > 0 {
        info!(
            accepted,
            duplicates,
            rejected,
            "Processed federation push"
        );
    }

    let current_hlc = federation.clock.tick();

    EventPushResponse {
        accepted,
        duplicates,
        rejected,
        errors,
        receiver_hlc: current_hlc,
        receiver_head_seq: 0, // TODO: read from local event log
    }
}

// ---------------------------------------------------------------------------
// Outbound Sync (Pushing Events to Peers)
// ---------------------------------------------------------------------------

/// Push signed events to a single peer.
///
/// Uses HTTP POST to the peer's federation endpoint.
/// Returns the peer's response or an error.
pub async fn push_to_peer(
    peer: &PeerInfo,
    events: Vec<SignedEvent>,
    sender_hlc: HlcTimestamp,
    sender_head_seq: u64,
) -> Result<EventPushResponse, SyncTransportError> {
    let url = format!("{}/api/federation/events", peer.endpoint.trim_end_matches('/'));

    let request = EventPushRequest {
        events,
        sender_hlc,
        sender_head_seq,
    };

    let body = serde_json::to_string(&request).map_err(|e| SyncTransportError {
        peer_id: peer.short_id(),
        endpoint: url.clone(),
        reason: format!("serialization failed: {}", e),
    })?;

    // Use reqwest-free approach: spawn a curl subprocess
    // This keeps dependencies minimal and matches the CLI-subprocess pattern
    let output = tokio::process::Command::new("curl")
        .args([
            "-s",
            "-X", "POST",
            "-H", "Content-Type: application/json",
            "-d", &body,
            "--connect-timeout", "5",
            "--max-time", "30",
            &url,
        ])
        .output()
        .await
        .map_err(|e| SyncTransportError {
            peer_id: peer.short_id(),
            endpoint: url.clone(),
            reason: format!("curl failed: {}", e),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SyncTransportError {
            peer_id: peer.short_id(),
            endpoint: url,
            reason: format!("HTTP request failed: {}", stderr.trim()),
        });
    }

    let response_body = String::from_utf8_lossy(&output.stdout);
    let response: EventPushResponse =
        serde_json::from_str(&response_body).map_err(|e| SyncTransportError {
            peer_id: peer.short_id(),
            endpoint: url,
            reason: format!("invalid response: {}", e),
        })?;

    Ok(response)
}

/// Pull events from a peer since a given sequence number.
pub async fn pull_from_peer(
    peer: &PeerInfo,
    since_seq: u64,
    limit: usize,
    requester_pubkey: &str,
) -> Result<EventPullResponse, SyncTransportError> {
    let url = format!(
        "{}/api/federation/events?since={}&limit={}&pubkey={}",
        peer.endpoint.trim_end_matches('/'),
        since_seq,
        limit,
        requester_pubkey
    );

    let output = tokio::process::Command::new("curl")
        .args([
            "-s",
            "-X", "GET",
            "--connect-timeout", "5",
            "--max-time", "30",
            &url,
        ])
        .output()
        .await
        .map_err(|e| SyncTransportError {
            peer_id: peer.short_id(),
            endpoint: url.clone(),
            reason: format!("curl failed: {}", e),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SyncTransportError {
            peer_id: peer.short_id(),
            endpoint: url,
            reason: format!("HTTP request failed: {}", stderr.trim()),
        });
    }

    let response_body = String::from_utf8_lossy(&output.stdout);
    let response: EventPullResponse =
        serde_json::from_str(&response_body).map_err(|e| SyncTransportError {
            peer_id: peer.short_id(),
            endpoint: url,
            reason: format!("invalid response: {}", e),
        })?;

    Ok(response)
}

/// Push events to ALL registered peers concurrently.
///
/// Returns results per peer. Failed pushes are logged but don't
/// block other peers — federation is resilient to individual failures.
pub async fn push_to_all_peers(
    federation: &FederationState,
    events: Vec<SignedEvent>,
) -> Vec<(String, Result<EventPushResponse, SyncTransportError>)> {
    let peers = federation.list_peers().await;
    let sender_hlc = federation.clock.tick();

    let mut handles = Vec::new();

    for peer in peers {
        if peer.status == PeerStatus::Removed {
            continue;
        }

        let events_clone = events.clone();
        let peer_id = peer.short_id();

        let handle = tokio::spawn(async move {
            let result = push_to_peer(&peer, events_clone, sender_hlc, 0).await;
            (peer_id, result)
        });

        handles.push(handle);
    }

    let mut results = Vec::new();
    for handle in handles {
        match handle.await {
            Ok((peer_id, result)) => {
                if let Err(ref e) = result {
                    warn!(peer = %peer_id, error = %e, "Push to peer failed");
                }
                results.push((peer_id, result));
            }
            Err(e) => {
                error!("Push task panicked: {}", e);
            }
        }
    }

    results
}

/// Pull and process events from ALL registered peers.
///
/// For each peer, pulls events since our last known sync point,
/// then processes them through the standard verification pipeline.
pub async fn pull_from_all_peers(
    federation: &FederationState,
) -> Vec<(String, Result<usize, SyncTransportError>)> {
    let peers = federation.list_peers().await;
    let our_pubkey = federation.identity.public_key_hex();

    let mut results = Vec::new();

    for peer in peers {
        if peer.status == PeerStatus::Removed {
            continue;
        }

        let peer_id = peer.short_id();
        let since_seq = peer.last_synced_seq;

        match pull_from_peer(&peer, since_seq, 100, &our_pubkey).await {
            Ok(response) => {
                // Process pulled events through the same verification pipeline
                let push_req = EventPushRequest {
                    events: response.events,
                    sender_hlc: response.sender_hlc,
                    sender_head_seq: response.head_seq,
                };

                let push_resp = process_push(federation, &push_req).await;

                // Update sync position
                federation
                    .update_peer_sync_seq(&peer.pubkey_hex(), response.head_seq)
                    .await;

                info!(
                    peer = %peer_id,
                    accepted = push_resp.accepted,
                    duplicates = push_resp.duplicates,
                    "Pull sync complete"
                );

                results.push((peer_id, Ok(push_resp.accepted)));
            }
            Err(e) => {
                warn!(peer = %peer_id, error = %e, "Pull from peer failed");
                results.push((peer_id, Err(e)));
            }
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Transport Errors
// ---------------------------------------------------------------------------

/// Error during federation sync transport (HTTP/network layer).
#[derive(Debug, Clone)]
pub struct SyncTransportError {
    pub peer_id: String,
    pub endpoint: String,
    pub reason: String,
}

impl std::fmt::Display for SyncTransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "sync error with peer {}: {} ({})",
            self.peer_id, self.reason, self.endpoint
        )
    }
}

impl std::error::Error for SyncTransportError {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::TeambookIdentity;

    #[tokio::test]
    async fn test_process_push_valid_event() {
        let federation = FederationState::init(
            "receiver".to_string(),
            "http://localhost:8080".to_string(),
        )
        .await
        .unwrap();

        // Create a sender identity and register them as a peer
        let sender = TeambookIdentity::generate();
        let req = crate::federation::PeerRegistrationRequest {
            public_key: sender.public_key(),
            display_name: "sender".to_string(),
            endpoint: "http://localhost:8081".to_string(),
            challenge_nonce: "test-nonce".to_string(),
            challenge_signature: hex::encode(sender.sign(b"test-nonce")),
        };
        federation.handle_registration(&req).await;

        // Create and sign an event
        let event = SignedEvent::sign(b"test event data".to_vec(), &sender);

        let push_req = EventPushRequest {
            events: vec![event],
            sender_hlc: HlcTimestamp::zero(42),
            sender_head_seq: 1,
        };

        let resp = process_push(&federation, &push_req).await;

        assert_eq!(resp.accepted, 1);
        assert_eq!(resp.rejected, 0);
        assert_eq!(resp.duplicates, 0);
    }

    #[tokio::test]
    async fn test_process_push_duplicate_event() {
        let federation = FederationState::init(
            "receiver".to_string(),
            "http://localhost:8080".to_string(),
        )
        .await
        .unwrap();

        let sender = TeambookIdentity::generate();
        let req = crate::federation::PeerRegistrationRequest {
            public_key: sender.public_key(),
            display_name: "sender".to_string(),
            endpoint: "http://localhost:8081".to_string(),
            challenge_nonce: "nonce".to_string(),
            challenge_signature: hex::encode(sender.sign(b"nonce")),
        };
        federation.handle_registration(&req).await;

        let event = SignedEvent::sign(b"event data".to_vec(), &sender);

        let push_req = EventPushRequest {
            events: vec![event.clone(), event],
            sender_hlc: HlcTimestamp::zero(42),
            sender_head_seq: 1,
        };

        let resp = process_push(&federation, &push_req).await;

        assert_eq!(resp.accepted, 1);
        assert_eq!(resp.duplicates, 1);
    }

    #[tokio::test]
    async fn test_process_push_unknown_peer() {
        let federation = FederationState::init(
            "receiver".to_string(),
            "http://localhost:8080".to_string(),
        )
        .await
        .unwrap();

        // Don't register this sender
        let unknown = TeambookIdentity::generate();
        let event = SignedEvent::sign(b"suspicious data".to_vec(), &unknown);

        let push_req = EventPushRequest {
            events: vec![event],
            sender_hlc: HlcTimestamp::zero(99),
            sender_head_seq: 1,
        };

        let resp = process_push(&federation, &push_req).await;

        assert_eq!(resp.accepted, 0);
        assert_eq!(resp.rejected, 1);
        assert_eq!(resp.errors[0].reason, SyncRejectReason::UnknownPeer);
    }

    #[tokio::test]
    async fn test_process_push_tampered_event() {
        let federation = FederationState::init(
            "receiver".to_string(),
            "http://localhost:8080".to_string(),
        )
        .await
        .unwrap();

        let sender = TeambookIdentity::generate();
        let req = crate::federation::PeerRegistrationRequest {
            public_key: sender.public_key(),
            display_name: "sender".to_string(),
            endpoint: "http://localhost:8081".to_string(),
            challenge_nonce: "n".to_string(),
            challenge_signature: hex::encode(sender.sign(b"n")),
        };
        federation.handle_registration(&req).await;

        let mut event = SignedEvent::sign(b"original data".to_vec(), &sender);
        event.event_bytes[0] ^= 0xFF; // tamper

        let push_req = EventPushRequest {
            events: vec![event],
            sender_hlc: HlcTimestamp::zero(42),
            sender_head_seq: 1,
        };

        let resp = process_push(&federation, &push_req).await;

        assert_eq!(resp.accepted, 0);
        assert_eq!(resp.rejected, 1);
        assert_eq!(
            resp.errors[0].reason,
            SyncRejectReason::ContentHashMismatch
        );
    }
}
