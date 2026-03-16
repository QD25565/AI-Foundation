//! Cursor-Tracked Replication for Federation Event Sync
//!
//! Each Teambook maintains a **per-peer cursor** that tracks the remote peer's
//! event log position. This enables efficient incremental sync:
//!
//! - **Push:** After delivering events, advance the peer's cursor to the
//!   sequence number they acknowledged.
//! - **Pull (reconnect):** On reconnect, request events since the cursor's
//!   last-seen sequence. Bounded, event-driven, never polled.
//! - **Dedup:** Content-addressed via `content_id` (SHA-256). Same event
//!   bytes from two paths = same hash = idempotent.
//!
//! # Design Principles
//!
//! - Each Teambook's event log is **authoritative for that Teambook**.
//!   There is no shared log, no CRDT merge — just push/pull of signed events.
//! - Cursors carry **source tags** — you always know which Teambook an event
//!   originated from.
//! - Content IDs provide idempotent dedup regardless of delivery order.
//! - HLC timestamps provide causal ordering; sequence numbers provide
//!   per-node ordering. Both are tracked.
//!
//! # Persistence
//!
//! Cursor state is persisted to `~/.ai-foundation/federation/cursors.toml`.
//! On startup, load cursors. On clean shutdown, flush. Crash recovery: worst
//! case is re-delivering events the peer already has (idempotent via content_id).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::hlc::HlcTimestamp;
use crate::sync::{EventPullRequest, EventPullResponse, EventPushResponse};
use crate::{FederationError, Result, SignedEvent};

// ---------------------------------------------------------------------------
// Replication Cursor — per-peer sync state
// ---------------------------------------------------------------------------

/// Tracks how far we've synced with a specific remote Teambook.
///
/// Each cursor represents one direction of a peer relationship:
/// - **Outbound cursor:** "What's the latest sequence from OUR log that
///   this peer has acknowledged receiving?"
/// - **Inbound cursor:** "What's the latest sequence from THEIR log that
///   we've received?"
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationCursor {
    /// Remote peer's public key (hex) — the cursor identity
    pub peer_pubkey_hex: String,

    /// Human-readable peer name (for logging/debugging)
    #[serde(default)]
    pub peer_name: String,

    /// Their highest sequence number we've seen (inbound cursor)
    ///
    /// Used to build `EventPullRequest.since_seq` on reconnect.
    pub inbound_head_seq: u64,

    /// Their HLC at the time of last received push
    #[serde(default)]
    pub inbound_hlc: Option<HlcTimestamp>,

    /// Our highest sequence number they've acknowledged (outbound cursor)
    ///
    /// Used to determine which local events still need pushing.
    pub outbound_acked_seq: u64,

    /// Content IDs of recently received events (sliding window for dedup)
    ///
    /// Keeps the last N content_ids to catch duplicates that arrive
    /// via different paths (multi-hop federation). Bounded to prevent
    /// unbounded growth.
    #[serde(default)]
    pub recent_content_ids: HashSet<String>,

    /// Unix timestamp (seconds) of last successful sync in either direction
    pub last_sync_at: u64,

    /// Number of consecutive sync failures (for backoff decisions)
    #[serde(default)]
    pub consecutive_failures: u32,
}

/// Maximum number of content IDs to retain per cursor for dedup.
/// At ~64 bytes per SHA-256 hex string, 10K entries ≈ 640KB per peer.
/// Well within reason for active federation peers.
const MAX_RECENT_CONTENT_IDS: usize = 10_000;

impl ReplicationCursor {
    /// Create a fresh cursor for a newly discovered peer.
    pub fn new(peer_pubkey_hex: &str, peer_name: &str) -> Self {
        Self {
            peer_pubkey_hex: peer_pubkey_hex.to_string(),
            peer_name: peer_name.to_string(),
            inbound_head_seq: 0,
            inbound_hlc: None,
            outbound_acked_seq: 0,
            recent_content_ids: HashSet::new(),
            last_sync_at: 0,
            consecutive_failures: 0,
        }
    }

    /// Check if we've already seen this content_id from any source.
    pub fn has_content_id(&self, content_id: &str) -> bool {
        self.recent_content_ids.contains(content_id)
    }

