//! Federation Inbox — Inbound event processing for PDUs and EDUs.
//!
//! Two delivery paths:
//!
//! **PDU path** (`POST /federation/events`):
//! Receives a batch of `SignedEvent` envelopes. Each is validated:
//! 1. Manifest gate — `accepts_inbound()` must be true
//! 2. Content hash integrity + Ed25519 signature (`SignedEvent::verify()`)
//! 3. HLC drift bound check (±60 seconds)
//! 4. Payload deserialization + manifest permission check
//! 5. Append `FederationInboxEvent` to inbox.jsonl
//!
//! **EDU path** (`POST /federation/presence`):
//! Fire-and-forget presence updates. No acknowledgment, no retry, no sig check.
//! Manifest `expose.presence` flag gates acceptance.
//! Appends `FEDERATED_PRESENCE` events to inbox.jsonl and updates AiRegistry.
//!
//! **inbox.jsonl** is the delivery pipe to the AI context window.
//! hook-bulletin (Step 8) reads from it to inject `|FEDERATION|` sections.
//!
//! **JSONL contract** (append-only, one JSON object per line):
//! ```json
//! {
//!   "id": "abc123...",              // deduplication key (content_id for PDUs)
//!   "source_teambook": "a3f7c2d1",  // remote Teambook short ID or name
//!   "source_ai": "alpha-001",        // optional: AI that generated the event
//!   "event_type": "FEDERATED_BROADCAST",
//!   "summary": "human-readable summary (no file names, no raw ops)",
//!   "created_at": 1771770052        // Unix secs
//! }
//! ```

use crate::{
    AiRegistry, HlcTimestamp, InboundActions, PermissionManifest,
    EventPushRequest, EventPushResponse, PresencePushRequest,
    SyncError, SyncRejectReason,
};
use crate::messages::{FederationMessage, FederationPayload, SignedEventError};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Inbox Event Record (JSONL format)
// ---------------------------------------------------------------------------

/// A federation event written to inbox.jsonl.
///
/// This is the canonical format consumed by hook-bulletin (Step 8) and any
/// other consumer that processes the federation inbox stream.
///
/// The `id` field is the deduplication key — consumers should skip IDs they
/// have already displayed. PDUs use the `content_id` hex (SHA-256 of event
/// bytes) for idempotency. EDUs use a timestamp-prefixed string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationInboxEvent {
    /// Unique event ID.
    ///
    /// - PDUs: `content_id` hex from `SignedEvent` (SHA-256 of event bytes)
    /// - EDUs: `"presence-{ai_id}-{now_us}"` (microsecond precision)
    pub id: String,

    /// Source Teambook short ID or name (e.g. "a3f7c2d1", "TestTeambook-B").
    pub source_teambook: String,

    /// Source AI ID, if known (e.g. "alpha-001"). Absent for anonymous events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ai: Option<String>,

    /// Event type string (e.g. "FEDERATED_PRESENCE", "FEDERATED_BROADCAST").
    pub event_type: String,

    /// Human-readable summary. Semantic level only — never contains file paths,
    /// raw tool calls, or operational details.
    pub summary: String,

    /// Unix timestamp in seconds when this event was received by this Teambook.
    pub created_at: u64,
}

// ---------------------------------------------------------------------------
// Inbox Writer
// ---------------------------------------------------------------------------

/// Thread-safe appender for federation inbox.jsonl.
///
/// Multiple async handlers may call `write_event()` concurrently. All share a
/// single `Arc<Mutex<File>>` so writes are serialized and never interleaved.
///
/// Each call to `write_event()` appends exactly one complete JSON line.
#[derive(Clone)]
pub struct InboxWriter {
    inner: Arc<Mutex<InboxWriterInner>>,
}

struct InboxWriterInner {
    #[allow(dead_code)]
    path: PathBuf,
    file: std::fs::File,
}

impl InboxWriter {
    /// Open (or create) the inbox.jsonl file for appending.
    ///
    /// Creates parent directories as needed. Fails loudly on IO error.
    pub fn open(path: PathBuf) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(InboxWriterInner { path, file })),
        })
    }

    /// Default path: `~/.ai-foundation/federation/inbox.jsonl`
    pub fn default_path() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".ai-foundation").join("federation").join("inbox.jsonl")
    }

    /// Append a federation event to inbox.jsonl.
    ///
    /// Serializes the event as a single JSON line followed by `\n`.
    /// Thread-safe: acquires the internal mutex for the duration of the write.
    pub fn write_event(&self, event: &FederationInboxEvent) -> std::io::Result<()> {
        let line = serde_json::to_string(event)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut inner = self.inner.lock().expect("inbox writer mutex poisoned");
        writeln!(inner.file, "{}", line)?;
        inner.file.flush()
    }
}

