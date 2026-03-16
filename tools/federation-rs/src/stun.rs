//! STUN hole punching for NAT traversal (RFC 5389 / 8489).
//!
//! Enables QUIC connections between federation nodes behind NATs by:
//!
//! 1. **STUN probing** — each node sends a `Binding Request` to a public STUN
//!    server over UDP. The server's `XOR-MAPPED-ADDRESS` response reveals the
//!    node's external IP:port as seen on the public internet.
//!
//! 2. **Address exchange** — nodes share their external addresses through an
//!    already-established relay path (relay endpoint or connect-code flow).
//!
//! 3. **Simultaneous punch** — both nodes send UDP probe packets to each
//!    other's external address at the same time. This causes each NAT to create
//!    an inbound-permitting mapping for the other peer's external address.
//!
//! 4. **QUIC upgrade** — the caller passes the now-punched UDP socket to
//!    `quinn::Endpoint` to complete the QUIC handshake through the open path.
//!
//! # NAT type caveats
//!
//! Symmetric NATs (rare, but used by some mobile carriers) assign a *different*
//! external port for each destination. STUN against a third-party server won't
//! predict the correct port for a peer connection. When detected,
//! [`HolePunchResult::SymmetricNat`] is returned and the caller should fall
//! back to relay.
//!
//! # STUN message format (RFC 5389 §6)
//!
//! ```text
//!  0                   1                   2                   3
//!  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |0 0|     STUN Message Type     |         Message Length        |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |                    Magic Cookie = 0x2112A442                  |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |                     Transaction ID (96 bits)                  |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! ```

use crate::{Endpoint, FederationError, Result};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
#[cfg(test)]
use std::net::IpAddr;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::timeout;

// ---------------------------------------------------------------------------
// STUN protocol constants (RFC 5389 §6, §15)
// ---------------------------------------------------------------------------

/// STUN magic cookie — present in all RFC 5389+ messages at bytes 4-7.
const MAGIC_COOKIE: u32 = 0x2112_A442;

/// STUN Binding Request message type.
const BINDING_REQUEST: u16 = 0x0001;

/// STUN Binding Success Response message type.
const BINDING_RESPONSE: u16 = 0x0101;

/// STUN Binding Error Response message type.
const BINDING_ERROR: u16 = 0x0111;

/// `MAPPED-ADDRESS` attribute type (RFC 3489 legacy, no XOR).
const ATTR_MAPPED_ADDRESS: u16 = 0x0001;

/// `XOR-MAPPED-ADDRESS` attribute type (RFC 5389 preferred).
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;

/// Fixed size of the STUN message header in bytes.
const STUN_HEADER_LEN: usize = 20;

/// Address family: IPv4.
const FAMILY_IPV4: u8 = 0x01;

/// Address family: IPv6.
const FAMILY_IPV6: u8 = 0x02;

/// UDP probe payload sent during hole punching.
const PUNCH_PAYLOAD: &[u8] = b"TEAMBOOK-PUNCH-v1";

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The external (NAT-mapped) address returned by a STUN server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StunMappedAddress {
    /// External socket address as observed by the STUN server.
    pub external_addr: SocketAddr,

    /// Which STUN server produced this mapping.
    pub via_server: SocketAddr,
}

/// Result of a UDP hole-punch attempt.
#[derive(Debug, Clone)]
pub enum HolePunchResult {
    /// Hole punched successfully — QUIC can proceed.
    ///
    /// Pass `local_socket_addr` to `quinn` when binding its endpoint so that
    /// the OS maps the QUIC connection through the same NAT entry.
    Success {
        /// The peer's external address (now reachable through the punched hole).
        peer_addr: SocketAddr,
        /// Our bound local socket address (the port STUN observed).
        local_socket_addr: SocketAddr,
    },

    /// Symmetric NAT detected — external port varies per destination.
    ///
    /// STUN-predicted address won't work. Fall back to relay.
    SymmetricNat,