    /// Record a received content_id. Trims the set if over limit.
    pub fn record_content_id(&mut self, content_id: String) {
        self.recent_content_ids.insert(content_id);

        // Trim if over limit — remove arbitrary entries (HashSet has no ordering,
        // but that's fine — we just need bounded memory, not LRU precision)
        if self.recent_content_ids.len() > MAX_RECENT_CONTENT_IDS {
            let excess = self.recent_content_ids.len() - MAX_RECENT_CONTENT_IDS;
            let to_remove: Vec<String> = self
                .recent_content_ids
                .iter()
                .take(excess)
                .cloned()
                .collect();
            for id in to_remove {
                self.recent_content_ids.remove(&id);
            }
        }
    }

    /// Advance inbound cursor after successfully receiving events.
    ///
    /// `remote_head_seq` is the sender's head sequence from the push.
    /// `remote_hlc` is the sender's HLC at push time.
    pub fn advance_inbound(
        &mut self,
        remote_head_seq: u64,
        remote_hlc: HlcTimestamp,
        accepted_content_ids: &[String],
    ) {
        // Only advance forward — never go backward
        if remote_head_seq > self.inbound_head_seq {
            self.inbound_head_seq = remote_head_seq;
        }

        self.inbound_hlc = Some(remote_hlc);

        for id in accepted_content_ids {
            self.record_content_id(id.clone());
        }

        self.last_sync_at = now_secs();
        self.consecutive_failures = 0;
    }

    /// Advance outbound cursor after peer acknowledges our push.
    ///
    /// `acked_seq` is the sequence number the peer confirmed receiving up to.
    pub fn advance_outbound(&mut self, acked_seq: u64) {
        if acked_seq > self.outbound_acked_seq {
            self.outbound_acked_seq = acked_seq;
        }
        self.last_sync_at = now_secs();
        self.consecutive_failures = 0;
    }

    /// Record a sync failure for backoff tracking.
    pub fn record_failure(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
    }

    /// Suggested backoff duration based on consecutive failures.
    ///
    /// Exponential backoff: 1s, 2s, 4s, 8s, ... capped at 5 minutes.
    pub fn backoff_secs(&self) -> u64 {
        let base: u64 = 1;
        let max: u64 = 300; // 5 minutes
        let exp = base.saturating_mul(1u64.checked_shl(self.consecutive_failures as u32).unwrap_or(u64::MAX));
        exp.min(max)
    }

    /// How many events the peer is ahead of us (based on sequence gap).
    ///
    /// Returns 0 if we're caught up or if we have no information.
    pub fn inbound_lag(&self, remote_head_seq: u64) -> u64 {
        remote_head_seq.saturating_sub(self.inbound_head_seq)
    }

    /// How many of our events the peer hasn't acknowledged yet.
    pub fn outbound_lag(&self, our_head_seq: u64) -> u64 {
        our_head_seq.saturating_sub(self.outbound_acked_seq)
    }
}

// ---------------------------------------------------------------------------
// Cursor Store — persistence layer
// ---------------------------------------------------------------------------

/// Persistent storage for per-peer replication cursors.
///
/// Serialized as TOML to `~/.ai-foundation/federation/cursors.toml`.
/// Thread-safe: wrap in `Arc<Mutex<CursorStore>>` for concurrent access.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CursorStore {
    /// Per-peer cursors, keyed by peer pubkey hex
    #[serde(default)]
    pub cursors: HashMap<String, ReplicationCursor>,
}

impl CursorStore {
    /// Create an empty cursor store.
    pub fn new() -> Self {
        Self {
            cursors: HashMap::new(),
        }
    }