// ---------------------------------------------------------------------------
// Inbox State
// ---------------------------------------------------------------------------

/// Shared state for all inbox request handlers.
///
/// Designed to be cheap to clone — the `InboxWriter` and `AiRegistry` carry
/// their heavy fields behind `Arc` internally.
#[derive(Clone)]
pub struct InboxState {
    /// Permission manifest (operator ceiling for all exposure decisions).
    pub manifest: Arc<PermissionManifest>,

    /// Append writer for inbox.jsonl.
    pub writer: InboxWriter,

    /// AI registry — updated on presence EDUs.
    pub registry: AiRegistry,
}

impl InboxState {
    pub fn new(
        manifest: PermissionManifest,
        registry: AiRegistry,
        writer: InboxWriter,
    ) -> Self {
        Self {
            manifest: Arc::new(manifest),
            writer,
            registry,
        }
    }
}

// ---------------------------------------------------------------------------
// PDU Inbox (POST /federation/events)
// ---------------------------------------------------------------------------

/// HLC drift bound — events with sender HLC more than 60 seconds from our
/// local clock are rejected to prevent replay attacks from stale caches.
const MAX_DRIFT_US: u64 = 60 * 1_000_000;

/// Process an inbound `EventPushRequest` (batch of signed PDUs).
///
/// Validation pipeline per event:
/// 1. Manifest gate — `accepts_inbound()` must be true (checked once for batch)
/// 2. HLC drift check — sender physical time within ±60s of local clock
/// 3. `SignedEvent::verify()` — content hash + Ed25519 signature
/// 4. `FederationMessage::from_bytes()` — CBOR deserialization
/// 5. `classify_payload()` — manifest permission check, event type, summary
/// 6. `InboxWriter::write_event()` — append to inbox.jsonl
///
/// Returns `EventPushResponse` with per-event counts and error details.
/// This function is synchronous and pure — it does not update any registry.
/// The Gateway (Step 9) is responsible for per-peer state management.
pub fn process_push_request(
    state: &InboxState,
    request: EventPushRequest,
) -> EventPushResponse {
    let now_us = now_us();
    let receiver_hlc = current_hlc(now_us);

    // --- Batch gate: manifest must accept inbound connections ---
    if !state.manifest.accepts_inbound() {
        let n = request.events.len();
        return EventPushResponse {
            accepted: 0,
            duplicates: 0,
            rejected: n,
            errors: request.events.iter().enumerate().map(|(i, e)| SyncError {
                index: i,
                content_id: e.content_id.clone(),
                reason: SyncRejectReason::NotPermittedByManifest,
            }).collect(),
            receiver_hlc,
            receiver_head_seq: 0,
        };
    }

    // --- Batch drift check (single sender HLC for the whole push) ---
    let sender_us = request.sender_hlc.physical_time_us;
    let drift = now_us.abs_diff(sender_us);

    let mut accepted = 0usize;
    let duplicates = 0usize; // dedup is handled by FederationGateway (Step 9)
    let mut errors: Vec<SyncError> = Vec::new();

    for (i, signed_event) in request.events.iter().enumerate() {
        // 1. HLC drift
        if drift > MAX_DRIFT_US {
            errors.push(SyncError {
                index: i,
                content_id: signed_event.content_id.clone(),
                reason: SyncRejectReason::ExcessiveDrift,
            });
            continue;
        }

        // 2. Content hash + signature
        if let Err(verify_err) = signed_event.verify() {
            let reason = match verify_err {
                SignedEventError::ContentHashMismatch => SyncRejectReason::ContentHashMismatch,
                SignedEventError::InvalidPublicKey | SignedEventError::InvalidSignature => {
                    SyncRejectReason::InvalidSignature
                }
            };
            errors.push(SyncError {
                index: i,
                content_id: signed_event.content_id.clone(),
                reason,
            });
            continue;
        }

        // 3. Payload deserialization
        let msg = match FederationMessage::from_bytes(&signed_event.event_bytes) {
            Ok(m) => m,
            Err(_) => {
                errors.push(SyncError {
                    index: i,
                    content_id: signed_event.content_id.clone(),
                    reason: SyncRejectReason::MalformedEvent,
                });
                continue;
            }
        };

        // 4. Manifest permission + classify
        let (event_type, summary, permitted) =
            classify_payload(&state.manifest, &msg.payload);

        if !permitted {
            errors.push(SyncError {
                index: i,
                content_id: signed_event.content_id.clone(),
                reason: SyncRejectReason::NotPermittedByManifest,
            });
            continue;
        }

        // 5. Derive source_teambook from origin_pubkey short ID (first 8 hex chars)
        let source_teambook = signed_event.origin_pubkey
            .get(..8)
            .unwrap_or(&signed_event.origin_pubkey)
            .to_string();

        let inbox_event = FederationInboxEvent {
            id: signed_event.content_id.clone(),
            source_teambook,
            source_ai: None, // Populated by Gateway (Step 9) after peer resolution
            event_type,
            summary,
            created_at: now_us / 1_000_000,
        };

        // 6. Write to inbox.jsonl
        match state.writer.write_event(&inbox_event) {
            Ok(_) => accepted += 1,
            Err(e) => {
                // IO failure — fail loudly. The sender will retry and succeed
                // once the IO condition clears.
                eprintln!(
                    "federation inbox: IO error writing event {}: {}",
                    inbox_event.id, e
                );
                errors.push(SyncError {
                    index: i,
                    content_id: signed_event.content_id.clone(),
                    reason: SyncRejectReason::MalformedEvent,
                });
            }
        }
    }

    let rejected = errors.len();
    EventPushResponse {
        accepted,
        duplicates,
        rejected,
        errors,
        receiver_hlc,
        receiver_head_seq: 0, // Full sequence tracking is Step 9 (Gateway)
    }
}

