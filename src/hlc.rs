//! Hybrid Logical Clock — Causal Ordering Without Synchronized Clocks
//!
//! Each federation event carries an HLC timestamp: (physical_time_us, counter, node_id).
//! This provides "happens-before" guarantees across Teambooks without NTP:
//!
//! - If event B is causally dependent on event A, then HLC(B) > HLC(A)
//! - Events from the same node are strictly ordered
//! - Concurrent events from different nodes are ordered deterministically (by node_id)
//!
//! Drift protection: reject timestamps more than MAX_DRIFT_US ahead of local time.
//! This bounds the damage a malicious or misconfigured clock can cause.
//!
//! Reference: "Logical Physical Clocks and Consistent Snapshots in Globally
//! Distributed Databases" — Kulkarni et al., 2014

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Maximum allowed clock drift: 60 seconds in microseconds.
/// Events with physical_time more than this far ahead of local time are rejected.
/// This prevents a node with a fast clock from dominating ordering.
const MAX_DRIFT_US: u64 = 60_000_000;

/// A Hybrid Logical Clock timestamp.
///
/// Ordering: physical_time_us > counter > node_id (lexicographic).
/// This ensures total ordering across all events in the federation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HlcTimestamp {
    /// Physical time in microseconds since UNIX epoch.
    /// Advances with wall clock, but never goes backward.
    pub physical_time_us: u64,

    /// Logical counter — incremented when physical time hasn't advanced.
    /// Guarantees ordering of rapid-fire events within the same microsecond.
    pub counter: u32,

    /// Node identifier — first 8 bytes of the Teambook's Ed25519 public key.
    /// Breaks ties when physical_time and counter are identical across nodes.
    pub node_id: u64,
}

impl HlcTimestamp {
    /// Create a timestamp at the UNIX epoch (used as initial state).
    pub fn zero(node_id: u64) -> Self {
        Self {
            physical_time_us: 0,
            counter: 0,
            node_id,
        }
    }

    /// Serialize to 20 bytes: [physical_time:8][counter:4][node_id:8], little-endian.
    pub fn to_bytes(&self) -> [u8; 20] {
        let mut buf = [0u8; 20];
        buf[0..8].copy_from_slice(&self.physical_time_us.to_le_bytes());
        buf[8..12].copy_from_slice(&self.counter.to_le_bytes());
        buf[12..20].copy_from_slice(&self.node_id.to_le_bytes());
        buf
    }

    /// Deserialize from 20 bytes.
    pub fn from_bytes(bytes: &[u8; 20]) -> Self {
        Self {
            physical_time_us: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            counter: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            node_id: u64::from_le_bytes(bytes[12..20].try_into().unwrap()),
        }
    }
}

impl PartialOrd for HlcTimestamp {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HlcTimestamp {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.physical_time_us
            .cmp(&other.physical_time_us)
            .then(self.counter.cmp(&other.counter))
            .then(self.node_id.cmp(&other.node_id))
    }
}

impl std::fmt::Display for HlcTimestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{:016x}",
            self.physical_time_us, self.counter, self.node_id
        )
    }
}

// ---------------------------------------------------------------------------
// HLC Clock Instance
// ---------------------------------------------------------------------------

/// A Hybrid Logical Clock for a single Teambook node.
///
/// Thread-safe via internal Mutex. The clock guarantees:
/// - Monotonically increasing timestamps (never goes backward)
/// - Causal ordering when `receive()` is called with remote timestamps
/// - Bounded drift rejection for untrusted remote clocks
pub struct HybridClock {
    state: Mutex<HlcTimestamp>,
}

impl HybridClock {
    /// Create a new HLC for a node.
    ///
    /// `node_id` should be derived from the Teambook's Ed25519 public key
    /// (first 8 bytes interpreted as u64). This ensures globally unique node IDs
    /// without coordination.
    pub fn new(node_id: u64) -> Self {
        Self {
            state: Mutex::new(HlcTimestamp::zero(node_id)),
        }
    }