    /// Load cursor state from disk. Returns empty store if file doesn't exist.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::new());
        }

        let content = std::fs::read_to_string(path).map_err(|e| {
            FederationError::Internal(format!("Failed to read cursor store: {e}"))
        })?;

        toml::from_str(&content).map_err(|e| {
            FederationError::Internal(format!("Failed to parse cursor store: {e}"))
        })
    }

    /// Save cursor state to disk atomically (write-tmp + rename).
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self).map_err(|e| {
            FederationError::Internal(format!("Failed to serialize cursor store: {e}"))
        })?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                FederationError::Internal(format!("Failed to create cursor directory: {e}"))
            })?;
        }

        // Write to temp file then rename (atomic on most filesystems)
        let tmp_path = path.with_extension("toml.tmp");
        std::fs::write(&tmp_path, &content).map_err(|e| {
            FederationError::Internal(format!("Failed to write cursor store: {e}"))
        })?;
        std::fs::rename(&tmp_path, path).map_err(|e| {
            FederationError::Internal(format!("Failed to rename cursor store: {e}"))
        })?;

        debug!(path = %path.display(), peers = self.cursors.len(), "Cursor store saved");
        Ok(())
    }

    /// Default path for the cursor store.
    pub fn default_path() -> PathBuf {
        let base = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        base.join(".ai-foundation")
            .join("federation")
            .join("cursors.toml")
    }

    /// Get or create a cursor for a peer.
    pub fn get_or_create(&mut self, peer_pubkey_hex: &str, peer_name: &str) -> &mut ReplicationCursor {
        self.cursors
            .entry(peer_pubkey_hex.to_string())
            .or_insert_with(|| ReplicationCursor::new(peer_pubkey_hex, peer_name))
    }

    /// Get a cursor for a peer (read-only).
    pub fn get(&self, peer_pubkey_hex: &str) -> Option<&ReplicationCursor> {
        self.cursors.get(peer_pubkey_hex)
    }

    /// Remove a peer's cursor (e.g., when they're removed from the registry).
    pub fn remove(&mut self, peer_pubkey_hex: &str) -> Option<ReplicationCursor> {
        self.cursors.remove(peer_pubkey_hex)
    }

    /// Get all peers that have a non-zero inbound lag relative to `remote_head_seq`.
    pub fn peers_needing_catchup(&self) -> Vec<(&str, &ReplicationCursor)> {
        self.cursors
            .iter()
            .filter(|(_, c)| c.consecutive_failures == 0 || c.backoff_secs() == 0)
            .map(|(k, c)| (k.as_str(), c))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Replication Orchestrator
// ---------------------------------------------------------------------------

/// Orchestrates cursor-tracked replication between this Teambook and its peers.
///
/// This is the high-level sync coordinator that:
/// 1. Determines which events to push to each peer (based on outbound cursor)
/// 2. Builds EventPullRequests on reconnect (based on inbound cursor)
/// 3. Advances cursors on successful sync
/// 4. Handles dedup via content_id tracking
///
/// The orchestrator does NOT own the QUIC connections — it works with
/// `PeerSession` instances provided by the caller.
pub struct ReplicationOrchestrator {
    /// Per-peer cursor state
    cursors: CursorStore,

    /// Path to persist cursor state
    cursor_path: PathBuf,

    /// Our local event log head sequence
    local_head_seq: u64,

    /// Our Teambook's public key hex (for identifying ourselves)
    our_pubkey_hex: String,
}

impl ReplicationOrchestrator {
    /// Create or load a replication orchestrator.
    pub fn new(our_pubkey_hex: &str, cursor_path: Option<PathBuf>) -> Result<Self> {
        let path = cursor_path.unwrap_or_else(CursorStore::default_path);
        let cursors = CursorStore::load(&path)?;

        info!(
            peers = cursors.cursors.len(),
            path = %path.display(),
            "Replication orchestrator loaded"
        );

        Ok(Self {
            cursors,
            cursor_path: path,
            local_head_seq: 0,
            our_pubkey_hex: our_pubkey_hex.to_string(),
        })
    }

    /// Update our local head sequence (call when local events are written).
    pub fn set_local_head_seq(&mut self, seq: u64) {
        self.local_head_seq = seq;
    }

    /// Get our current local head sequence.
    pub fn local_head_seq(&self) -> u64 {
        self.local_head_seq
    }

    /// Flush cursor state to disk.
    pub fn flush(&self) -> Result<()> {
        self.cursors.save(&self.cursor_path)
    }

    // -------------------------------------------------------------------
    // Cursor access
    // -------------------------------------------------------------------

    /// Read-only access to a specific peer's cursor.
    pub fn cursor(&self, peer_pubkey_hex: &str) -> Option<&ReplicationCursor> {
        self.cursors.get(peer_pubkey_hex)
    }

    /// Mutable access to the cursor store (for advancing cursors on filtered events).
    pub fn cursors_mut(&mut self) -> &mut CursorStore {
        &mut self.cursors
    }

    // -------------------------------------------------------------------
    // Outbound: Determine what to push to a peer
    // -------------------------------------------------------------------

    /// Get the sequence number to start pushing from for a given peer.
    ///
    /// Returns the peer's `outbound_acked_seq` — events with sequence
    /// strictly greater than this value need to be pushed.
    pub fn outbound_since_seq(&self, peer_pubkey_hex: &str) -> u64 {
        self.cursors
            .get(peer_pubkey_hex)
            .map(|c| c.outbound_acked_seq)
            .unwrap_or(0)
    }

    /// How many of our events this peer hasn't seen yet.
    pub fn outbound_lag(&self, peer_pubkey_hex: &str) -> u64 {
        self.local_head_seq
            .saturating_sub(self.outbound_since_seq(peer_pubkey_hex))
    }

    /// Process the response from a successful push to a peer.
    ///
    /// Advances the outbound cursor based on the peer's acknowledged position.
    pub fn on_push_acked(
        &mut self,
        peer_pubkey_hex: &str,
        peer_name: &str,
        response: &EventPushResponse,
    ) {
        let cursor = self.cursors.get_or_create(peer_pubkey_hex, peer_name);

        // The peer tells us their head_seq after accepting our events.
        // But for outbound tracking, what matters is: they accepted N events
        // that we sent starting from outbound_acked_seq. So we advance by
        // the number accepted.
        //
        // More precisely: if we sent events [acked+1 .. acked+batch_size],
        // and they accepted `response.accepted` of them, we advance by that count.
        // Duplicates don't advance (they were already counted).
        let new_acked = cursor
            .outbound_acked_seq
            .saturating_add(response.accepted as u64);
        cursor.advance_outbound(new_acked);

        debug!(
            peer = peer_pubkey_hex,
            accepted = response.accepted,
            duplicates = response.duplicates,
            new_acked = new_acked,
            "Outbound cursor advanced"
        );
    }

    /// Record a push failure for backoff tracking.
    pub fn on_push_failed(&mut self, peer_pubkey_hex: &str, peer_name: &str) {
        let cursor = self.cursors.get_or_create(peer_pubkey_hex, peer_name);
        cursor.record_failure();
        warn!(
            peer = peer_pubkey_hex,
            failures = cursor.consecutive_failures,
            backoff_secs = cursor.backoff_secs(),
            "Push failed, backing off"
        );
    }

    // -------------------------------------------------------------------
    // Inbound: Process received events and advance cursor
    // -------------------------------------------------------------------

    /// Check if an event has already been seen (content-addressed dedup).
    ///
    /// Returns true if this content_id is in any peer's recent set.
    /// This is a fast pre-check before full verification.
    pub fn is_duplicate(&self, content_id: &str) -> bool {
        self.cursors
            .cursors
            .values()
            .any(|c| c.has_content_id(content_id))
    }

    /// Pre-filter a batch of events, removing duplicates we've already seen.
    ///
    /// Returns (new_events, duplicate_count).
    pub fn dedup_events<'a>(&self, events: &'a [SignedEvent]) -> (Vec<&'a SignedEvent>, usize) {
        let mut new_events = Vec::new();
        let mut dup_count = 0;

        for event in events {
            if self.is_duplicate(&event.content_id) {
                dup_count += 1;
            } else {
                new_events.push(event);
            }
        }

        (new_events, dup_count)
    }

    /// Process a successful inbound push from a peer.
    ///
    /// Call after events have been validated and written to the local log.
    /// Advances the inbound cursor and records content IDs for dedup.
    pub fn on_events_received(
        &mut self,
        peer_pubkey_hex: &str,
        peer_name: &str,
        sender_head_seq: u64,
        sender_hlc: HlcTimestamp,
        accepted_content_ids: Vec<String>,
    ) {
        let cursor = self.cursors.get_or_create(peer_pubkey_hex, peer_name);
        cursor.advance_inbound(sender_head_seq, sender_hlc, &accepted_content_ids);

        info!(
            peer = peer_pubkey_hex,
            inbound_head = cursor.inbound_head_seq,
            content_ids = accepted_content_ids.len(),
            "Inbound cursor advanced"
        );
    }

    // -------------------------------------------------------------------
    // Reconnect: Build catch-up pull request
    // -------------------------------------------------------------------

    /// Build an `EventPullRequest` to catch up with a peer after reconnect.
    ///
    /// Uses the inbound cursor's `inbound_head_seq` as the `since_seq`.
    /// Returns `None` if we have no cursor for this peer (first connection).
    pub fn build_catchup_pull(
        &self,
        peer_pubkey_hex: &str,
        limit: Option<usize>,
    ) -> EventPullRequest {
        let since_seq = self
            .cursors
            .get(peer_pubkey_hex)
            .map(|c| c.inbound_head_seq)
            .unwrap_or(0);

        EventPullRequest {
            since_seq,
            limit: limit.unwrap_or(100),
            requester_pubkey: self.our_pubkey_hex.clone(),
        }
    }

    /// Process a pull response (catch-up events received after reconnect).
    ///
    /// Returns the events that are genuinely new (not duplicates).
    pub fn process_pull_response<'a>(
        &self,
        response: &'a EventPullResponse,
    ) -> Vec<&'a SignedEvent> {
        response
            .events
            .iter()
            .filter(|e| !self.is_duplicate(&e.content_id))
            .collect()
    }

    /// Advance cursor after processing a pull response.
    pub fn on_pull_complete(
        &mut self,
        peer_pubkey_hex: &str,
        peer_name: &str,
        response: &EventPullResponse,
        accepted_content_ids: Vec<String>,
    ) {
        let cursor = self.cursors.get_or_create(peer_pubkey_hex, peer_name);

        cursor.advance_inbound(
            response.head_seq,
            response.sender_hlc,
            &accepted_content_ids,
        );

        if response.has_more {
            debug!(
                peer = peer_pubkey_hex,
                head_seq = response.head_seq,
                "Pull response has more events — another round needed"
            );
        }
    }

    // -------------------------------------------------------------------
    // Serve: Respond to pull requests from peers
    // -------------------------------------------------------------------

    /// Build an `EventPullResponse` for a peer requesting catch-up.
    ///
    /// `get_events_since` is a callback that retrieves signed events from the
    /// local event log with sequence > since_seq, up to `limit`.
    /// The callback returns `(events, has_more)`.
    pub fn serve_pull_request(
        &self,
        request: &EventPullRequest,
        sender_hlc: HlcTimestamp,
        get_events_since: impl FnOnce(u64, usize) -> (Vec<SignedEvent>, bool),
    ) -> EventPullResponse {
        let (events, has_more) = get_events_since(request.since_seq, request.limit);

        EventPullResponse {
            events,
            head_seq: self.local_head_seq,
            has_more,
            sender_hlc,
        }
    }

    // -------------------------------------------------------------------
    // Status / Diagnostics
    // -------------------------------------------------------------------

    /// Get a summary of all peer cursor states.
    pub fn peer_statuses(&self) -> Vec<PeerSyncStatus> {
        self.cursors
            .cursors
            .values()
            .map(|c| PeerSyncStatus {
                peer_pubkey_hex: c.peer_pubkey_hex.clone(),
                peer_name: c.peer_name.clone(),
                inbound_head_seq: c.inbound_head_seq,
                outbound_acked_seq: c.outbound_acked_seq,
                inbound_lag: 0, // would need remote head to compute
                outbound_lag: self.local_head_seq.saturating_sub(c.outbound_acked_seq),
                last_sync_at: c.last_sync_at,
                consecutive_failures: c.consecutive_failures,
                dedup_cache_size: c.recent_content_ids.len(),
            })
            .collect()
    }
}