// ---------------------------------------------------------------------------
// EDU Inbox (POST /federation/presence)
// ---------------------------------------------------------------------------

/// Process an inbound `PresencePushRequest` (fire-and-forget presence EDUs).
///
/// EDUs don't carry Ed25519 signatures — presence is ephemeral and stale-on-
/// disconnect is acceptable. The manifest `expose.presence` flag gates all
/// acceptance; if false, the request is silently dropped (no ack required).
///
/// Side effects:
/// 1. Writes `FEDERATED_PRESENCE` events to inbox.jsonl
/// 2. Updates the `AiRegistry` with each AI's current presence
///
/// Note: `teambook_pubkey_hex` is not available in EDUs. The registry entry
/// uses `sender_short_id` as a placeholder — the Gateway (Step 9) will update
/// the entry with the full pubkey when the connection is established.
pub async fn process_presence_request(state: &InboxState, request: PresencePushRequest) {
    if !state.manifest.may_expose_presence() {
        return; // Presence not permitted — silent drop (EDU semantics)
    }

    let now_us = now_us();
    let sender_short_id = &request.sender_short_id;

    for presence in &request.presences {
        let summary = match &presence.activity {
            Some(activity) => {
                format!("{} is {} ({})", presence.ai_id, presence.status, activity)
            }
            None => format!("{} is {}", presence.ai_id, presence.status),
        };

        // Microsecond ID for dedup — two updates for the same AI in the same
        // microsecond are vanishingly unlikely and harmless if they collide.
        let inbox_event = FederationInboxEvent {
            id: format!("presence-{}-{}", presence.ai_id, now_us),
            source_teambook: sender_short_id.clone(),
            source_ai: Some(presence.ai_id.clone()),
            event_type: "FEDERATED_PRESENCE".to_string(),
            summary,
            created_at: now_us / 1_000_000,
        };

        if let Err(e) = state.writer.write_event(&inbox_event) {
            eprintln!(
                "federation inbox: IO error writing presence for {}: {}",
                presence.ai_id, e
            );
        }

        // Update AI registry. pubkey_hex is not available in EDUs — the
        // Gateway will refresh this entry with the full pubkey on connection.
        state
            .registry
            .register_remote(
                &presence.ai_id,
                sender_short_id, // placeholder until Gateway resolves full pubkey
                sender_short_id,
                sender_short_id, // use short ID as name until Gateway provides display name
                &presence.status,
                presence.activity.clone(),
            )
            .await;
    }
}

