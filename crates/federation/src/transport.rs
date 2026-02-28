//! QUIC Transport Layer for Federation
//!
//! Uses iroh for P2P QUIC connections with:
//! - Ed25519 identity (same keys as TeambookIdentity — byte-compatible)
//! - NAT traversal (relay + UDP hole-punching, ~90% direct success)
//! - LAN discovery (iroh's built-in mDNS)
//! - Multiplexed bidirectional QUIC streams
//! - 0-RTT reconnection, connection migration
//!
//! # Architecture
//!
//! `QuicTransport` wraps an `iroh::Endpoint`. It creates the endpoint from
//! a `TeambookIdentity`, binds to a local port, and provides methods to
//! accept incoming connections and connect to peers.
//!
//! Messages use a 4-byte big-endian length prefix followed by the payload.
//! The payload format is defined by the caller (typically CBOR or bincode).

use crate::{FederationError, Result, TeambookIdentity};
use tracing::info;

/// ALPN protocol identifier for Teambook federation.
///
/// Both sides must use identical ALPN bytes or the QUIC handshake is rejected.
/// Versioned so we can evolve the wire protocol without breaking connections.
pub const FEDERATION_ALPN: &[u8] = b"teambook/federation/0";

/// Maximum federation message size (2 MB).
///
/// Events are typically < 4 KB. With MAX_EVENTS_PER_PUSH = 500, a normal
/// batch of 500 events at 4 KB each = 2 MB. This bounds the allocation
/// from a network-controlled length prefix — a malicious peer can trigger
/// at most a 2 MB heap allocation per stream, not 16 MB.
///
/// If larger payloads are ever needed, use pagination (has_more flag in
/// pull responses) rather than increasing this limit.
pub const MAX_MESSAGE_SIZE: usize = 2 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Identity bridge: ed25519_dalek ↔ iroh
// ---------------------------------------------------------------------------

/// Convert a `TeambookIdentity`'s Ed25519 key to an `iroh::SecretKey`.
///
/// Both types wrap the same 32-byte Ed25519 seed. The conversion is zero-cost
/// (just reinterprets the bytes).
pub fn identity_to_iroh_key(identity: &TeambookIdentity) -> iroh::SecretKey {
    iroh::SecretKey::from_bytes(&identity.secret_key_bytes())
}

// ---------------------------------------------------------------------------
// QUIC Transport
// ---------------------------------------------------------------------------

/// P2P QUIC transport layer powered by iroh.
///
/// Provides connection management for federation peers. The endpoint is
/// created from a `TeambookIdentity` — the Ed25519 public key becomes
/// the iroh `EndpointId`, which is how peers address each other.
///
/// # Connection lifecycle
///
/// 1. Call `QuicTransport::bind()` to create and start the endpoint
/// 2. Accept incoming connections via `accept()` in a loop
/// 3. Connect to known peers via `connect()` or `connect_addr()`
/// 4. Exchange messages on bidirectional streams using `send_message`/`recv_message`
/// 5. Call `shutdown()` for graceful teardown
pub struct QuicTransport {
    endpoint: iroh::Endpoint,
}

impl QuicTransport {
    /// Create and bind the QUIC transport.
    ///
    /// This generates an iroh endpoint from the Teambook's Ed25519 identity,
    /// connects to relay servers for NAT traversal, and begins listening
    /// for incoming connections.
    pub async fn bind(identity: &TeambookIdentity) -> Result<Self> {
        let secret_key = identity_to_iroh_key(identity);

        let endpoint = iroh::Endpoint::builder()
            .secret_key(secret_key)
            .alpns(vec![FEDERATION_ALPN.to_vec()])
            .bind()
            .await
            .map_err(|e| {
                FederationError::TransportError(format!("Failed to bind iroh endpoint: {e}"))
            })?;

        // Wait for the endpoint to be fully online (relay connected, addresses resolved)
        endpoint.online().await;

        let endpoint_id = endpoint.id();
        let local_addrs = endpoint.bound_sockets();
        info!(
            %endpoint_id,
            ?local_addrs,
            "Federation QUIC transport online"
        );

        Ok(Self { endpoint })
    }