/// Summary of sync state with a single peer.
#[derive(Debug, Clone, Serialize)]
pub struct PeerSyncStatus {
    pub peer_pubkey_hex: String,
    pub peer_name: String,
    pub inbound_head_seq: u64,
    pub outbound_acked_seq: u64,
    pub inbound_lag: u64,
    pub outbound_lag: u64,
    pub last_sync_at: u64,
    pub consecutive_failures: u32,
    pub dedup_cache_size: usize,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_hlc(time_us: u64) -> HlcTimestamp {
        HlcTimestamp {
            physical_time_us: time_us,
            counter: 0,
            node_id: 1,
        }
    }

    #[test]
    fn test_cursor_new() {
        let cursor = ReplicationCursor::new("abcd1234", "test-peer");
        assert_eq!(cursor.peer_pubkey_hex, "abcd1234");
        assert_eq!(cursor.inbound_head_seq, 0);
        assert_eq!(cursor.outbound_acked_seq, 0);
        assert!(cursor.recent_content_ids.is_empty());
        assert_eq!(cursor.consecutive_failures, 0);
    }

    #[test]
    fn test_cursor_advance_inbound() {
        let mut cursor = ReplicationCursor::new("abcd1234", "test-peer");

        cursor.advance_inbound(
            42,
            test_hlc(1000),
            &["hash_a".to_string(), "hash_b".to_string()],
        );

        assert_eq!(cursor.inbound_head_seq, 42);
        assert!(cursor.has_content_id("hash_a"));
        assert!(cursor.has_content_id("hash_b"));
        assert!(!cursor.has_content_id("hash_c"));
        assert!(cursor.last_sync_at > 0);
    }

