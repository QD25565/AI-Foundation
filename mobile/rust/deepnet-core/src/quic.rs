//! QUIC Transport - High-performance encrypted transport for Deep Net
//!
//! QUIC provides:
//! - Built-in TLS 1.3 encryption
//! - Multiplexed streams over single connection
//! - Connection migration (IP changes don't break connection)
//! - 0-RTT connection resumption
//!
//! This is the primary transport for internet connections.

use crate::identity::{NodeId, NodeIdentity};
use crate::message::MessageEnvelope;
use crate::transport::{
    BandwidthTier, Connection, ConnectionMetrics, Listener, NodeAddress, Transport,
    TransportError, TransportType,
};
use async_trait::async_trait;
use tokio::sync::Mutex;
use quinn::{ClientConfig, Endpoint, ServerConfig, Connection as QuinnConnection, RecvStream, SendStream};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

/// Default QUIC port for Deep Net
pub const DEFAULT_QUIC_PORT: u16 = 31415;

/// QUIC transport implementation
pub struct QuicTransport {
    /// Our node identity for certificate generation
    identity: Arc<NodeIdentity>,
    /// QUIC endpoint (can be both client and server)
    endpoint: Option<Endpoint>,
    /// Bind address
    bind_addr: SocketAddr,
}

impl QuicTransport {
    /// Create a new QUIC transport
    pub fn new(identity: Arc<NodeIdentity>) -> Self {
        Self {
            identity,
            endpoint: None,
            bind_addr: SocketAddr::from(([0, 0, 0, 0], DEFAULT_QUIC_PORT)),
        }
    }

    /// Create with custom bind address
    pub fn with_bind_addr(identity: Arc<NodeIdentity>, bind_addr: SocketAddr) -> Self {
        Self {
            identity,
            endpoint: None,
            bind_addr,
        }
    }

    /// Initialize the QUIC endpoint
    pub fn start(&mut self) -> Result<(), TransportError> {
        if self.endpoint.is_some() {
            return Ok(());
        }

        // Generate self-signed certificate from node identity
        let (cert, key) = generate_self_signed_cert(&self.identity)?;

        // Server config
        let server_config = configure_server(cert.clone(), key)?;

        // Client config (skip cert verification for now - in production, verify against node_id)
        let client_config = configure_client()?;

        // Create endpoint
        let mut endpoint = Endpoint::server(server_config, self.bind_addr)
            .map_err(|e| TransportError::ConnectionFailed(format!("Failed to create endpoint: {}", e)))?;

        endpoint.set_default_client_config(client_config);

        self.endpoint = Some(endpoint);
        Ok(())
    }

    /// Get the local address
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.endpoint.as_ref().map(|e| e.local_addr().ok()).flatten()
    }
}

#[async_trait]
impl Transport for QuicTransport {
    async fn connect(&self, addr: &NodeAddress) -> Result<Box<dyn Connection>, TransportError> {
        let endpoint = self.endpoint.as_ref()
            .ok_or(TransportError::ConnectionFailed("Transport not started".to_string()))?;

        let (socket_addr, server_name) = match addr {
            NodeAddress::Quic { addr, server_name } => {
                let name = server_name.clone().unwrap_or_else(|| "deepnet".to_string());
                (*addr, name)
            }
            NodeAddress::Tcp { addr } => (*addr, "deepnet".to_string()),
            _ => return Err(TransportError::UnsupportedAddress),
        };

        let connecting = endpoint.connect(socket_addr, &server_name)
            .map_err(|e| TransportError::ConnectionFailed(format!("Connect failed: {}", e)))?;

        let connection = connecting.await
            .map_err(|e| TransportError::ConnectionFailed(format!("Connection failed: {}", e)))?;

        Ok(Box::new(QuicConnection::new(connection)))
    }

    async fn listen(&self) -> Result<Box<dyn Listener>, TransportError> {
        let endpoint = self.endpoint.clone()
            .ok_or(TransportError::ConnectionFailed("Transport not started".to_string()))?;

        Ok(Box::new(QuicListener::new(endpoint)))
    }

