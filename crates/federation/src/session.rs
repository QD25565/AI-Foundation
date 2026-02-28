//! Peer Session — QUIC-based federation protocol session.
//!
//! Manages the lifecycle of a connection to a federation peer:
//! handshake (Hello/Welcome), event exchange, presence, and shutdown.
//!
//! # Protocol Flow
//!
//! ```text
//! Initiator                          Responder
//!     |                                  |
//!     |--- [bidi stream 0] ------------>|
//!     |    Hello(node, version)          |
//!     |<---------------------------------|
//!     |    Welcome(node, version, ok)    |
//!     |                                  |
//!     |--- [bidi stream N] ------------>|  (one per push)
//!     |    EventPushRequest              |
//!     |<---------------------------------|
//!     |    EventPushResponse             |
//!     |                                  |
//!     |--- [uni stream] --------------->|  (fire-and-forget)
//!     |    PresencePushRequest           |
//!     |                                  |
//! ```
//!
//! Each logical exchange gets its own QUIC stream — no head-of-line blocking.
//! The handshake uses the first bidirectional stream. Subsequent event pushes
//! each open a new bidirectional stream. Presence is fire-and-forget on
//! unidirectional streams.

use crate::{
    gateway::PeerRegistryConfig,
    messages::{FederationMessage, FederationPayload},
    node::FederationNode,
    sync::{EventPushRequest, EventPushResponse, PresencePushRequest},
    transport::{recv_message, send_message, send_message_finish},
    FederationError, Result, TeambookIdentity,
};
use ed25519_dalek::SigningKey;
use tracing::{debug, info, warn};

/// Current federation protocol version.
pub const PROTOCOL_VERSION: u32 = 1;

/// Handshake timeout in seconds.
const HANDSHAKE_TIMEOUT_SECS: u64 = 30;

// ---------------------------------------------------------------------------
// CBOR helpers
// ---------------------------------------------------------------------------

fn cbor_encode<T: serde::Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    ciborium::into_writer(value, &mut buf)
        .map_err(|e| FederationError::SerializationError(format!("CBOR encode failed: {e}")))?;
    Ok(buf)
}

fn cbor_decode<T: serde::de::DeserializeOwned>(data: &[u8]) -> Result<T> {
    ciborium::from_reader(data)
        .map_err(|e| FederationError::SerializationError(format!("CBOR decode failed: {e}")))
}

// ---------------------------------------------------------------------------
// PeerSession
// ---------------------------------------------------------------------------

/// A live session with a connected federation peer over QUIC.
///
/// Created by `connect()` (initiator) or `accept()` (responder) after a
/// successful Hello/Welcome handshake. Provides methods to push events
/// and presence to the remote peer.
pub struct PeerSession {
    /// The underlying QUIC connection.
    conn: iroh::endpoint::Connection,

    /// Remote node identity and capabilities (from their Hello/Welcome).
    remote_node: FederationNode,
}

impl PeerSession {
    /// The remote node's identity and capabilities.
    pub fn remote_node(&self) -> &FederationNode {
        &self.remote_node
    }

    /// The remote node's ID (hex hash of public key).
    pub fn remote_node_id(&self) -> &str {
        &self.remote_node.node_id
    }

    /// The remote peer's iroh endpoint ID (Ed25519 public key).
    pub fn remote_endpoint_id(&self) -> iroh::EndpointId {
        self.conn.remote_id()
    }

    // -----------------------------------------------------------------------
    // Initiator-side handshake
    // -----------------------------------------------------------------------