    #[test]
    fn test_cursor_advance_inbound_never_goes_backward() {
        let mut cursor = ReplicationCursor::new("abcd1234", "test-peer");

        cursor.advance_inbound(100, test_hlc(1000), &[]);
        assert_eq!(cursor.inbound_head_seq, 100);

        // Attempt to go backward — should be ignored
        cursor.advance_inbound(50, test_hlc(2000), &[]);
        assert_eq!(cursor.inbound_head_seq, 100);
    }

    #[test]
    fn test_cursor_advance_outbound() {
        let mut cursor = ReplicationCursor::new("abcd1234", "test-peer");

        cursor.advance_outbound(25);
        assert_eq!(cursor.outbound_acked_seq, 25);

        cursor.advance_outbound(50);
        assert_eq!(cursor.outbound_acked_seq, 50);

        // Never backward
        cursor.advance_outbound(30);
        assert_eq!(cursor.outbound_acked_seq, 50);
    }

    #[test]
    fn test_cursor_backoff() {
        let mut cursor = ReplicationCursor::new("abcd1234", "test-peer");

        assert_eq!(cursor.backoff_secs(), 1);

        cursor.record_failure();
        assert_eq!(cursor.consecutive_failures, 1);
        assert_eq!(cursor.backoff_secs(), 2);

        cursor.record_failure();
        assert_eq!(cursor.backoff_secs(), 4);

        cursor.record_failure();
        assert_eq!(cursor.backoff_secs(), 8);

        // Reset on success
        cursor.advance_outbound(10);
        assert_eq!(cursor.consecutive_failures, 0);
        assert_eq!(cursor.backoff_secs(), 1);
    }