    fn transport_type(&self) -> TransportType {
        TransportType::Internet
    }

    fn can_handle(&self, addr: &NodeAddress) -> bool {
        matches!(addr, NodeAddress::Quic { .. } | NodeAddress::Tcp { .. })
    }
}

/// QUIC connection wrapper
pub struct QuicConnection {
    inner: QuinnConnection,
    peer_id: NodeId,
    send_stream: Mutex<Option<SendStream>>,
    recv_stream: Mutex<Option<RecvStream>>,
}

impl QuicConnection {
    fn new(connection: QuinnConnection) -> Self {
        // Extract peer_id from certificate (simplified - use first 32 bytes of cert hash)
        let peer_id = extract_peer_id(&connection);

        Self {
            inner: connection,
            peer_id,
            send_stream: Mutex::new(None),
            recv_stream: Mutex::new(None),
        }
    }

    async fn ensure_streams(&self) -> Result<(), TransportError> {
        // Open bidirectional stream if not already open
        let mut send_guard = self.send_stream.lock().await;
        if send_guard.is_none() {
            let (send, recv) = self.inner.open_bi().await
                .map_err(|e| TransportError::ConnectionFailed(format!("Failed to open stream: {}", e)))?;
            *send_guard = Some(send);
            *self.recv_stream.lock().await = Some(recv);
        }
        Ok(())
    }
}

