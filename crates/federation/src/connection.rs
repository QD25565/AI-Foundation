//! Federation connection state machine

use crate::{
    Endpoint, FederationNode, NegotiatedSharing,
    SharingPreferences, TransportType, Result, FederationError,
};
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use std::time::Duration;

/// State of a federation connection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionState {
    /// Looking for the peer
    Discovering,
    /// Transport layer connecting
    Connecting,
    /// Exchanging Hello/Welcome (AFP-style)
    Handshaking,
    /// Negotiating sharing preferences
    Negotiating,
    /// Fully connected and operational
    Connected,
    /// Temporarily suspended (e.g., rate limited)
    Suspended,
    /// Connection closed or failed
    Disconnected,
}

impl Default for ConnectionState {
    fn default() -> Self {
        ConnectionState::Disconnected
    }
}

/// A connection to another federation node
#[derive(Debug)]
pub struct FederationConnection {
    /// The remote node
    pub remote_node: FederationNode,

    /// Current state
    pub state: ConnectionState,

    /// The endpoint used for this connection
    pub endpoint: Endpoint,

    /// Transport type in use
    pub transport_type: TransportType,

    /// When the connection was established
    pub established_at: Option<DateTime<Utc>>,

    /// When we last received data
    pub last_activity: DateTime<Utc>,

    /// Local sharing preferences for this connection
    pub local_prefs: SharingPreferences,

    /// Remote sharing preferences
    pub remote_prefs: SharingPreferences,

    /// Negotiated sharing result
    pub negotiated: Option<NegotiatedSharing>,

    /// Connection quality metrics
    pub metrics: ConnectionMetrics,

    /// Pending messages to send
    pending_send: Vec<Vec<u8>>,

    /// Received messages to process
    pending_recv: Vec<Vec<u8>>,
}

impl FederationConnection {
    /// Create a new outbound connection
    pub fn new_outbound(remote_node: FederationNode, endpoint: Endpoint, local_prefs: SharingPreferences) -> Self {
        Self {
            transport_type: endpoint.transport_type(),
            remote_node,
            state: ConnectionState::Discovering,
            endpoint,
            established_at: None,
            last_activity: Utc::now(),
            local_prefs,
            remote_prefs: SharingPreferences::minimal(),
            negotiated: None,
            metrics: ConnectionMetrics::default(),
            pending_send: Vec::new(),
            pending_recv: Vec::new(),
        }
    }

    /// Create from an inbound connection
    pub fn new_inbound(remote_node: FederationNode, endpoint: Endpoint, local_prefs: SharingPreferences) -> Self {
        Self {
            transport_type: endpoint.transport_type(),
            remote_node,
            state: ConnectionState::Handshaking,
            endpoint,
            established_at: None,
            last_activity: Utc::now(),
            local_prefs,
            remote_prefs: SharingPreferences::minimal(),
            negotiated: None,
            metrics: ConnectionMetrics::default(),
            pending_send: Vec::new(),
            pending_recv: Vec::new(),
        }
    }

    /// Transition to a new state
    pub fn transition(&mut self, new_state: ConnectionState) -> Result<()> {
        use ConnectionState::*;

        // Validate state transitions
        let valid = match (self.state, new_state) {
            // From Discovering
            (Discovering, Connecting) => true,
            (Discovering, Disconnected) => true,

            // From Connecting
            (Connecting, Handshaking) => true,
            (Connecting, Disconnected) => true,

            // From Handshaking
            (Handshaking, Negotiating) => true,
            (Handshaking, Disconnected) => true,

            // From Negotiating
            (Negotiating, Connected) => true,
            (Negotiating, Disconnected) => true,

            // From Connected
            (Connected, Suspended) => true,
            (Connected, Disconnected) => true,

            // From Suspended
            (Suspended, Connected) => true,
            (Suspended, Disconnected) => true,

            // Already disconnected
            (Disconnected, Discovering) => true,  // Retry
            (Disconnected, _) => false,

            // Same state is no-op
            (s1, s2) if s1 == s2 => true,

            _ => false,
        };

        if valid {
            self.state = new_state;
            self.last_activity = Utc::now();

            if new_state == Connected {
                self.established_at = Some(Utc::now());
            }

            Ok(())
        } else {
            Err(FederationError::Internal(format!(
                "Invalid state transition: {:?} -> {:?}",
                self.state, new_state
            )))
        }
    }

    /// Check if connection is active
    pub fn is_active(&self) -> bool {
        matches!(self.state, ConnectionState::Connected | ConnectionState::Suspended)
    }

    /// Check if connection is fully operational
    pub fn is_connected(&self) -> bool {
        self.state == ConnectionState::Connected
    }

    /// Update remote preferences and re-negotiate
    pub fn update_remote_prefs(&mut self, prefs: SharingPreferences) {
        self.remote_prefs = prefs;
        self.negotiated = Some(NegotiatedSharing::negotiate(&self.local_prefs, &self.remote_prefs));
    }

    /// Check if a data category can be shared on this connection
    pub fn can_share(&self, category: crate::DataCategory) -> bool {
        self.negotiated
            .as_ref()
            .map(|n| n.shared_categories.contains(&category))
            .unwrap_or(false)
    }

    /// Check if DMs are allowed
    pub fn can_dm(&self) -> bool {
        self.negotiated
            .as_ref()
            .map(|n| n.dms_allowed)
            .unwrap_or(false)
    }