    /// Our endpoint ID (Ed25519 public key) — this is how peers address us.
    pub fn endpoint_id(&self) -> iroh::EndpointId {
        self.endpoint.id()
    }

    /// Get the full endpoint address (endpoint ID + relay URLs + direct addrs).
    ///
    /// Share this with peers so they can connect to us. On LAN, iroh's
    /// built-in mDNS handles this automatically.
    pub fn endpoint_addr(&self) -> iroh::EndpointAddr {
        self.endpoint.addr()
    }

    /// Accept the next incoming connection.
    ///
    /// Blocks until a peer connects. Returns `None` if the endpoint is
    /// shutting down. The caller should run this in a loop.
    pub async fn accept(&self) -> Option<iroh::endpoint::Incoming> {
        self.endpoint.accept().await
    }

    /// Connect to a peer by their endpoint ID.
    ///
    /// iroh resolves the peer's address via:
    /// 1. LAN mDNS (if on same network)
    /// 2. Relay servers (always works, may have 10-20ms extra latency)
    /// 3. Direct UDP hole-punching (attempted automatically, ~90% success)
    pub async fn connect(
        &self,
        endpoint_id: iroh::EndpointId,
    ) -> Result<iroh::endpoint::Connection> {
        self.connect_addr(endpoint_id).await
    }

    /// Connect to a peer with explicit address info.
    ///
    /// Use when you have the peer's relay URL or direct socket addresses
    /// (e.g., from mDNS discovery TXT records or a pairing passkey).
    pub async fn connect_addr(
        &self,
        addr: impl Into<iroh::EndpointAddr>,
    ) -> Result<iroh::endpoint::Connection> {
        self.endpoint
            .connect(addr, FEDERATION_ALPN)
            .await
            .map_err(|e| {
                FederationError::ConnectionFailed(format!("QUIC connect failed: {e}"))
            })
    }

    /// Get a reference to the underlying iroh endpoint.
    pub fn endpoint(&self) -> &iroh::Endpoint {
        &self.endpoint
    }

    /// Graceful shutdown — closes all connections and releases the port.
    pub async fn shutdown(&self) -> Result<()> {
        self.endpoint.close().await;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Length-prefixed message protocol
// ---------------------------------------------------------------------------

/// Send a length-prefixed message on a QUIC send stream.
///
/// Wire format: `[4 bytes big-endian length][payload bytes]`
///
/// The stream is NOT finished after sending — caller can send multiple
/// messages. Call `send.finish()` when done.
pub async fn send_message(
    send: &mut iroh::endpoint::SendStream,
    data: &[u8],
) -> Result<()> {
    if data.len() > MAX_MESSAGE_SIZE {
        return Err(FederationError::TransportError(format!(
            "Message too large: {} > {MAX_MESSAGE_SIZE}",
            data.len()
        )));
    }

    let len = (data.len() as u32).to_be_bytes();
    send.write_all(&len)
        .await
        .map_err(|e| FederationError::TransportError(format!("Write length failed: {e}")))?;
    send.write_all(data)
        .await
        .map_err(|e| FederationError::TransportError(format!("Write payload failed: {e}")))?;
    Ok(())
}

/// Receive a length-prefixed message from a QUIC recv stream.
///
/// Returns the payload bytes. Fails loudly if the message exceeds
/// `MAX_MESSAGE_SIZE` or if the stream is closed prematurely.
pub async fn recv_message(
    recv: &mut iroh::endpoint::RecvStream,
) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf)
        .await
        .map_err(|e| FederationError::TransportError(format!("Read length failed: {e}")))?;

    let msg_len = u32::from_be_bytes(len_buf) as usize;

    if msg_len > MAX_MESSAGE_SIZE {
        return Err(FederationError::TransportError(format!(
            "Message too large: {msg_len} > {MAX_MESSAGE_SIZE}"
        )));
    }

    if msg_len == 0 {
        return Ok(Vec::new());
    }