    #[test]
    fn test_cursor_backoff_capped() {
        let mut cursor = ReplicationCursor::new("abcd1234", "test-peer");

        // Simulate many failures
        for _ in 0..20 {
            cursor.record_failure();
        }

        // Should be capped at 300 seconds (5 minutes)
        assert!(cursor.backoff_secs() <= 300);
    }

    #[test]
    fn test_cursor_lag() {
        let cursor = ReplicationCursor::new("abcd1234", "test-peer");

        assert_eq!(cursor.inbound_lag(100), 100);
        assert_eq!(cursor.outbound_lag(50), 50);
    }

    #[test]
    fn test_cursor_content_id_dedup() {
        let mut cursor = ReplicationCursor::new("abcd1234", "test-peer");

        assert!(!cursor.has_content_id("hash_1"));

        cursor.record_content_id("hash_1".to_string());
        assert!(cursor.has_content_id("hash_1"));

        // Recording same ID again is idempotent
        cursor.record_content_id("hash_1".to_string());
        assert!(cursor.has_content_id("hash_1"));
    }

    #[test]
    fn test_cursor_content_id_trim() {
        let mut cursor = ReplicationCursor::new("abcd1234", "test-peer");

        // Add more than MAX_RECENT_CONTENT_IDS
        for i in 0..MAX_RECENT_CONTENT_IDS + 500 {
            cursor.record_content_id(format!("hash_{i}"));
        }

        // Should be trimmed to MAX_RECENT_CONTENT_IDS
        assert!(cursor.recent_content_ids.len() <= MAX_RECENT_CONTENT_IDS);
    }