    /// Punch attempt timed out — peer's probes never arrived.
    TimedOut,

    /// IO error during the punch sequence.
    Error(String),
}

/// Configuration for STUN probing and hole punching.
#[derive(Debug, Clone)]
pub struct HolePunchConfig {
    /// STUN servers to try, in order. First responsive one wins.
    pub stun_servers: Vec<SocketAddr>,

    /// Timeout for each individual STUN server response.
    pub stun_timeout: Duration,

    /// Number of UDP probe bursts to send to the peer during punching.
    ///
    /// More bursts improve reliability against packet loss, at the cost of
    /// a slightly longer punch sequence.
    pub punch_probes: u8,

    /// Delay between consecutive probe bursts.
    pub probe_interval: Duration,

    /// Total time to wait for the peer's return probe after sending ours.
    pub punch_timeout: Duration,
}

impl Default for HolePunchConfig {
    fn default() -> Self {
        Self {
            stun_servers: default_stun_servers(),
            stun_timeout: Duration::from_secs(3),
            punch_probes: 5,
            probe_interval: Duration::from_millis(200),
            punch_timeout: Duration::from_secs(10),
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Probe STUN servers to discover our external (NAT-mapped) address.
///
/// Queries servers in order, stopping at the first successful response.
/// If `check_symmetric` is true, queries two servers and compares their
/// responses; if the external port differs, the NAT is symmetric and an
/// error is returned.
///
/// The `socket` must remain bound to the same local port for the entire
/// hole-punch sequence — the NAT mapping is port-specific.
pub async fn probe_external_address(
    socket: &UdpSocket,
    config: &HolePunchConfig,
    check_symmetric: bool,
) -> Result<StunMappedAddress> {
    let mut first: Option<StunMappedAddress> = None;

    for &server in &config.stun_servers {
        let result = timeout(
            config.stun_timeout,
            send_binding_request(socket, server),
        )
        .await;

        match result {
            Ok(Ok(mapped)) => {
                if let Some(ref prev) = first {
                    if check_symmetric && prev.external_addr.port() != mapped.external_addr.port() {
                        return Err(FederationError::TransportError(
                            "symmetric NAT: external port differs between STUN servers".to_string(),
                        ));
                    }
                    // Two servers agree — high confidence.
                    return Ok(mapped);
                } else {
                    first = Some(mapped);
                    if !check_symmetric {
                        return Ok(mapped);
                    }
                    // Continue to query a second server for symmetric NAT check.
                }
            }
            Ok(Err(e)) => {
                tracing::debug!("STUN server {} failed: {}", server, e);
            }
            Err(_) => {
                tracing::debug!("STUN server {} timed out", server);
            }
        }
    }

    first.ok_or_else(|| {
        FederationError::TransportError(
            "all STUN servers failed or timed out".to_string(),
        )
    })
}

/// Perform simultaneous UDP hole punching to a peer's external address.
///
/// **Both peers must call this function at approximately the same time.**
/// The caller is responsible for:
///   1. Exchanging external addresses through a relay/signaling channel.
///   2. Coordinating the punch start time (e.g. both punch after ACKing each
///      other's address).
///
/// The function sends `config.punch_probes` UDP probe bursts to
/// `peer_external_addr`, then waits up to `config.punch_timeout` for the
/// peer's probes to arrive. On [`HolePunchResult::Success`], the caller should
/// pass `socket` to `quinn::Endpoint` (same bound port = same NAT mapping).
pub async fn punch_hole(
    socket: &UdpSocket,
    peer_external_addr: SocketAddr,
    config: &HolePunchConfig,
) -> HolePunchResult {
    // Send probe bursts. Each burst:
    // - Causes our NAT to create (or refresh) an outbound mapping for peer_addr.
    // - If peer is already punching, some probes will arrive at their NAT and
    //   create the inbound-permitting entry on their side.
    for burst in 0..config.punch_probes {
        if let Err(e) = socket.send_to(PUNCH_PAYLOAD, peer_external_addr).await {
            return HolePunchResult::Error(format!("probe burst {}: {}", burst, e));
        }
        if burst + 1 < config.punch_probes {
            tokio::time::sleep(config.probe_interval).await;
        }
    }

    // Wait for the peer's probes to punch through their NAT to us.
    let mut buf = [0u8; 64];
    match timeout(config.punch_timeout, socket.recv_from(&mut buf)).await {
        Ok(Ok((_n, from))) if from == peer_external_addr => {
            let local_socket_addr = match socket.local_addr() {
                Ok(a) => a,
                Err(e) => {
                    return HolePunchResult::Error(format!("local_addr: {}", e));
                }
            };
            tracing::info!(
                "hole punched: peer={} local={}",
                peer_external_addr,
                local_socket_addr,
            );
            HolePunchResult::Success {
                peer_addr: peer_external_addr,
                local_socket_addr,
            }
        }
        Ok(Ok((_n, from))) => {
            // Received from unexpected source — the hole isn't punched yet.
            tracing::warn!("punch: received probe from unexpected source {}", from);
            HolePunchResult::TimedOut
        }
        Ok(Err(e)) => HolePunchResult::Error(format!("recv: {}", e)),
        Err(_) => HolePunchResult::TimedOut,
    }
}

/// Convert a successful [`HolePunchResult`] to an [`Endpoint::Quic`].
///
/// Returns `None` for non-success results. The returned endpoint can be stored
/// in a [`crate::discovery::DiscoveredPeer`] or [`crate::FederationConnection`].
pub fn hole_punch_to_endpoint(result: &HolePunchResult) -> Option<Endpoint> {
    match result {
        HolePunchResult::Success { peer_addr, .. } => Some(Endpoint::quic(*peer_addr)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// STUN message encoding / decoding
// ---------------------------------------------------------------------------

/// Send a STUN Binding Request to `server` and return the mapped address.
async fn send_binding_request(
    socket: &UdpSocket,
    server: SocketAddr,
) -> Result<StunMappedAddress> {
    let txn_id = random_transaction_id();
    let request = encode_binding_request(txn_id);

    socket
        .send_to(&request, server)
        .await
        .map_err(|e| FederationError::TransportError(format!("STUN send: {}", e)))?;

    let mut buf = [0u8; 512];
    let (n, from) = socket
        .recv_from(&mut buf)
        .await
        .map_err(|e| FederationError::TransportError(format!("STUN recv: {}", e)))?;

    if from != server {
        return Err(FederationError::TransportError(format!(
            "STUN response from unexpected source {} (expected {})",
            from, server
        )));
    }

    let external_addr = decode_binding_response(&buf[..n], txn_id)?;
    Ok(StunMappedAddress { external_addr, via_server: server })
}

/// Encode a STUN Binding Request header (20 bytes, no attributes).
fn encode_binding_request(txn_id: [u8; 12]) -> [u8; 20] {
    let mut msg = [0u8; 20];

    // Message type: Binding Request
    msg[0] = (BINDING_REQUEST >> 8) as u8;
    msg[1] = (BINDING_REQUEST & 0xFF) as u8;

    // Message length: 0 (no attributes)
    // msg[2..4] already zero.

    // Magic cookie
    msg[4] = 0x21;
    msg[5] = 0x12;
    msg[6] = 0xA4;
    msg[7] = 0x42;

    // Transaction ID
    msg[8..20].copy_from_slice(&txn_id);

    msg
}

/// Decode a STUN Binding Response, extracting the mapped external address.
///
/// Validates: message type, magic cookie, transaction ID, attribute presence.
/// Prefers `XOR-MAPPED-ADDRESS`; falls back to `MAPPED-ADDRESS` for RFC 3489
/// servers that don't include the XOR variant.
pub(crate) fn decode_binding_response(buf: &[u8], expected_txn: [u8; 12]) -> Result<SocketAddr> {
    if buf.len() < STUN_HEADER_LEN {
        return Err(FederationError::TransportError(
            "STUN response too short".to_string(),
        ));
    }

    let msg_type = u16::from_be_bytes([buf[0], buf[1]]);
    if msg_type == BINDING_ERROR {
        return Err(FederationError::TransportError(
            "STUN server returned error response".to_string(),
        ));
    }
    if msg_type != BINDING_RESPONSE {
        return Err(FederationError::TransportError(format!(
            "unexpected STUN message type: 0x{:04X}",
            msg_type
        )));
    }

    let cookie = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    if cookie != MAGIC_COOKIE {
        return Err(FederationError::TransportError(
            "STUN magic cookie mismatch (not RFC 5389?)".to_string(),
        ));
    }

    if buf[8..20] != expected_txn {
        return Err(FederationError::TransportError(
            "STUN response transaction ID mismatch".to_string(),
        ));
    }

    let attr_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    if buf.len() < STUN_HEADER_LEN + attr_len {
        return Err(FederationError::TransportError(
            "STUN response truncated (attr_len exceeds buffer)".to_string(),
        ));
    }

    // Walk attributes — prefer XOR-MAPPED-ADDRESS, fall back to MAPPED-ADDRESS.
    let txn_id = &buf[8..20];
    let attrs = &buf[STUN_HEADER_LEN..STUN_HEADER_LEN + attr_len];
    let mut xor_mapped: Option<SocketAddr> = None;
    let mut plain_mapped: Option<SocketAddr> = None;
    let mut pos = 0;

    while pos + 4 <= attrs.len() {
        let attr_type = u16::from_be_bytes([attrs[pos], attrs[pos + 1]]);
        let value_len = u16::from_be_bytes([attrs[pos + 2], attrs[pos + 3]]) as usize;
        pos += 4;

        if pos + value_len > attrs.len() {
            break; // Malformed — stop walking.
        }

        let value = &attrs[pos..pos + value_len];
        match attr_type {
            ATTR_XOR_MAPPED_ADDRESS => {
                if let Ok(addr) = decode_xor_mapped_address(value, txn_id) {
                    xor_mapped = Some(addr);
                }
            }
            ATTR_MAPPED_ADDRESS => {
                if let Ok(addr) = decode_plain_mapped_address(value) {
                    plain_mapped = Some(addr);
                }
            }
            _ => {} // RFC 5389 §7.3: unknown comprehension-optional attrs are ignored.
        }

        // Attributes are padded to 4-byte boundaries.
        let padded = (value_len + 3) & !3;
        pos += padded;
    }

    xor_mapped.or(plain_mapped).ok_or_else(|| {
        FederationError::TransportError(
            "STUN response contains no usable address attribute".to_string(),
        )
    })
}

/// Decode an `XOR-MAPPED-ADDRESS` attribute value (RFC 5389 §15.2).
///
/// XOR encoding removes NAT ALG interference:
/// - `port  = X-Port  XOR (magic_cookie >> 16)`
/// - `IPv4  = X-Addr  XOR magic_cookie`
/// - `IPv6  = X-Addr  XOR (magic_cookie_bytes ++ txn_id)`
fn decode_xor_mapped_address(
    value: &[u8],
    txn_id: &[u8],
) -> std::result::Result<SocketAddr, ()> {
    if value.len() < 8 {
        return Err(());
    }

    let family = value[1];
    let xport = u16::from_be_bytes([value[2], value[3]]);
    let port = xport ^ ((MAGIC_COOKIE >> 16) as u16);

    match family {
        FAMILY_IPV4 => {
            let xaddr = u32::from_be_bytes([value[4], value[5], value[6], value[7]]);
            let addr_u32 = xaddr ^ MAGIC_COOKIE;
            let ip = Ipv4Addr::from(addr_u32.to_be_bytes());
            Ok(SocketAddr::V4(SocketAddrV4::new(ip, port)))
        }
        FAMILY_IPV6 => {
            if value.len() < 20 {
                return Err(());
            }
            let mut xaddr = [0u8; 16];
            xaddr.copy_from_slice(&value[4..20]);

            // XOR first 4 bytes with magic cookie, remaining 12 with txn_id.
            let mc = MAGIC_COOKIE.to_be_bytes();
            for i in 0..4 {
                xaddr[i] ^= mc[i];
            }
            for i in 0..12 {
                xaddr[4 + i] ^= txn_id[i];
            }

            let ip = Ipv6Addr::from(xaddr);
            Ok(SocketAddr::V6(SocketAddrV6::new(ip, port, 0, 0)))
        }
        _ => Err(()),
    }
}

/// Decode a `MAPPED-ADDRESS` attribute value (RFC 3489, no XOR).
fn decode_plain_mapped_address(value: &[u8]) -> std::result::Result<SocketAddr, ()> {
    if value.len() < 8 {
        return Err(());
    }

    let family = value[1];
    let port = u16::from_be_bytes([value[2], value[3]]);

    match family {
        FAMILY_IPV4 => {
            let ip = Ipv4Addr::new(value[4], value[5], value[6], value[7]);
            Ok(SocketAddr::V4(SocketAddrV4::new(ip, port)))
        }
        FAMILY_IPV6 => {
            if value.len() < 20 {
                return Err(());
            }
            let mut bytes = [0u8; 16];
            bytes.copy_from_slice(&value[4..20]);
            let ip = Ipv6Addr::from(bytes);
            Ok(SocketAddr::V6(SocketAddrV6::new(ip, port, 0, 0)))
        }
        _ => Err(()),
    }
}

/// Generate a cryptographically random 12-byte STUN transaction ID.
fn random_transaction_id() -> [u8; 12] {
    use rand::RngCore;
    let mut id = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut id);
    id
}

/// Default STUN servers (well-known, no authentication required).
///
/// Pre-resolved to avoid DNS during critical connection paths.
/// For production deployments, consider DNS-resolving at startup and caching,
/// or configuring operator-specific STUN servers in the manifest.
fn default_stun_servers() -> Vec<SocketAddr> {
    vec![
        // stun.l.google.com:19302
        "74.125.250.129:19302".parse().unwrap(),
        // stun1.l.google.com:19302
        "74.125.197.127:19302".parse().unwrap(),
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Encoding / decoding unit tests — no network required
    // -----------------------------------------------------------------------

    #[test]
    fn test_encode_binding_request() {
        let txn: [u8; 12] = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
                              0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C];
        let req = encode_binding_request(txn);

        assert_eq!(u16::from_be_bytes([req[0], req[1]]), BINDING_REQUEST);
        assert_eq!(u16::from_be_bytes([req[2], req[3]]), 0, "no attributes");
        assert_eq!(
            u32::from_be_bytes([req[4], req[5], req[6], req[7]]),
            MAGIC_COOKIE
        );
        assert_eq!(&req[8..20], &txn, "transaction ID preserved");
    }

    #[test]
    fn test_decode_response_too_short() {
        let err = decode_binding_response(&[0u8; 10], [0u8; 12]).unwrap_err();
        assert!(err.to_string().contains("too short"), "{}", err);
    }

    #[test]
    fn test_decode_response_wrong_magic_cookie() {
        let txn = [0xAA; 12];
        let mut buf = [0u8; 20];
        buf[0] = 0x01; buf[1] = 0x01; // Binding Response
        buf[4] = 0xFF; buf[5] = 0xFF; buf[6] = 0xFF; buf[7] = 0xFF; // wrong cookie
        buf[8..20].copy_from_slice(&txn);

        let err = decode_binding_response(&buf, txn).unwrap_err();
        assert!(err.to_string().contains("magic cookie"), "{}", err);
    }

    #[test]
    fn test_decode_response_txn_id_mismatch() {
        let txn_sent = [0xAA; 12];
        let txn_recv = [0xBB; 12];
        let mut buf = [0u8; 20];
        buf[0] = 0x01; buf[1] = 0x01;
        buf[4] = 0x21; buf[5] = 0x12; buf[6] = 0xA4; buf[7] = 0x42;
        buf[8..20].copy_from_slice(&txn_recv);

        let err = decode_binding_response(&buf, txn_sent).unwrap_err();
        assert!(err.to_string().contains("transaction ID mismatch"), "{}", err);
    }

    #[test]
    fn test_decode_response_error_type() {
        let txn = [0xCC; 12];
        let mut buf = [0u8; 20];
        buf[0] = 0x01; buf[1] = 0x11; // BINDING_ERROR
        buf[4] = 0x21; buf[5] = 0x12; buf[6] = 0xA4; buf[7] = 0x42;
        buf[8..20].copy_from_slice(&txn);

        let err = decode_binding_response(&buf, txn).unwrap_err();
        assert!(err.to_string().contains("error response"), "{}", err);
    }

    /// XOR-MAPPED-ADDRESS decoding for 203.0.113.1:54321.
    #[test]
    fn test_decode_xor_mapped_address_ipv4() {
        let txn_id = [0u8; 12];

        let port: u16 = 54321;
        let xport = port ^ ((MAGIC_COOKIE >> 16) as u16);
        let addr_u32: u32 = u32::from_be_bytes([203, 0, 113, 1]);
        let xaddr = addr_u32 ^ MAGIC_COOKIE;

        let value: [u8; 8] = [
            0x00, FAMILY_IPV4,
            (xport >> 8) as u8, (xport & 0xFF) as u8,
            ((xaddr >> 24) & 0xFF) as u8,
            ((xaddr >> 16) & 0xFF) as u8,
            ((xaddr >> 8) & 0xFF) as u8,
            (xaddr & 0xFF) as u8,
        ];

        let addr = decode_xor_mapped_address(&value, &txn_id).unwrap();
        assert_eq!(addr.port(), 54321);
        assert_eq!(addr.ip(), IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1)));
    }

    #[test]
    fn test_decode_plain_mapped_address_ipv4() {
        let value: [u8; 8] = [
            0x00, FAMILY_IPV4,
            0x04, 0xD2, // port = 1234
            0x01, 0x02, 0x03, 0x04, // 1.2.3.4
        ];

        let addr = decode_plain_mapped_address(&value).unwrap();
        assert_eq!(addr.port(), 1234);
        assert_eq!(addr.ip(), IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)));
    }

    /// Full end-to-end decode of a crafted Binding Response for 1.2.3.4:5678.
    #[test]
    fn test_decode_full_binding_response_xor_mapped() {
        let txn: [u8; 12] = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
                              0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C];

        let port: u16 = 5678;
        let xport = port ^ ((MAGIC_COOKIE >> 16) as u16);
        let addr_u32: u32 = u32::from_be_bytes([1, 2, 3, 4]);
        let xaddr = addr_u32 ^ MAGIC_COOKIE;

        // header(20) + attr_header(4) + attr_value(8) = 32 bytes
        let mut buf = [0u8; 32];
        buf[0] = 0x01; buf[1] = 0x01; // Binding Response
        buf[2] = 0x00; buf[3] = 0x0C; // attr_len = 12 (4 hdr + 8 val)
        buf[4] = 0x21; buf[5] = 0x12; buf[6] = 0xA4; buf[7] = 0x42;
        buf[8..20].copy_from_slice(&txn);
        buf[20] = 0x00; buf[21] = 0x20; // XOR-MAPPED-ADDRESS
        buf[22] = 0x00; buf[23] = 0x08; // value length = 8
        buf[24] = 0x00; buf[25] = FAMILY_IPV4;
        buf[26] = (xport >> 8) as u8; buf[27] = (xport & 0xFF) as u8;
        buf[28] = ((xaddr >> 24) & 0xFF) as u8;
        buf[29] = ((xaddr >> 16) & 0xFF) as u8;
        buf[30] = ((xaddr >> 8) & 0xFF) as u8;
        buf[31] = (xaddr & 0xFF) as u8;

        let addr = decode_binding_response(&buf, txn).unwrap();
        assert_eq!(addr.port(), 5678);
        assert_eq!(addr.ip(), IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)));
    }

    /// Prefers XOR-MAPPED-ADDRESS over MAPPED-ADDRESS when both present.
    #[test]
    fn test_prefers_xor_over_plain_mapped() {
        let txn = [0u8; 12];
        let xor_port: u16 = 9999;
        let xor_addr: u32 = 0x7F000001; // 127.0.0.1

        let x_port = xor_port ^ ((MAGIC_COOKIE >> 16) as u16);
        let x_addr = xor_addr ^ MAGIC_COOKIE;

        let plain_val: [u8; 8] = [0x00, FAMILY_IPV4, 0x04, 0x57, 2, 2, 2, 2];
        let xor_val: [u8; 8] = [
            0x00, FAMILY_IPV4,
            (x_port >> 8) as u8, (x_port & 0xFF) as u8,
            ((x_addr >> 24) & 0xFF) as u8,
            ((x_addr >> 16) & 0xFF) as u8,
            ((x_addr >> 8) & 0xFF) as u8,
            (x_addr & 0xFF) as u8,
        ];

        // header + (4+8) MAPPED + (4+8) XOR = 20 + 12 + 12 = 44 bytes
        let mut buf = [0u8; 44];
        buf[0] = 0x01; buf[1] = 0x01;
        buf[2] = 0x00; buf[3] = 0x18; // attr_len = 24
        buf[4] = 0x21; buf[5] = 0x12; buf[6] = 0xA4; buf[7] = 0x42;
        buf[8..20].copy_from_slice(&txn);
        buf[20] = 0x00; buf[21] = 0x01; // MAPPED-ADDRESS
        buf[22] = 0x00; buf[23] = 0x08;
        buf[24..32].copy_from_slice(&plain_val);
        buf[32] = 0x00; buf[33] = 0x20; // XOR-MAPPED-ADDRESS
        buf[34] = 0x00; buf[35] = 0x08;
        buf[36..44].copy_from_slice(&xor_val);

        let addr = decode_binding_response(&buf, txn).unwrap();
        assert_eq!(addr.port(), 9999, "should prefer XOR-MAPPED-ADDRESS");
        assert_eq!(addr.ip(), IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
    }

    // -----------------------------------------------------------------------
    // Integration tests — loopback sockets, no external network
    // -----------------------------------------------------------------------

    /// Mock STUN server: receives one Binding Request and replies with the
    /// client's observed address in XOR-MAPPED-ADDRESS.
    async fn mock_stun_server(
        server_sock: UdpSocket,
        respond_with: Option<SocketAddr>,
    ) {
        let mut buf = [0u8; 512];
        let (n, from) = server_sock.recv_from(&mut buf).await.unwrap();
        let request = &buf[..n];

        let cookie = u32::from_be_bytes([request[4], request[5], request[6], request[7]]);
        assert_eq!(cookie, MAGIC_COOKIE);

        let mut txn = [0u8; 12];
        txn.copy_from_slice(&request[8..20]);

        let report_addr = respond_with.unwrap_or(from);
        let report_port = report_addr.port();
        let xport = report_port ^ ((MAGIC_COOKIE >> 16) as u16);
        let ip_bytes = match report_addr.ip() {
            IpAddr::V4(v4) => v4.octets(),
            _ => unreachable!("test uses IPv4 only"),
        };
        let xaddr = u32::from_be_bytes(ip_bytes) ^ MAGIC_COOKIE;

        let attr_val: [u8; 8] = [
            0x00, FAMILY_IPV4,
            (xport >> 8) as u8, (xport & 0xFF) as u8,
            ((xaddr >> 24) & 0xFF) as u8,
            ((xaddr >> 16) & 0xFF) as u8,
            ((xaddr >> 8) & 0xFF) as u8,
            (xaddr & 0xFF) as u8,
        ];

        let mut resp = [0u8; 32];
        resp[0] = 0x01; resp[1] = 0x01;
        resp[2] = 0x00; resp[3] = 0x0C;
        resp[4] = 0x21; resp[5] = 0x12; resp[6] = 0xA4; resp[7] = 0x42;
        resp[8..20].copy_from_slice(&txn);
        resp[20] = 0x00; resp[21] = 0x20;
        resp[22] = 0x00; resp[23] = 0x08;
        resp[24..32].copy_from_slice(&attr_val);

        server_sock.send_to(&resp, from).await.unwrap();
    }

    #[tokio::test]
    async fn test_probe_external_address_loopback() {
        let server_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let server_addr = server_sock.local_addr().unwrap();

        tokio::spawn(mock_stun_server(server_sock, None));

        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let client_addr = client.local_addr().unwrap();

        let config = HolePunchConfig {
            stun_servers: vec![server_addr],
            stun_timeout: Duration::from_secs(2),
            ..HolePunchConfig::default()
        };

        let mapped = probe_external_address(&client, &config, false).await.unwrap();

        assert_eq!(mapped.external_addr.port(), client_addr.port());
        assert_eq!(mapped.via_server, server_addr);
    }

    /// Two loopback sockets punch holes simultaneously — verifies the
    /// simultaneous-send / receive-and-succeed flow end-to-end.
    #[tokio::test]
    async fn test_punch_hole_loopback_simultaneous() {
        let sock_a = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let sock_b = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr_a = sock_a.local_addr().unwrap();
        let addr_b = sock_b.local_addr().unwrap();

        let config = HolePunchConfig {
            punch_probes: 3,
            probe_interval: Duration::from_millis(10),
            punch_timeout: Duration::from_secs(2),
            ..HolePunchConfig::default()
        };

        let (res_a, res_b) = tokio::join!(
            punch_hole(&sock_a, addr_b, &config),
            punch_hole(&sock_b, addr_a, &config),
        );

        assert!(
            matches!(res_a, HolePunchResult::Success { .. }),
            "sock_a result: {:?}", res_a
        );
        assert!(
            matches!(res_b, HolePunchResult::Success { .. }),
            "sock_b result: {:?}", res_b
        );

        assert_eq!(hole_punch_to_endpoint(&res_a).unwrap(), Endpoint::quic(addr_b));
        assert_eq!(hole_punch_to_endpoint(&res_b).unwrap(), Endpoint::quic(addr_a));
    }

    #[tokio::test]
    async fn test_punch_hole_timeout() {
        let sock_a = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let sock_b = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr_b = sock_b.local_addr().unwrap();

        let config = HolePunchConfig {
            punch_probes: 2,
            probe_interval: Duration::from_millis(5),
            punch_timeout: Duration::from_millis(100),
            ..HolePunchConfig::default()
        };

        let _ = sock_b; // keep alive so addr is valid
        let result = punch_hole(&sock_a, addr_b, &config).await;
        assert!(matches!(result, HolePunchResult::TimedOut), "{:?}", result);
    }

    #[test]
    fn test_hole_punch_to_endpoint_success() {
        let peer: SocketAddr = "203.0.113.1:54321".parse().unwrap();
        let local: SocketAddr = "0.0.0.0:44444".parse().unwrap();
        let result = HolePunchResult::Success {
            peer_addr: peer,
            local_socket_addr: local,
        };
        let ep = hole_punch_to_endpoint(&result).unwrap();
        assert_eq!(ep, Endpoint::quic(peer));
    }

    #[test]
    fn test_hole_punch_to_endpoint_non_success() {
        assert!(hole_punch_to_endpoint(&HolePunchResult::TimedOut).is_none());
        assert!(hole_punch_to_endpoint(&HolePunchResult::SymmetricNat).is_none());
        assert!(hole_punch_to_endpoint(
            &HolePunchResult::Error("test".into())
        ).is_none());
    }
}