    /// Perform the initiator-side handshake (outbound connection).
    ///
    /// Opens a bidirectional stream, sends Hello, receives Welcome.
    /// Verifies the remote peer's signature and registry membership.
    /// Returns a connected `PeerSession` on success.
    pub async fn connect(
        conn: iroh::endpoint::Connection,
        identity: &TeambookIdentity,
        local_node: &FederationNode,
        peers: &PeerRegistryConfig,
    ) -> Result<Self> {
        let signing_key = SigningKey::from_bytes(&identity.secret_key_bytes());
        let my_node_id = crate::node_id_from_pubkey(identity.verifying_key());

        // Open the handshake stream
        let (mut send, mut recv) = conn.open_bi().await.map_err(|e| {
            FederationError::ConnectionFailed(format!("Failed to open handshake stream: {e}"))
        })?;

        // Send Hello
        let hello = FederationMessage::new(
            &my_node_id,
            FederationPayload::Hello {
                node: local_node.clone(),
                protocol_version: PROTOCOL_VERSION,
            },
            &signing_key,
        );
        send_message(&mut send, &hello.to_bytes()?).await?;
        debug!(remote = %conn.remote_id(), "Sent Hello");

        // Receive Welcome (with timeout)
        let welcome_bytes = tokio::time::timeout(
            std::time::Duration::from_secs(HANDSHAKE_TIMEOUT_SECS),
            recv_message(&mut recv),
        )
        .await
        .map_err(|_| {
            FederationError::ConnectionFailed(
                "Handshake timeout waiting for Welcome".to_string(),
            )
        })??;

        let welcome_msg = FederationMessage::from_bytes(&welcome_bytes)?;

        match welcome_msg.payload {
            FederationPayload::Welcome {
                ref node,
                protocol_version,
                accepted,
                ref rejection_reason,
            } => {
                if !accepted {
                    return Err(FederationError::ConnectionFailed(format!(
                        "Peer rejected connection: {}",
                        rejection_reason.as_deref().unwrap_or("no reason given")
                    )));
                }

                // Verify the Welcome message signature
                if !welcome_msg.verify(&node.pubkey) {
                    return Err(FederationError::AuthenticationFailed(
                        "Welcome message signature verification failed".to_string(),
                    ));
                }

                // Verify peer is in our registry
                let remote_pubkey_hex = hex::encode(node.pubkey.as_bytes());
                if !peers.is_known_peer(&remote_pubkey_hex) {
                    return Err(FederationError::AuthenticationFailed(format!(
                        "Remote peer {} not in local registry",
                        node.node_id
                    )));
                }

                if protocol_version != PROTOCOL_VERSION {
                    warn!(
                        local = PROTOCOL_VERSION,
                        remote = protocol_version,
                        "Protocol version mismatch (continuing anyway)"
                    );
                }

                info!(
                    remote_node_id = %node.node_id,
                    remote_name = %node.display_name,
                    "Federation handshake complete (initiator)"
                );

                Ok(Self { conn, remote_node: node.clone() })
            }
            _ => Err(FederationError::ConnectionFailed(
                "Expected Welcome message, got unexpected payload".to_string(),
            )),
        }
    }

    // -----------------------------------------------------------------------
    // Responder-side handshake
    // -----------------------------------------------------------------------

    /// Perform the responder-side handshake (inbound connection).
    ///
    /// Accepts a bidirectional stream, receives Hello, validates the peer,
    /// sends Welcome. On validation failure, sends a rejection Welcome
    /// before returning an error.
    pub async fn accept(
        conn: iroh::endpoint::Connection,
        identity: &TeambookIdentity,
        local_node: &FederationNode,
        peers: &PeerRegistryConfig,
    ) -> Result<Self> {
        let signing_key = SigningKey::from_bytes(&identity.secret_key_bytes());
        let my_node_id = crate::node_id_from_pubkey(identity.verifying_key());

        // Accept the handshake stream
        let (mut send, mut recv) = conn.accept_bi().await.map_err(|e| {
            FederationError::ConnectionFailed(format!("Failed to accept handshake stream: {e}"))
        })?;

        // Receive Hello (with timeout)
        let hello_bytes = tokio::time::timeout(
            std::time::Duration::from_secs(HANDSHAKE_TIMEOUT_SECS),
            recv_message(&mut recv),
        )
        .await
        .map_err(|_| {
            FederationError::ConnectionFailed(
                "Handshake timeout waiting for Hello".to_string(),
            )
        })??;

        let hello_msg = FederationMessage::from_bytes(&hello_bytes)?;

        match hello_msg.payload {
            FederationPayload::Hello {
                node: ref remote_node,
                protocol_version,
            } => {
                // Verify the Hello message signature
                if !hello_msg.verify(&remote_node.pubkey) {
                    let reject = Self::build_welcome(
                        &my_node_id,
                        local_node,
                        false,
                        Some("Signature verification failed"),
                        &signing_key,
                    );
                    let _ = send_message(&mut send, &reject.to_bytes()?).await;
                    return Err(FederationError::AuthenticationFailed(
                        "Hello message signature verification failed".to_string(),
                    ));
                }

                // Verify peer is in our registry
                let remote_pubkey_hex = hex::encode(remote_node.pubkey.as_bytes());
                if !peers.is_known_peer(&remote_pubkey_hex) {
                    let reject = Self::build_welcome(
                        &my_node_id,
                        local_node,
                        false,
                        Some("Unknown peer — not in local registry"),
                        &signing_key,
                    );
                    let _ = send_message(&mut send, &reject.to_bytes()?).await;
                    return Err(FederationError::AuthenticationFailed(format!(
                        "Remote peer {} not in local registry",
                        remote_node.node_id
                    )));
                }

                if protocol_version != PROTOCOL_VERSION {
                    warn!(
                        local = PROTOCOL_VERSION,
                        remote = protocol_version,
                        "Protocol version mismatch (continuing anyway)"
                    );
                }

                // Send Welcome (accepted)
                let welcome = Self::build_welcome(
                    &my_node_id,
                    local_node,
                    true,
                    None,
                    &signing_key,
                );
                send_message(&mut send, &welcome.to_bytes()?).await?;

                info!(
                    remote_node_id = %remote_node.node_id,
                    remote_name = %remote_node.display_name,
                    "Federation handshake complete (responder)"
                );

                Ok(Self {
                    conn,
                    remote_node: remote_node.clone(),
                })
            }
            _ => Err(FederationError::ConnectionFailed(
                "Expected Hello message, got unexpected payload".to_string(),
            )),
        }
    }