    /// Queue a message to send
    pub fn queue_send(&mut self, data: Vec<u8>) {
        self.pending_send.push(data);
    }

    /// Get pending messages to send
    pub fn drain_send_queue(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.pending_send)
    }

    /// Add received message
    pub fn queue_recv(&mut self, data: Vec<u8>) {
        self.pending_recv.push(data);
        self.last_activity = Utc::now();
    }

    /// Get pending received messages
    pub fn drain_recv_queue(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.pending_recv)
    }

    /// Update metrics after successful operation
    pub fn record_success(&mut self, latency_ms: u32) {
        self.metrics.record_success(latency_ms);
        self.last_activity = Utc::now();
    }

    /// Update metrics after failure
    pub fn record_failure(&mut self) {
        self.metrics.record_failure();
    }

    /// Check if connection appears dead
    pub fn is_stale(&self, timeout: Duration) -> bool {
        let elapsed = Utc::now().signed_duration_since(self.last_activity);
        elapsed.to_std().map(|d| d > timeout).unwrap_or(true)
    }

    /// Get connection uptime
    pub fn uptime(&self) -> Option<Duration> {
        self.established_at.map(|est| {
            Utc::now()
                .signed_duration_since(est)
                .to_std()
                .unwrap_or(Duration::ZERO)
        })
    }
}

/// Connection quality metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnectionMetrics {
    /// Average latency in ms
    pub avg_latency_ms: f64,

    /// Latency samples count
    latency_samples: u32,

    /// Messages sent
    pub messages_sent: u64,

    /// Messages received
    pub messages_received: u64,

    /// Bytes sent
    pub bytes_sent: u64,

    /// Bytes received
    pub bytes_received: u64,

    /// Successful operations
    pub successes: u64,

    /// Failed operations
    pub failures: u64,

    /// Current reliability score (0.0 - 1.0)
    pub reliability: f64,
}

impl ConnectionMetrics {
    /// Record a successful operation
    pub fn record_success(&mut self, latency_ms: u32) {
        self.successes += 1;

        // Update running average latency
        let new_count = self.latency_samples + 1;
        self.avg_latency_ms = (self.avg_latency_ms * self.latency_samples as f64
            + latency_ms as f64) / new_count as f64;
        self.latency_samples = new_count;

        self.update_reliability();
    }

    /// Record a failed operation
    pub fn record_failure(&mut self) {
        self.failures += 1;
        self.update_reliability();
    }

    /// Update reliability score
    fn update_reliability(&mut self) {
        let total = self.successes + self.failures;
        if total > 0 {
            self.reliability = self.successes as f64 / total as f64;
        }
    }

    /// Record sent data
    pub fn record_send(&mut self, bytes: u64) {
        self.messages_sent += 1;
        self.bytes_sent += bytes;
    }

    /// Record received data
    pub fn record_recv(&mut self, bytes: u64) {
        self.messages_received += 1;
        self.bytes_received += bytes;
    }
}

/// Connection event for external handling
#[derive(Debug, Clone)]
pub enum ConnectionEvent {
    /// Connection state changed
    StateChanged {
        node_id: String,
        old_state: ConnectionState,
        new_state: ConnectionState,
    },

    /// Message received
    MessageReceived {
        node_id: String,
        data: Vec<u8>,
    },

    /// Connection established
    Connected {
        node_id: String,
        negotiated: NegotiatedSharing,
    },

    /// Connection lost
    Disconnected {
        node_id: String,
        reason: String,
    },

    /// Error occurred
    Error {
        node_id: String,
        error: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    fn make_test_node() -> FederationNode {
        let signing_key = SigningKey::generate(&mut OsRng);
        FederationNode::new_local("Test", &signing_key)
    }

    #[test]
    fn test_state_transitions() {
        let node = make_test_node();
        let endpoint = Endpoint::quic("127.0.0.1:31420".parse().unwrap());
        let mut conn = FederationConnection::new_outbound(node, endpoint, SharingPreferences::default());

        assert_eq!(conn.state, ConnectionState::Discovering);

        conn.transition(ConnectionState::Connecting).unwrap();
        assert_eq!(conn.state, ConnectionState::Connecting);

        conn.transition(ConnectionState::Handshaking).unwrap();
        conn.transition(ConnectionState::Negotiating).unwrap();
        conn.transition(ConnectionState::Connected).unwrap();

        assert!(conn.is_connected());
        assert!(conn.established_at.is_some());
    }

    #[test]
    fn test_invalid_transition() {
        let node = make_test_node();
        let endpoint = Endpoint::quic("127.0.0.1:31420".parse().unwrap());
        let mut conn = FederationConnection::new_outbound(node, endpoint, SharingPreferences::default());

        // Can't jump from Discovering to Connected
        let result = conn.transition(ConnectionState::Connected);
        assert!(result.is_err());
    }

    #[test]
    fn test_metrics() {
        let mut metrics = ConnectionMetrics::default();

        metrics.record_success(100);
        metrics.record_success(200);
        metrics.record_failure();

        assert_eq!(metrics.successes, 2);
        assert_eq!(metrics.failures, 1);
        assert!((metrics.avg_latency_ms - 150.0).abs() < 0.1);
        assert!((metrics.reliability - 0.666).abs() < 0.01);
    }
}