    /// Derive a node_id from an Ed25519 public key (first 8 bytes as u64).
    pub fn node_id_from_pubkey(pubkey: &[u8; 32]) -> u64 {
        u64::from_le_bytes(pubkey[0..8].try_into().unwrap())
    }

    /// Generate a timestamp for a LOCAL event (send/create).
    ///
    /// Algorithm:
    /// 1. Read wall clock
    /// 2. If wall clock > last physical_time: advance, reset counter
    /// 3. If wall clock <= last physical_time: increment counter
    /// 4. Return new timestamp
    pub fn tick(&self) -> HlcTimestamp {
        let now_us = now_microseconds();
        let mut state = self.state.lock().unwrap();

        if now_us > state.physical_time_us {
            state.physical_time_us = now_us;
            state.counter = 0;
        } else {
            state.counter += 1;
        }

        *state
    }

    /// Update the clock upon RECEIVING a remote event's timestamp.
    ///
    /// Algorithm:
    /// 1. Read wall clock
    /// 2. Take max(local_physical, remote_physical, wall_clock) as new physical
    /// 3. Adjust counter based on which times were equal
    /// 4. Return new timestamp (guaranteed > both local state and remote timestamp)
    ///
    /// Returns `Err` if the remote timestamp's physical time exceeds local time
    /// by more than MAX_DRIFT_US (60 seconds). This rejects events from nodes
    /// with wildly wrong clocks.
    pub fn receive(&self, remote: &HlcTimestamp) -> Result<HlcTimestamp, HlcDriftError> {
        let now_us = now_microseconds();

        // Drift check: reject timestamps too far in the future
        if remote.physical_time_us > now_us + MAX_DRIFT_US {
            return Err(HlcDriftError {
                remote_time_us: remote.physical_time_us,
                local_time_us: now_us,
                drift_us: remote.physical_time_us - now_us,
                max_drift_us: MAX_DRIFT_US,
            });
        }

        let mut state = self.state.lock().unwrap();

        let old_physical = state.physical_time_us;

        if now_us > old_physical && now_us > remote.physical_time_us {
            // Wall clock is ahead of both — use it, reset counter
            state.physical_time_us = now_us;
            state.counter = 0;
        } else if old_physical == remote.physical_time_us {
            // Local and remote tied — take max counter + 1
            state.counter = state.counter.max(remote.counter) + 1;
        } else if old_physical > remote.physical_time_us {
            // Local is ahead — increment local counter
            state.counter += 1;
        } else {
            // Remote is ahead — adopt remote time, increment remote counter
            state.physical_time_us = remote.physical_time_us;
            state.counter = remote.counter + 1;
        }

        Ok(*state)
    }

    /// Read the current clock state without advancing it.
    pub fn now(&self) -> HlcTimestamp {
        *self.state.lock().unwrap()
    }
}

/// Error when a remote HLC timestamp exceeds the drift bound.
#[derive(Debug, Clone)]
pub struct HlcDriftError {
    pub remote_time_us: u64,
    pub local_time_us: u64,
    pub drift_us: u64,
    pub max_drift_us: u64,
}

impl std::fmt::Display for HlcDriftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "HLC drift exceeded: remote is {}ms ahead (max {}ms)",
            self.drift_us / 1000,
            self.max_drift_us / 1000,
        )
    }
}

impl std::error::Error for HlcDriftError {}

// ---------------------------------------------------------------------------
// Wall clock helper
// ---------------------------------------------------------------------------