// ---------------------------------------------------------------------------
// Payload classification
// ---------------------------------------------------------------------------

/// Classify a `FederationPayload` for inbox delivery.
///
/// Returns `(event_type, summary, permitted)`:
/// - `event_type`: the `FEDERATED_*` string written to inbox.jsonl
/// - `summary`: human-readable description (semantic level — no raw ops)
/// - `permitted`: whether the manifest allows this event type to cross
///
/// Only `PresenceUpdate`, `Broadcast`, and `DirectMessage` can be permitted.
/// All handshake, routing, and sync payloads are rejected (`permitted = false`).
fn classify_payload(
    manifest: &PermissionManifest,
    payload: &FederationPayload,
) -> (String, String, bool) {
    match payload {
        FederationPayload::PresenceUpdate(p) => {
            let permitted = manifest.may_expose_presence();
            let summary = match &p.activity {
                Some(a) => format!("{} is {} ({})", p.ai_id, p.status, a),
                None => format!("{} is {}", p.ai_id, p.status),
            };
            ("FEDERATED_PRESENCE".to_string(), summary, permitted)
        }

        FederationPayload::Broadcast { channel, content } => {
            let permitted = manifest.may_expose_broadcasts();
            let summary = truncate_summary(&format!("[{}] {}", channel, content), 200);
            ("FEDERATED_BROADCAST".to_string(), summary, permitted)
        }

        FederationPayload::DirectMessage { to, content, .. } => {
            let permitted = manifest.inbound_actions != InboundActions::None;
            let summary = truncate_summary(&format!("→{}: {}", to, content), 200);
            ("FEDERATED_DM".to_string(), summary, permitted)
        }

        // Handshake messages — never inbox events
        FederationPayload::Hello { .. }
        | FederationPayload::Welcome { .. }
        | FederationPayload::NodeAnnounce(_)
        | FederationPayload::NodeQuery { .. }
        | FederationPayload::NodeResponse { .. }
        | FederationPayload::PeerListRequest { .. }
        | FederationPayload::PeerList { .. }
        | FederationPayload::Ping { .. }
        | FederationPayload::Pong { .. }
        | FederationPayload::Goodbye { .. }
        | FederationPayload::MessageAck { .. }
        | FederationPayload::Error { .. }
        | FederationPayload::RouteRequest { .. }
        | FederationPayload::RouteResponse { .. }
        | FederationPayload::Relay { .. } => {
            ("FEDERATED_CONTROL".to_string(), String::new(), false)
        }

        // Sync/negotiation — internal protocol, never exposed to AI context
        FederationPayload::SharePreferences(_)
        | FederationPayload::NegotiationComplete { .. }
        | FederationPayload::DataRequest { .. }
        | FederationPayload::DataResponse { .. }
        | FederationPayload::SyncVector { .. }
        | FederationPayload::PresenceBatch { .. }
        | FederationPayload::PresenceQuery { .. } => {
            ("FEDERATED_SYNC".to_string(), String::new(), false)
        }

        // Event replication — relayed teamengram events from a peer's event log
        FederationPayload::EventRelay { event_type, source_ai, origin_seq, .. } => {
            let permitted = manifest.inbound_actions != InboundActions::None;
            let category = teamengram::event::event_type::category(*event_type);
            let summary = format!(
                "Relayed {} event (seq {}) from {}",
                category, origin_seq, source_ai,
            );
            ("FEDERATED_EVENT_RELAY".to_string(), summary, permitted)
        }
    }
}

/// Truncate a summary to at most `max_chars` characters, appending `…` if cut.
fn truncate_summary(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}…", truncated)
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_micros() as u64
}