    // -----------------------------------------------------------------------
    // Event exchange
    // -----------------------------------------------------------------------

    /// Push events to the remote peer.
    ///
    /// Opens a new bidirectional stream, sends the request, and waits for
    /// the response. Each push gets its own stream (no head-of-line blocking).
    pub async fn push_events(
        &self,
        request: &EventPushRequest,
    ) -> Result<EventPushResponse> {
        let (send, mut recv) = self.conn.open_bi().await.map_err(|e| {
            FederationError::TransportError(format!("Failed to open event stream: {e}"))
        })?;

        // Send request and finish the send side
        let request_bytes = cbor_encode(request)?;
        send_message_finish(send, &request_bytes).await?;

        // Receive response
        let response_bytes = recv_message(&mut recv).await?;
        let response: EventPushResponse = cbor_decode(&response_bytes)?;

        debug!(
            accepted = response.accepted,
            duplicates = response.duplicates,
            rejected = response.rejected,
            "Event push response received"
        );

        Ok(response)
    }

    /// Handle an incoming event push from the remote peer.
    ///
    /// Accepts a bidirectional stream, reads the request, processes it
    /// with the provided handler, and sends the response.
    pub async fn handle_event_push<F>(
        &self,
        handler: F,
    ) -> Result<()>
    where
        F: FnOnce(EventPushRequest) -> EventPushResponse,
    {
        let (send, mut recv) = self.conn.accept_bi().await.map_err(|e| {
            FederationError::TransportError(format!("Failed to accept event stream: {e}"))
        })?;

        let request_bytes = recv_message(&mut recv).await?;
        let request: EventPushRequest = cbor_decode(&request_bytes)?;

        let response = handler(request);

        let response_bytes = cbor_encode(&response)?;
        send_message_finish(send, &response_bytes).await?;

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Presence (fire-and-forget)
    // -----------------------------------------------------------------------

    /// Send presence information to the remote peer.
    ///
    /// Opens a unidirectional stream and sends the presence data.
    /// No response is expected (EDU — fire-and-forget).
    pub async fn send_presence(&self, request: &PresencePushRequest) -> Result<()> {
        let send = self.conn.open_uni().await.map_err(|e| {
            FederationError::TransportError(format!("Failed to open presence stream: {e}"))
        })?;

        let request_bytes = cbor_encode(request)?;
        send_message_finish(send, &request_bytes).await?;

        debug!(
            ai_count = request.presences.len(),
            "Presence sent to peer"
        );

        Ok(())
    }

    /// Receive presence from the remote peer (unidirectional stream).
    pub async fn recv_presence(&self) -> Result<PresencePushRequest> {
        let mut recv = self.conn.accept_uni().await.map_err(|e| {
            FederationError::TransportError(format!("Failed to accept presence stream: {e}"))
        })?;

        let data = recv_message(&mut recv).await?;
        let request: PresencePushRequest = cbor_decode(&data)?;

        debug!(
            ai_count = request.presences.len(),
            from = %request.sender_short_id,
            "Presence received from peer"
        );

        Ok(request)
    }

    // -----------------------------------------------------------------------
    // Shutdown
    // -----------------------------------------------------------------------

    /// Close the session gracefully.
    ///
    /// Sends a QUIC close frame with the provided reason. The remote peer
    /// will see the connection as closed with error code 0 (no error).
    pub fn close(self, reason: &str) {
        info!(
            remote_node_id = %self.remote_node.node_id,
            reason,
            "Closing federation session"
        );
        self.conn.close(0u32.into(), reason.as_bytes());
    }

    /// Access the underlying QUIC connection (for advanced use).
    pub fn connection(&self) -> &iroh::endpoint::Connection {
        &self.conn
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn build_welcome(
        from: &str,
        local_node: &FederationNode,
        accepted: bool,
        rejection_reason: Option<&str>,
        signing_key: &SigningKey,
    ) -> FederationMessage {
        FederationMessage::new(
            from,
            FederationPayload::Welcome {
                node: local_node.clone(),
                protocol_version: PROTOCOL_VERSION,
                accepted,
                rejection_reason: rejection_reason.map(|s| s.to_string()),
            },
            signing_key,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::PeerRegistryConfig;
    use crate::node::FederationNode;

    fn make_identity_and_node(name: &str) -> (TeambookIdentity, FederationNode, SigningKey) {
        let identity = TeambookIdentity::generate();
        let signing_key = SigningKey::from_bytes(&identity.secret_key_bytes());
        let node = FederationNode::new_local(name, &signing_key);
        (identity, node, signing_key)
    }

    fn make_peers_with(pubkey_hex: &str, name: &str) -> PeerRegistryConfig {
        use crate::gateway::PeerEntry;
        PeerRegistryConfig {
            peers: vec![PeerEntry {
                pubkey_hex: pubkey_hex.to_string(),
                endpoint: "quic://test".to_string(),
                name: name.to_string(),
                trusted: true,
            }],
        }
    }

    #[test]
    fn test_cbor_roundtrip_push_request() {
        use crate::hlc::HlcTimestamp;

        let request = EventPushRequest {
            events: vec![],
            sender_hlc: HlcTimestamp {
                physical_time_us: 1000,
                counter: 0,
                node_id: 42,
            },
            sender_head_seq: 100,
        };

        let encoded = cbor_encode(&request).unwrap();
        let decoded: EventPushRequest = cbor_decode(&encoded).unwrap();

        assert_eq!(decoded.sender_head_seq, 100);
        assert!(decoded.events.is_empty());
    }

    #[test]
    fn test_cbor_roundtrip_push_response() {
        use crate::hlc::HlcTimestamp;

        let response = EventPushResponse {
            accepted: 5,
            duplicates: 2,
            rejected: 0,
            errors: vec![],
            receiver_hlc: HlcTimestamp {
                physical_time_us: 2000,
                counter: 1,
                node_id: 99,
            },
            receiver_head_seq: 50,
        };

        let encoded = cbor_encode(&response).unwrap();
        let decoded: EventPushResponse = cbor_decode(&encoded).unwrap();

        assert_eq!(decoded.accepted, 5);
        assert_eq!(decoded.duplicates, 2);
        assert_eq!(decoded.receiver_head_seq, 50);
    }

    #[test]
    fn test_hello_message_sign_verify() {
        let (_identity, node, signing_key) = make_identity_and_node("TestNode");
        let node_id = node.node_id.clone();

        let hello = FederationMessage::new(
            &node_id,
            FederationPayload::Hello {
                node: node.clone(),
                protocol_version: PROTOCOL_VERSION,
            },
            &signing_key,
        );

        // Should verify with correct key
        assert!(hello.verify(&node.pubkey));

        // Should fail with wrong key
        let (_, other_node, _) = make_identity_and_node("Other");
        assert!(!hello.verify(&other_node.pubkey));
    }

    #[test]
    fn test_welcome_message_sign_verify() {
        let (_identity, node, signing_key) = make_identity_and_node("Responder");
        let node_id = node.node_id.clone();

        let welcome = PeerSession::build_welcome(
            &node_id,
            &node,
            true,
            None,
            &signing_key,
        );

        assert!(welcome.verify(&node.pubkey));

        // Rejection message should also verify
        let reject = PeerSession::build_welcome(
            &node_id,
            &node,
            false,
            Some("test rejection"),
            &signing_key,
        );
        assert!(reject.verify(&node.pubkey));
    }

    #[test]
    fn test_hello_welcome_cbor_roundtrip() {
        let (_identity, node, signing_key) = make_identity_and_node("TestNode");
        let node_id = node.node_id.clone();

        let hello = FederationMessage::new(
            &node_id,
            FederationPayload::Hello {
                node: node.clone(),
                protocol_version: PROTOCOL_VERSION,
            },
            &signing_key,
        );

        // Serialize and deserialize
        let bytes = hello.to_bytes().unwrap();
        let restored = FederationMessage::from_bytes(&bytes).unwrap();

        assert_eq!(restored.from, node_id);
        match restored.payload {
            FederationPayload::Hello {
                protocol_version, ..
            } => {
                assert_eq!(protocol_version, PROTOCOL_VERSION);
            }
            _ => panic!("Expected Hello payload"),
        }

        // Signature should still verify after roundtrip
        assert!(restored.verify(&node.pubkey));
    }

    #[test]
    fn test_peer_registry_known_check() {
        let (_identity, node, _signing_key) = make_identity_and_node("KnownPeer");
        let pubkey_hex = hex::encode(node.pubkey.as_bytes());

        let peers = make_peers_with(&pubkey_hex, "KnownPeer");
        assert!(peers.is_known_peer(&pubkey_hex));

        let (_identity2, node2, _) = make_identity_and_node("UnknownPeer");
        let unknown_hex = hex::encode(node2.pubkey.as_bytes());
        assert!(!peers.is_known_peer(&unknown_hex));
    }

    #[test]
    fn test_cbor_roundtrip_presence() {
        let request = PresencePushRequest {
            presences: vec![],
            sender_short_id: "a3f7c2d1".to_string(),
        };

        let encoded = cbor_encode(&request).unwrap();
        let decoded: PresencePushRequest = cbor_decode(&encoded).unwrap();

        assert_eq!(decoded.sender_short_id, "a3f7c2d1");
        assert!(decoded.presences.is_empty());
    }

    #[test]
    fn test_protocol_version_constant() {
        assert!(PROTOCOL_VERSION >= 1);
    }

    /// Full handshake integration test over real QUIC.
    ///
    /// Requires native network stack — iroh's QUIC loopback doesn't
    /// work in WSL2 (virtual network adapters).
    #[tokio::test]
    #[ignore = "requires native network stack (not WSL2) for QUIC loopback"]
    async fn test_full_handshake_over_quic() {
        use crate::transport::{identity_to_iroh_key, FEDERATION_ALPN};
        use iroh::address_lookup::memory::MemoryLookup;

        let (id_a, node_a, _) = make_identity_and_node("Teambook-A");
        let (id_b, node_b, _) = make_identity_and_node("Teambook-B");

        // Each side knows the other
        let pubkey_a_hex = hex::encode(node_a.pubkey.as_bytes());
        let pubkey_b_hex = hex::encode(node_b.pubkey.as_bytes());
        let peers_a = make_peers_with(&pubkey_b_hex, "Teambook-B");
        let peers_b = make_peers_with(&pubkey_a_hex, "Teambook-A");

        let address_lookup = MemoryLookup::new();

        // Create endpoints
        let ep_a = iroh::Endpoint::empty_builder(iroh::RelayMode::Disabled)
            .secret_key(identity_to_iroh_key(&id_a))
            .alpns(vec![FEDERATION_ALPN.to_vec()])
            .address_lookup(address_lookup.clone())
            .bind()
            .await
            .unwrap();

        let ep_b = iroh::Endpoint::empty_builder(iroh::RelayMode::Disabled)
            .secret_key(identity_to_iroh_key(&id_b))
            .alpns(vec![FEDERATION_ALPN.to_vec()])
            .address_lookup(address_lookup.clone())
            .bind()
            .await
            .unwrap();

        let addr_b = ep_b.addr();

        // Initiator connects, responder accepts — concurrently
        let node_a_clone = node_a.clone();
        let node_b_clone = node_b.clone();

        let (conn_a_result, incoming_b) = tokio::join!(
            ep_a.connect(addr_b, FEDERATION_ALPN),
            ep_b.accept()
        );

        let conn_a = conn_a_result.unwrap();
        let conn_b = incoming_b.unwrap().await.unwrap();

        // Perform handshake concurrently
        let (session_a_result, session_b_result) = tokio::join!(
            PeerSession::connect(conn_a, &id_a, &node_a_clone, &peers_a),
            PeerSession::accept(conn_b, &id_b, &node_b_clone, &peers_b),
        );

        let session_a = session_a_result.expect("Initiator handshake should succeed");
        let session_b = session_b_result.expect("Responder handshake should succeed");

        // Verify identities were exchanged correctly
        assert_eq!(session_a.remote_node().node_id, node_b.node_id);
        assert_eq!(session_b.remote_node().node_id, node_a.node_id);
        assert_eq!(
            session_a.remote_node().display_name,
            "Teambook-B"
        );
        assert_eq!(
            session_b.remote_node().display_name,
            "Teambook-A"
        );

        // Clean up
        session_a.close("test complete");
        session_b.close("test complete");
        ep_a.close().await;
        ep_b.close().await;
    }
}