    #[test]
    fn test_cursor_store_new_is_empty() {
        let store = CursorStore::new();
        assert!(store.cursors.is_empty());
    }

    #[test]
    fn test_cursor_store_get_or_create() {
        let mut store = CursorStore::new();

        let cursor = store.get_or_create("abc", "peer-a");
        assert_eq!(cursor.peer_pubkey_hex, "abc");
        assert_eq!(cursor.inbound_head_seq, 0);

        // Modify
        cursor.advance_outbound(42);

        // Get again — should be same cursor, not a new one
        let cursor2 = store.get_or_create("abc", "peer-a");
        assert_eq!(cursor2.outbound_acked_seq, 42);
    }

    #[test]
    fn test_cursor_store_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cursors.toml");

        // Create and populate
        let mut store = CursorStore::new();
        let cursor = store.get_or_create("aabb", "peer-1");
        cursor.advance_inbound(100, test_hlc(5000), &["hash_x".to_string()]);
        cursor.advance_outbound(75);

        // Save
        store.save(&path).unwrap();

        // Load
        let loaded = CursorStore::load(&path).unwrap();
        let loaded_cursor = loaded.get("aabb").unwrap();
        assert_eq!(loaded_cursor.inbound_head_seq, 100);
        assert_eq!(loaded_cursor.outbound_acked_seq, 75);
        assert!(loaded_cursor.has_content_id("hash_x"));
    }

    #[test]
    fn test_cursor_store_load_nonexistent_returns_empty() {
        let path = Path::new("/tmp/nonexistent_cursors_test_12345.toml");
        let store = CursorStore::load(path).unwrap();
        assert!(store.cursors.is_empty());
    }

    #[test]
    fn test_orchestrator_outbound_lag() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cursors.toml");

        let mut orch = ReplicationOrchestrator::new("our_key", Some(path)).unwrap();
        orch.set_local_head_seq(100);

        // Unknown peer — full lag
        assert_eq!(orch.outbound_lag("unknown_peer"), 100);

        // Push acked — reduces lag
        orch.on_push_acked("peer_a", "Peer A", &EventPushResponse {
            accepted: 30,
            duplicates: 0,
            rejected: 0,
            errors: vec![],
            receiver_hlc: test_hlc(1000),
            receiver_head_seq: 30,
        });
        assert_eq!(orch.outbound_lag("peer_a"), 70);
    }

    #[test]
    fn test_orchestrator_build_catchup_pull() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cursors.toml");

        let mut orch = ReplicationOrchestrator::new("our_key", Some(path)).unwrap();

        // No cursor yet — pull from 0
        let pull = orch.build_catchup_pull("peer_a", None);
        assert_eq!(pull.since_seq, 0);
        assert_eq!(pull.limit, 100);

        // After receiving events, cursor advances
        orch.on_events_received("peer_a", "Peer A", 50, test_hlc(1000), vec!["h1".into()]);

        let pull2 = orch.build_catchup_pull("peer_a", Some(50));
        assert_eq!(pull2.since_seq, 50);
        assert_eq!(pull2.limit, 50);
    }

    #[test]
    fn test_orchestrator_dedup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cursors.toml");

        let mut orch = ReplicationOrchestrator::new("our_key", Some(path)).unwrap();

        // Receive event from peer A
        orch.on_events_received("peer_a", "Peer A", 10, test_hlc(1000), vec!["hash_1".into()]);

        // Same content_id should be detected as duplicate
        assert!(orch.is_duplicate("hash_1"));
        assert!(!orch.is_duplicate("hash_2"));
    }

    #[test]
    fn test_orchestrator_dedup_events_batch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cursors.toml");

        let mut orch = ReplicationOrchestrator::new("our_key", Some(path)).unwrap();

        // Pre-seed some content IDs
        orch.on_events_received(
            "peer_a",
            "Peer A",
            10,
            test_hlc(1000),
            vec!["dup_1".into(), "dup_2".into()],
        );

        // Create a batch with mix of new and duplicate events
        let events = vec![
            SignedEvent {
                event_bytes: vec![1],
                origin_pubkey: "key".into(),
                signature: "sig".into(),
                content_id: "dup_1".into(), // duplicate
            },
            SignedEvent {
                event_bytes: vec![2],
                origin_pubkey: "key".into(),
                signature: "sig".into(),
                content_id: "new_1".into(), // new
            },
            SignedEvent {
                event_bytes: vec![3],
                origin_pubkey: "key".into(),
                signature: "sig".into(),
                content_id: "dup_2".into(), // duplicate
            },
        ];

        let (new_events, dup_count) = orch.dedup_events(&events);
        assert_eq!(dup_count, 2);
        assert_eq!(new_events.len(), 1);
        assert_eq!(new_events[0].content_id, "new_1");
    }

    #[test]
    fn test_orchestrator_push_failure_backoff() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cursors.toml");

        let mut orch = ReplicationOrchestrator::new("our_key", Some(path)).unwrap();

        orch.on_push_failed("peer_a", "Peer A");
        orch.on_push_failed("peer_a", "Peer A");
        orch.on_push_failed("peer_a", "Peer A");

        let cursor = orch.cursors.get("peer_a").unwrap();
        assert_eq!(cursor.consecutive_failures, 3);
        assert_eq!(cursor.backoff_secs(), 8); // 2^3

        // Success resets backoff
        orch.on_push_acked("peer_a", "Peer A", &EventPushResponse {
            accepted: 1,
            duplicates: 0,
            rejected: 0,
            errors: vec![],
            receiver_hlc: test_hlc(1000),
            receiver_head_seq: 1,
        });

        let cursor = orch.cursors.get("peer_a").unwrap();
        assert_eq!(cursor.consecutive_failures, 0);
    }

    #[test]
    fn test_orchestrator_serve_pull() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cursors.toml");

        let orch = ReplicationOrchestrator::new("our_key", Some(path)).unwrap();

        let request = EventPullRequest {
            since_seq: 10,
            limit: 50,
            requester_pubkey: "peer_a_key".into(),
        };

        let hlc = test_hlc(5000);

        let response = orch.serve_pull_request(&request, hlc, |since, limit| {
            // Simulate returning 3 events
            assert_eq!(since, 10);
            assert_eq!(limit, 50);
            let events = vec![
                SignedEvent {
                    event_bytes: vec![1],
                    origin_pubkey: "key".into(),
                    signature: "sig".into(),
                    content_id: "e1".into(),
                },
                SignedEvent {
                    event_bytes: vec![2],
                    origin_pubkey: "key".into(),
                    signature: "sig".into(),
                    content_id: "e2".into(),
                },
            ];
            (events, false)
        });

        assert_eq!(response.events.len(), 2);
        assert!(!response.has_more);
    }

    #[test]
    fn test_orchestrator_peer_statuses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cursors.toml");

        let mut orch = ReplicationOrchestrator::new("our_key", Some(path)).unwrap();
        orch.set_local_head_seq(100);

        orch.on_events_received("peer_a", "Peer A", 50, test_hlc(1000), vec![]);
        orch.on_push_acked("peer_a", "Peer A", &EventPushResponse {
            accepted: 30,
            duplicates: 0,
            rejected: 0,
            errors: vec![],
            receiver_hlc: test_hlc(2000),
            receiver_head_seq: 30,
        });

        let statuses = orch.peer_statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].peer_name, "Peer A");
        assert_eq!(statuses[0].inbound_head_seq, 50);
        assert_eq!(statuses[0].outbound_acked_seq, 30);
        assert_eq!(statuses[0].outbound_lag, 70); // 100 - 30
    }
}