    let mut buf = vec![0u8; msg_len];
    recv.read_exact(&mut buf)
        .await
        .map_err(|e| FederationError::TransportError(format!("Read payload failed: {e}")))?;

    Ok(buf)
}

/// Send a message and finish the stream (request-response pattern).
///
/// Use when sending a single request or response on a stream.
pub async fn send_message_finish(
    mut send: iroh::endpoint::SendStream,
    data: &[u8],
) -> Result<()> {
    send_message(&mut send, data).await?;
    send.finish()
        .map_err(|e| FederationError::TransportError(format!("Stream finish failed: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_key_conversion() {
        let identity = TeambookIdentity::generate();
        let iroh_key = identity_to_iroh_key(&identity);

        // The iroh public key should match the identity's public key bytes
        let iroh_pubkey = iroh_key.public();
        let iroh_pubkey_bytes = iroh_pubkey.as_bytes();
        let identity_pubkey_bytes = identity.verifying_key().as_bytes();
        assert_eq!(iroh_pubkey_bytes, identity_pubkey_bytes);
    }

    #[test]
    fn test_alpn_is_valid() {
        assert!(!FEDERATION_ALPN.is_empty());
        assert!(FEDERATION_ALPN.len() < 256); // ALPN length fits in u8
        // ALPN should be valid UTF-8 for debugging
        assert!(std::str::from_utf8(FEDERATION_ALPN).is_ok());
    }

    /// Integration test: two endpoints connect via QUIC and exchange a message.
    ///
    /// Requires a real network environment with working QUIC/UDP between local
    /// endpoints. Ignored by default because iroh's address resolution pipeline
    /// doesn't work correctly in WSL2 (Windows test binary, virtual adapters).
    /// Run explicitly with: `cargo test test_transport_bind_and_connect -- --ignored`
    #[tokio::test]
    #[ignore = "requires native network stack (not WSL2) for QUIC loopback"]
    async fn test_transport_bind_and_connect() {
        use iroh::address_lookup::memory::MemoryLookup;

        let identity_a = TeambookIdentity::generate();
        let identity_b = TeambookIdentity::generate();

        // Shared in-memory address lookup — replaces relay for local tests
        let address_lookup = MemoryLookup::new();

        // Create endpoints without relay (no internet needed for local tests)
        let ep_a = iroh::Endpoint::empty_builder(iroh::RelayMode::Disabled)
            .secret_key(identity_to_iroh_key(&identity_a))
            .alpns(vec![FEDERATION_ALPN.to_vec()])
            .address_lookup(address_lookup.clone())
            .bind()
            .await
            .unwrap();

        let ep_b = iroh::Endpoint::empty_builder(iroh::RelayMode::Disabled)
            .secret_key(identity_to_iroh_key(&identity_b))
            .alpns(vec![FEDERATION_ALPN.to_vec()])
            .address_lookup(address_lookup.clone())
            .bind()
            .await
            .unwrap();

        // Verify endpoint IDs are different
        assert_ne!(ep_a.id(), ep_b.id());

        // Get B's full address
        let addr_b = ep_b.addr();

        // Connect and accept concurrently
        let (conn_a_result, incoming_b) = tokio::join!(
            ep_a.connect(addr_b, FEDERATION_ALPN),
            ep_b.accept()
        );
        let conn_a = conn_a_result.unwrap();
        let conn_b = incoming_b.unwrap().await.unwrap();

        // Exchange a message: A sends to B via bidirectional stream
        let msg = b"Hello from Teambook A!";

        let send_fut = async {
            let (mut send, _recv) = conn_a.open_bi().await.unwrap();
            send_message(&mut send, msg).await.unwrap();
            send.finish().unwrap();
        };

        let recv_fut = async {
            let (_send, mut recv) = conn_b.accept_bi().await.unwrap();
            recv_message(&mut recv).await.unwrap()
        };

        let (_, received) = tokio::join!(send_fut, recv_fut);
        assert_eq!(received.as_slice(), msg);

        // Explicit shutdown
        ep_a.close().await;
        ep_b.close().await;
    }
}