/// Current wall-clock time in microseconds since UNIX epoch.
fn now_microseconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_micros() as u64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tick_monotonic() {
        let clock = HybridClock::new(1);

        let t1 = clock.tick();
        let t2 = clock.tick();
        let t3 = clock.tick();

        assert!(t2 > t1, "t2 should be after t1");
        assert!(t3 > t2, "t3 should be after t2");
    }

    #[test]
    fn test_tick_same_microsecond_increments_counter() {
        let clock = HybridClock::new(1);

        // Force same physical time by ticking rapidly
        let t1 = clock.tick();
        let t2 = clock.tick();

        if t2.physical_time_us == t1.physical_time_us {
            assert!(
                t2.counter > t1.counter,
                "counter should increment when physical time hasn't advanced"
            );
        }
        // Either way, t2 > t1
        assert!(t2 > t1);
    }

    #[test]
    fn test_receive_advances_past_remote() {
        let clock_a = HybridClock::new(1);
        let clock_b = HybridClock::new(2);

        let t_a = clock_a.tick();
        let t_b = clock_b.receive(&t_a).unwrap();

        // B's timestamp must be after A's
        assert!(t_b > t_a, "receive should produce timestamp > remote");
    }

    #[test]
    fn test_receive_rejects_excessive_drift() {
        let clock = HybridClock::new(1);

        let far_future = HlcTimestamp {
            physical_time_us: now_microseconds() + MAX_DRIFT_US + 1_000_000,
            counter: 0,
            node_id: 99,
        };

        let result = clock.receive(&far_future);
        assert!(result.is_err(), "should reject timestamps beyond drift bound");
    }

    #[test]
    fn test_receive_accepts_within_drift() {
        let clock = HybridClock::new(1);

        let near_future = HlcTimestamp {
            physical_time_us: now_microseconds() + MAX_DRIFT_US - 1_000_000,
            counter: 0,
            node_id: 99,
        };

        let result = clock.receive(&near_future);
        assert!(result.is_ok(), "should accept timestamps within drift bound");
    }

    #[test]
    fn test_causal_ordering_two_nodes() {
        let clock_a = HybridClock::new(1);
        let clock_b = HybridClock::new(2);

        // A creates event
        let t1 = clock_a.tick();

        // B receives A's event, then creates its own
        let _ = clock_b.receive(&t1).unwrap();
        let t2 = clock_b.tick();

        // A receives B's event, then creates its own
        let _ = clock_a.receive(&t2).unwrap();
        let t3 = clock_a.tick();

        // Causal chain: t1 -> t2 -> t3
        assert!(t2 > t1, "t2 should be causally after t1");
        assert!(t3 > t2, "t3 should be causally after t2");
    }

    #[test]
    fn test_timestamp_serialization_roundtrip() {
        let ts = HlcTimestamp {
            physical_time_us: 1_708_000_000_000_000,
            counter: 42,
            node_id: 0xDEAD_BEEF_CAFE_BABE,
        };

        let bytes = ts.to_bytes();
        let recovered = HlcTimestamp::from_bytes(&bytes);

        assert_eq!(ts, recovered);
    }

    #[test]
    fn test_timestamp_ordering() {
        let a = HlcTimestamp {
            physical_time_us: 100,
            counter: 5,
            node_id: 1,
        };
        let b = HlcTimestamp {
            physical_time_us: 100,
            counter: 5,
            node_id: 2,
        };
        let c = HlcTimestamp {
            physical_time_us: 100,
            counter: 6,
            node_id: 1,
        };
        let d = HlcTimestamp {
            physical_time_us: 101,
            counter: 0,
            node_id: 1,
        };

        // Physical time dominates, then counter, then node_id
        assert!(a < b, "same time/counter, higher node_id wins");
        assert!(a < c, "same time, higher counter wins");
        assert!(c < d, "higher physical time wins regardless of counter");
    }

    #[test]
    fn test_node_id_from_pubkey() {
        let mut pubkey = [0u8; 32];
        pubkey[0..8].copy_from_slice(&42u64.to_le_bytes());

        assert_eq!(HybridClock::node_id_from_pubkey(&pubkey), 42);
    }

    #[test]
    fn test_display() {
        let ts = HlcTimestamp {
            physical_time_us: 1000000,
            counter: 3,
            node_id: 0xFF,
        };
        let s = format!("{}", ts);
        assert!(s.contains("1000000"));
        assert!(s.contains(":3:"));
    }
}