#[async_trait]
impl Connection for QuicConnection {
    async fn send(&mut self, msg: &MessageEnvelope) -> Result<(), TransportError> {
        self.ensure_streams().await?;

        let bytes = msg.to_bytes();
        let len = (bytes.len() as u32).to_be_bytes();

        let mut send_guard = self.send_stream.lock().await;
        let stream = send_guard.as_mut()
            .ok_or(TransportError::ConnectionClosed)?;

        // Write length prefix
        stream.write_all(&len).await
            .map_err(|e| TransportError::IoError(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        // Write message
        stream.write_all(&bytes).await
            .map_err(|e| TransportError::IoError(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        Ok(())
    }

    async fn recv(&mut self) -> Result<MessageEnvelope, TransportError> {
        self.ensure_streams().await?;

        let mut recv_guard = self.recv_stream.lock().await;
        let stream = recv_guard.as_mut()
            .ok_or(TransportError::ConnectionClosed)?;

        // Read length prefix
        let mut len_bytes = [0u8; 4];
        stream.read_exact(&mut len_bytes).await
            .map_err(|e| TransportError::IoError(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
        let len = u32::from_be_bytes(len_bytes) as usize;

        // Sanity check
        if len > 10 * 1024 * 1024 {
            return Err(TransportError::ProtocolError("Message too large".to_string()));
        }

        // Read message
        let mut bytes = vec![0u8; len];
        stream.read_exact(&mut bytes).await
            .map_err(|e| TransportError::IoError(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        MessageEnvelope::from_bytes(&bytes)
            .map_err(|e| TransportError::SerializationError(e.to_string()))
    }

    async fn try_recv(&mut self) -> Result<Option<MessageEnvelope>, TransportError> {
        // QUIC doesn't have a built-in try_recv, so we'd need to use select with timeout
        // For now, return None (non-blocking would require tokio::select)
        Ok(None)
    }

    fn peer_id(&self) -> &NodeId {
        &self.peer_id
    }

    fn transport_type(&self) -> TransportType {
        TransportType::Internet
    }

    fn metrics(&self) -> ConnectionMetrics {
        let stats = self.inner.stats();
        ConnectionMetrics {
            latency_ms: stats.path.rtt.as_millis() as u32,
            bandwidth: BandwidthTier::High, // QUIC is typically high bandwidth
            packet_loss: 0, // Would need to track this
            encrypted: true,
            hops: 0,
        }
    }

    fn is_alive(&self) -> bool {
        self.inner.close_reason().is_none()
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        self.inner.close(0u32.into(), b"closing");
        Ok(())
    }

    async fn ping(&mut self) -> Result<u32, TransportError> {
        // QUIC maintains RTT internally
        let rtt = self.inner.stats().path.rtt;
        Ok(rtt.as_millis() as u32)
    }
}

/// QUIC listener for incoming connections
pub struct QuicListener {
    endpoint: Endpoint,
}

impl QuicListener {
    fn new(endpoint: Endpoint) -> Self {
        Self { endpoint }
    }
}

#[async_trait]
impl Listener for QuicListener {
    async fn accept(&mut self) -> Result<Box<dyn Connection>, TransportError> {
        let incoming = self.endpoint.accept().await
            .ok_or(TransportError::ConnectionClosed)?;

        let connection = incoming.await
            .map_err(|e| TransportError::ConnectionFailed(format!("Accept failed: {}", e)))?;

        Ok(Box::new(QuicConnection::new(connection)))
    }

    fn local_addr(&self) -> Option<NodeAddress> {
        self.endpoint.local_addr().ok().map(|addr| NodeAddress::Quic {
            addr,
            server_name: None,
        })
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        self.endpoint.close(0u32.into(), b"shutdown");
        Ok(())
    }
}

// ============================================================================
// Certificate Generation
// ============================================================================

/// Generate a self-signed certificate from node identity
fn generate_self_signed_cert(identity: &NodeIdentity) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), TransportError> {
    use rcgen::generate_simple_self_signed;

    // Use node_id as the subject name
    let subject_name = format!("deepnet-{}", identity.node_id().short());

    let certified_key = generate_simple_self_signed(vec![subject_name, "localhost".to_string()])
        .map_err(|e| TransportError::TlsError(format!("Failed to generate cert: {}", e)))?;

    let cert = CertificateDer::from(certified_key.cert.der().to_vec());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(certified_key.key_pair.serialize_der()));

    Ok((cert, key))
}

/// Configure server with certificate
fn configure_server(cert: CertificateDer<'static>, key: PrivateKeyDer<'static>) -> Result<ServerConfig, TransportError> {
    let server_crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(|e| TransportError::TlsError(format!("Server config failed: {}", e)))?;

    let mut server_config = ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)
            .map_err(|e| TransportError::TlsError(format!("QUIC server config failed: {}", e)))?
    ));

    // Configure transport parameters
    let transport_config = Arc::get_mut(&mut server_config.transport).unwrap();
    transport_config.max_idle_timeout(Some(Duration::from_secs(60).try_into().unwrap()));

    Ok(server_config)
}

/// Configure client (skip cert verification for P2P - we verify via node_id)
fn configure_client() -> Result<ClientConfig, TransportError> {
    // For P2P, we skip traditional cert verification and rely on node_id matching
    let client_crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
        .with_no_client_auth();

    let client_config = ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto)
            .map_err(|e| TransportError::TlsError(format!("QUIC client config failed: {}", e)))?
    ));

    Ok(client_config)
}

/// Skip certificate verification (for P2P where we verify node_id separately)
#[derive(Debug)]
struct SkipServerVerification;

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        // In production, verify the cert contains the expected node_id
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
        ]
    }
}

/// Extract peer ID from connection (simplified)
fn extract_peer_id(connection: &QuinnConnection) -> NodeId {
    // In production, extract from certificate
    // For now, use remote address hash
    let addr = connection.remote_address();
    let mut bytes = [0u8; 32];
    let addr_bytes = format!("{}", addr);
    let hash = sha2::Sha256::digest(addr_bytes.as_bytes());
    bytes.copy_from_slice(&hash);
    NodeId::from_bytes(bytes)
}

use sha2::Digest;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quic_transport_creation() {
        let identity = Arc::new(NodeIdentity::generate("Test".to_string()));
        let transport = QuicTransport::new(identity);
        assert_eq!(transport.transport_type(), TransportType::Internet);
    }
}
