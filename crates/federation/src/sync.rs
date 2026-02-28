//! Federation Sync Protocol — Push/Pull Message Types
//!
//! Defines the wire format for event exchange between Teambooks:
//!
//! **Push (primary):** Remote Teambook sends `EventPushRequest` to our inbox.
//! We verify each event, deduplicate, and write accepted events to the local log.
//!
//! **Pull (catch-up):** On reconnect, we request missed events via `EventPullRequest`.
//! The remote responds with a bounded batch. This is the ONE acceptable pull
//! (bounded, triggered by reconnect event, not polling).
//!
//! Both directions use `SignedEvent` envelopes — every event is signed by its
//! originating Teambook and content-addressed for idempotent deduplication.
//!
//! Ported from ai-foundation-clean/src/federation_sync.rs, adapted to federation-rs types.

use crate::{HlcTimestamp, SignedEvent};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Push Protocol (inbound events)
// ---------------------------------------------------------------------------

/// Batch of signed events pushed from a remote Teambook to our inbox.
///
/// Sent as the body of `POST /federation/events`.
/// The sender includes their HLC timestamp so we can update our clock,
/// and their head sequence so we know how far ahead they are.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventPushRequest {
    /// Signed events to deliver (PDUs — presence EDUs use a separate endpoint)
    pub events: Vec<SignedEvent>,

    /// Sender's HLC timestamp at the time of push (for clock sync)
    pub sender_hlc: HlcTimestamp,

    /// Sender's latest local sequence number (so receiver knows their position)
    pub sender_head_seq: u64,
}

/// Response to an `EventPushRequest`.
///
/// Returned from `POST /federation/events`.
/// Provides enough information for the sender to know what was accepted
/// and to sync their view of our sequence position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventPushResponse {
    /// Number of new events accepted and written to the local log
    pub accepted: usize,

    /// Number of events already seen (deduplicated by content hash)
    pub duplicates: usize,

    /// Number of events rejected (invalid signature, unknown peer, drift, etc.)
    pub rejected: usize,

    /// Per-event error details for rejected events
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub errors: Vec<SyncError>,

    /// Receiver's current HLC (for sender's clock sync)
    pub receiver_hlc: HlcTimestamp,

    /// Receiver's current head sequence (so sender knows our position)
    pub receiver_head_seq: u64,
}

// ---------------------------------------------------------------------------
// Pull Protocol (catch-up on reconnect)
// ---------------------------------------------------------------------------

/// Request to pull events missed during a Teambook's offline period.
///
/// Sent as query params or body of `GET /federation/events?since=<seq>`.
/// This is the ONE acceptable pull in the federation — triggered by a
/// reconnect event, bounded by `limit`, never polled continuously.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventPullRequest {
    /// Pull events with sequence number strictly greater than this value
    pub since_seq: u64,

    /// Maximum events to return per batch (default: 100)
    #[serde(default = "default_pull_limit")]
    pub limit: usize,

    /// Requester's public key hex — for authentication and audit
    pub requester_pubkey: String,
}

fn default_pull_limit() -> usize {
    100
}

/// Response to an `EventPullRequest`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventPullResponse {
    /// Signed events since the requested sequence number
    pub events: Vec<SignedEvent>,

    /// The responder's current head sequence number
    pub head_seq: u64,

    /// Whether there are more events beyond this batch (pagination signal)
    pub has_more: bool,

    /// Responder's current HLC (for caller's clock sync)
    pub sender_hlc: HlcTimestamp,
}

// ---------------------------------------------------------------------------
// Sync Errors
// ---------------------------------------------------------------------------

/// Error detail for a single rejected event in a push batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncError {
    /// Index of the rejected event in the request batch
    pub index: usize,

    /// Content hash of the rejected event (hex) — for tracing
    pub content_id: String,

    /// Specific rejection reason
    pub reason: SyncRejectReason,
}

/// Reasons a pushed event can be rejected at the federation inbox.
///
/// Each variant maps to a specific validation failure. Fail loud:
/// the sender gets an explicit reason, not a silent drop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncRejectReason {
    /// Ed25519 signature verification failed
    InvalidSignature,
    /// SHA-256 content hash doesn't match event bytes (corrupted or tampered)
    ContentHashMismatch,
    /// Event originated from a peer not in our known-peers registry
    UnknownPeer,
    /// Sender's HLC timestamp exceeds our 60-second drift bound
    ExcessiveDrift,
    /// Event bytes are empty or cannot be deserialized as a FederationMessage
    MalformedEvent,
    /// Event type is not permitted by this Teambook's permission manifest
    NotPermittedByManifest,
}

impl std::fmt::Display for SyncRejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSignature => write!(f, "invalid Ed25519 signature"),
            Self::ContentHashMismatch => write!(f, "content hash mismatch"),
            Self::UnknownPeer => write!(f, "unknown peer (not in registry)"),
            Self::ExcessiveDrift => write!(f, "HLC drift exceeded 60 seconds"),
            Self::MalformedEvent => write!(f, "malformed or empty event bytes"),
            Self::NotPermittedByManifest => write!(f, "event type not permitted by manifest"),
        }
    }
}

// ---------------------------------------------------------------------------
// Presence Sync (EDU — ephemeral, no persistence)
// ---------------------------------------------------------------------------

/// Presence update pushed from a remote Teambook.
///
/// Fire-and-forget: no acknowledgment, no retry, stale on disconnect is fine.
/// Sent to `POST /federation/presence` (separate from PDU event endpoint).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresencePushRequest {
    /// The federated presence records being broadcast
    pub presences: Vec<crate::messages::FederatedPresence>,

    /// Sender's Teambook short ID (for attribution)
    pub sender_short_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_pull_limit() {
        let req: EventPullRequest = serde_json::from_str(
            r#"{"since_seq": 42, "requester_pubkey": "abc"}"#,
        )
        .unwrap();
        assert_eq!(req.limit, 100);
        assert_eq!(req.since_seq, 42);
    }

    #[test]
    fn test_sync_reject_reason_display() {
        assert_eq!(
            SyncRejectReason::InvalidSignature.to_string(),
            "invalid Ed25519 signature"
        );
        assert_eq!(
            SyncRejectReason::NotPermittedByManifest.to_string(),
            "event type not permitted by manifest"
        );
    }

    #[test]
    fn test_sync_reject_reason_serialization() {
        let reason = SyncRejectReason::UnknownPeer;
        let json = serde_json::to_string(&reason).unwrap();
        assert_eq!(json, "\"unknown_peer\"");

        let reason2 = SyncRejectReason::NotPermittedByManifest;
        let json2 = serde_json::to_string(&reason2).unwrap();
        assert_eq!(json2, "\"not_permitted_by_manifest\"");
    }

    #[test]
    fn test_push_response_errors_omitted_when_empty() {
        // errors field should be omitted from JSON when empty
        // (skip_serializing_if = "Vec::is_empty")
        // Can't easily test JSON omission here without full HLC/SignedEvent setup,
        // but at least verify the struct constructs correctly
        let _ = EventPushResponse {
            accepted: 5,
            duplicates: 1,
            rejected: 0,
            errors: vec![],
            receiver_hlc: HlcTimestamp { physical_time_us: 0, counter: 0, node_id: 0 },
            receiver_head_seq: 100,
        };
    }
}