fn current_hlc(now_us: u64) -> HlcTimestamp {
    HlcTimestamp {
        physical_time_us: now_us,
        counter: 0,
        node_id: 0,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::TeambookIdentity;
    use crate::messages::FederationMessage;
    use crate::{
        AiRegistry, AiResolution, ConnectionMode, BroadcastVisibility, ExposureConfig,
        EventPushRequest, PresencePushRequest,
    };
    use crate::messages::FederatedPresence;
    use tempfile::TempDir;

    // --- Helpers -----------------------------------------------------------

    fn open_writer(dir: &TempDir) -> InboxWriter {
        let path = dir.path().join("inbox.jsonl");
        InboxWriter::open(path).expect("open writer")
    }

    fn make_registry() -> AiRegistry {
        AiRegistry::new("a".repeat(64), "a3f7c2d1".to_string(), "TestNode".to_string())
    }

    fn closed_manifest() -> PermissionManifest {
        PermissionManifest::default() // connection_mode = Off, all expose = false
    }

    fn open_broadcasts_manifest() -> PermissionManifest {
        PermissionManifest {
            connection_mode: ConnectionMode::ConnectCode,
            inbound_actions: InboundActions::Open,
            expose: ExposureConfig {
                presence: true,
                broadcasts: BroadcastVisibility::All,
                task_complete: true,
                ..ExposureConfig::default()
            },
            channels: vec![],
        }
    }

    fn now_hlc() -> HlcTimestamp {
        current_hlc(now_us())
    }

    fn stale_hlc() -> HlcTimestamp {
        // 2 minutes in the past — exceeds the 60s drift bound
        let stale_us = now_us().saturating_sub(120 * 1_000_000);
        HlcTimestamp { physical_time_us: stale_us, counter: 0, node_id: 0 }
    }

    // Build a valid SignedEvent for a Broadcast payload.
    //
    // Uses TeambookIdentity::generate() (pub(crate)) for the envelope — no IO.
    // A separate SigningKey signs the inner FederationMessage (the two keys
    // are orthogonal: one is the AI's key, one is the Teambook identity key).
    fn make_signed_broadcast(channel: &str, content: &str) -> crate::SignedEvent {
        use ed25519_dalek::SigningKey;
        use rand::rngs::OsRng;

        let identity = TeambookIdentity::generate();
        let msg_key = SigningKey::generate(&mut OsRng);

        let msg = FederationMessage::new(
            &identity.short_id(),
            FederationPayload::Broadcast {
                channel: channel.to_string(),
                content: content.to_string(),
            },
            &msg_key,
        );
        let event_bytes = msg.to_bytes().expect("serialize message");
        crate::SignedEvent::sign(event_bytes, &identity)
    }

    fn make_state(manifest: PermissionManifest, writer: InboxWriter) -> InboxState {
        InboxState::new(manifest, make_registry(), writer)
    }

    // Read all events from the inbox JSONL in the tempdir
    fn read_inbox(dir: &TempDir) -> Vec<FederationInboxEvent> {
        let path = dir.path().join("inbox.jsonl");
        if !path.exists() {
            return Vec::new();
        }
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        content
            .lines()
            .filter(|l| !l.is_empty())
            .filter_map(|l| serde_json::from_str::<FederationInboxEvent>(l).ok())
            .collect()
    }

    // --- PDU Tests ---------------------------------------------------------

    #[test]
    fn test_push_manifest_closed_all_rejected() {
        let dir = TempDir::new().unwrap();
        let state = make_state(closed_manifest(), open_writer(&dir));

        let signed = make_signed_broadcast("general", "hello");
        let request = EventPushRequest {
            events: vec![signed],
            sender_hlc: now_hlc(),
            sender_head_seq: 1,
        };

        let response = process_push_request(&state, request);

        assert_eq!(response.accepted, 0);
        assert_eq!(response.rejected, 1);
        assert_eq!(response.errors[0].reason, SyncRejectReason::NotPermittedByManifest);
        assert!(read_inbox(&dir).is_empty());
    }

    #[test]
    fn test_push_excessive_drift_rejected() {
        let dir = TempDir::new().unwrap();
        let state = make_state(open_broadcasts_manifest(), open_writer(&dir));

        let signed = make_signed_broadcast("general", "hello");
        let request = EventPushRequest {
            events: vec![signed],
            sender_hlc: stale_hlc(), // 2 minutes old — exceeds 60s bound
            sender_head_seq: 1,
        };

        let response = process_push_request(&state, request);

        assert_eq!(response.accepted, 0);
        assert_eq!(response.rejected, 1);
        assert_eq!(response.errors[0].reason, SyncRejectReason::ExcessiveDrift);
        assert!(read_inbox(&dir).is_empty());
    }

    #[test]
    fn test_push_malformed_event_bytes_rejected() {
        let dir = TempDir::new().unwrap();
        let state = make_state(open_broadcasts_manifest(), open_writer(&dir));

        // A SignedEvent with valid signature but garbage CBOR payload
        let identity = TeambookIdentity::generate();
        let garbage: Vec<u8> = vec![0xFF, 0xFE, 0xFD]; // not valid CBOR FederationMessage
        let signed = crate::SignedEvent::sign(garbage, &identity);

        let request = EventPushRequest {
            events: vec![signed],
            sender_hlc: now_hlc(),
            sender_head_seq: 1,
        };

        let response = process_push_request(&state, request);

        assert_eq!(response.accepted, 0);
        assert_eq!(response.rejected, 1);
        assert_eq!(response.errors[0].reason, SyncRejectReason::MalformedEvent);
    }

    #[test]
    fn test_push_broadcast_accepted_and_written() {
        let dir = TempDir::new().unwrap();
        let state = make_state(open_broadcasts_manifest(), open_writer(&dir));

        let signed = make_signed_broadcast("cross-team", "deployment complete");
        let content_id = signed.content_id.clone();

        let request = EventPushRequest {
            events: vec![signed],
            sender_hlc: now_hlc(),
            sender_head_seq: 1,
        };

        let response = process_push_request(&state, request);

        assert_eq!(response.accepted, 1);
        assert_eq!(response.rejected, 0);
        assert!(response.errors.is_empty());

        let events = read_inbox(&dir);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, content_id);
        assert_eq!(events[0].event_type, "FEDERATED_BROADCAST");
        assert!(events[0].summary.contains("cross-team"));
        assert!(events[0].summary.contains("deployment complete"));
    }

    #[test]
    fn test_push_broadcast_not_permitted_by_manifest() {
        let dir = TempDir::new().unwrap();
        // Manifest accepts inbound but doesn't expose broadcasts
        let mut manifest = open_broadcasts_manifest();
        manifest.expose.broadcasts = BroadcastVisibility::None;
        let state = make_state(manifest, open_writer(&dir));

        let signed = make_signed_broadcast("general", "hello");
        let request = EventPushRequest {
            events: vec![signed],
            sender_hlc: now_hlc(),
            sender_head_seq: 1,
        };

        let response = process_push_request(&state, request);

        assert_eq!(response.accepted, 0);
        assert_eq!(response.rejected, 1);
        assert_eq!(response.errors[0].reason, SyncRejectReason::NotPermittedByManifest);
    }

    #[test]
    fn test_push_tampered_event_rejected() {
        let dir = TempDir::new().unwrap();
        let state = make_state(open_broadcasts_manifest(), open_writer(&dir));

        let mut signed = make_signed_broadcast("general", "authentic");
        // Tamper with the event bytes AFTER signing — signature will fail
        if let Some(b) = signed.event_bytes.first_mut() {
            *b = b.wrapping_add(1);
        }

        let request = EventPushRequest {
            events: vec![signed],
            sender_hlc: now_hlc(),
            sender_head_seq: 1,
        };

        let response = process_push_request(&state, request);

        assert_eq!(response.accepted, 0);
        assert_eq!(response.rejected, 1);
        // Either ContentHashMismatch or InvalidSignature — both are correct
        assert!(matches!(
            response.errors[0].reason,
            SyncRejectReason::ContentHashMismatch | SyncRejectReason::InvalidSignature
        ));
    }

    #[test]
    fn test_push_mixed_batch() {
        let dir = TempDir::new().unwrap();
        let state = make_state(open_broadcasts_manifest(), open_writer(&dir));

        let valid = make_signed_broadcast("general", "valid event");

        // Stale-signed event (valid sig but wrong content_id — tampered)
        let identity = TeambookIdentity::generate();
        let mut tampered = crate::SignedEvent::sign(b"garbage".to_vec(), &identity);
        tampered.event_bytes = vec![0x00]; // mismatch content_id vs bytes

        let request = EventPushRequest {
            events: vec![valid, tampered],
            sender_hlc: now_hlc(),
            sender_head_seq: 2,
        };

        let response = process_push_request(&state, request);

        assert_eq!(response.accepted, 1);
        assert_eq!(response.rejected, 1);
        assert_eq!(read_inbox(&dir).len(), 1);
    }

    // --- EDU Tests ---------------------------------------------------------

    fn make_presence(ai_id: &str, status: &str, activity: Option<&str>) -> FederatedPresence {
        use ed25519_dalek::SigningKey;
        use rand::rngs::OsRng;
        let key = SigningKey::generate(&mut OsRng);
        FederatedPresence::new(ai_id, "node-test", status, activity.map(str::to_string), &key)
    }

    #[tokio::test]
    async fn test_presence_manifest_off_nothing_written() {
        let dir = TempDir::new().unwrap();
        let state = make_state(closed_manifest(), open_writer(&dir));

        let request = PresencePushRequest {
            presences: vec![make_presence("alpha-001", "active", Some("working on Step 9"))],
            sender_short_id: "b4e8a1f2".to_string(),
        };

        process_presence_request(&state, request).await;

        assert!(read_inbox(&dir).is_empty());
    }

    #[tokio::test]
    async fn test_presence_written_and_registry_updated() {
        let dir = TempDir::new().unwrap();
        let state = make_state(open_broadcasts_manifest(), open_writer(&dir));

        let request = PresencePushRequest {
            presences: vec![make_presence("beta-002", "standby", None)],
            sender_short_id: "c1d2e3f4".to_string(),
        };

        process_presence_request(&state, request).await;

        let events = read_inbox(&dir);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "FEDERATED_PRESENCE");
        assert_eq!(events[0].source_ai, Some("beta-002".to_string()));
        assert_eq!(events[0].source_teambook, "c1d2e3f4");
        assert!(events[0].summary.contains("beta-002"));
        assert!(events[0].summary.contains("standby"));

        // Registry updated
        let resolution = state.registry.resolve("beta-002").await;
        assert!(matches!(resolution, AiResolution::Remote { .. }));
    }

    #[tokio::test]
    async fn test_presence_with_activity_in_summary() {
        let dir = TempDir::new().unwrap();
        let state = make_state(open_broadcasts_manifest(), open_writer(&dir));

        let request = PresencePushRequest {
            presences: vec![make_presence("alpha-001", "active", Some("building Step 9 Gateway"))],
            sender_short_id: "a3f7c2d1".to_string(),
        };

        process_presence_request(&state, request).await;

        let events = read_inbox(&dir);
        assert_eq!(events.len(), 1);
        assert!(events[0].summary.contains("building Step 9 Gateway"));
    }

    #[tokio::test]
    async fn test_presence_multiple_ais() {
        let dir = TempDir::new().unwrap();
        let state = make_state(open_broadcasts_manifest(), open_writer(&dir));

        let request = PresencePushRequest {
            presences: vec![
                make_presence("alpha-001", "active", None),
                make_presence("delta-004", "standby", Some("presence work")),
                make_presence("beta-002", "idle", None),
            ],
            sender_short_id: "b4e8a1f2".to_string(),
        };

        process_presence_request(&state, request).await;

        let events = read_inbox(&dir);
        assert_eq!(events.len(), 3);

        let ai_ids: Vec<&str> = events.iter()
            .filter_map(|e| e.source_ai.as_deref())
            .collect();
        assert!(ai_ids.contains(&"alpha-001"));
        assert!(ai_ids.contains(&"delta-004"));
        assert!(ai_ids.contains(&"beta-002"));
    }

    // --- Classify payload tests -------------------------------------------

    #[test]
    fn test_classify_presence_permitted() {
        let mut manifest = open_broadcasts_manifest();
        manifest.expose.presence = true;
        let p = make_presence("alpha-001", "active", Some("testing"));
        let (et, summary, permitted) = classify_payload(
            &manifest,
            &FederationPayload::PresenceUpdate(p),
        );
        assert_eq!(et, "FEDERATED_PRESENCE");
        assert!(summary.contains("alpha-001"));
        assert!(permitted);
    }

    #[test]
    fn test_classify_control_never_permitted() {
        let manifest = open_broadcasts_manifest();
        let (_, _, permitted) = classify_payload(
            &manifest,
            &FederationPayload::Ping { timestamp: 0 },
        );
        assert!(!permitted);
    }

    #[test]
    fn test_truncate_summary() {
        let short = "hello";
        assert_eq!(truncate_summary(short, 10), "hello");

        let long = "a".repeat(300);
        let result = truncate_summary(&long, 200);
        assert!(result.ends_with('…'));
        assert!(result.chars().count() <= 202); // 200 chars + ellipsis
    }
}
