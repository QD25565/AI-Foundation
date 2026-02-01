//! AFP Client
//!
//! Client for connecting to AFP servers (teambooks).
//! Handles authentication, message sending/receiving, and reconnection.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::error::{AFPError, Result};
use crate::fingerprint::HardwareFingerprint;
use crate::identity::{AIIdentity, TrustLevel};
use crate::keys::{FallbackStorage, KeyPair, KeyStorage};
use crate::message::{AFPMessage, MessageType, Payload};
use crate::transport::{ConnectionState, QuicTransport, Transport, WebSocketTransport};

/// AFP Client
pub struct AFPClient {
    ai_id: String,
    identity: Option<AIIdentity>,
    keypair: KeyPair,
    fingerprint: HardwareFingerprint,
    transport: Option<Box<dyn Transport>>,
    trust_level: TrustLevel,
    teambook_name: Option<String>,
    teambook_id: Option<String>,
}

impl AFPClient {
    /// Create a new AFP client
    pub fn new(ai_id: &str) -> Result<Self> {
        // Initialize key storage
        let storage = FallbackStorage::default_chain(ai_id);

        // Generate or load key pair
        let keypair = if storage.exists(ai_id) {
            info!("Loading existing key for {}", ai_id);
            storage.load(ai_id)?
        } else {
            info!("Generating new key for {}", ai_id);
            storage.generate_and_store(ai_id)?;
            storage.load(ai_id)?
        };

        // Collect hardware fingerprint
        let fingerprint = HardwareFingerprint::collect()?;
        info!("Hardware fingerprint: {}", fingerprint.short_hash());

        // Create identity
        let identity = AIIdentity::new(
            ai_id.to_string(),
            keypair.public_key(),
            "unconnected".to_string(),
        );

        Ok(Self {
            ai_id: ai_id.to_string(),
            identity: Some(identity),
            keypair,
            fingerprint,
            transport: None,
            trust_level: TrustLevel::Anonymous,
            teambook_name: None,
            teambook_id: None,
        })
    }

    /// Connect to a teambook server
    pub async fn connect(&mut self, addr: SocketAddr, use_websocket: bool) -> Result<()> {
        info!("Connecting to {} ({})", addr, if use_websocket { "WebSocket" } else { "QUIC" });

        // Create transport
        let mut transport: Box<dyn Transport> = if use_websocket {
            Box::new(WebSocketTransport::new())
        } else {
            Box::new(QuicTransport::new())
        };

        // Connect
        transport.connect(addr).await?;
        info!("Transport connected");

        // Update fingerprint with teambook salt (for privacy)
        self.fingerprint.compute_hash(Some(&addr.to_string()));

        // Send Hello
        let identity = self.identity.as_ref().ok_or(AFPError::Internal("No identity".to_string()))?;
        let mut hello = AFPMessage::new(
            MessageType::Request,
            identity,
            None,
            Payload::Hello {
                fingerprint: self.fingerprint.clone(),
                capabilities: vec!["afp-v1".to_string()],
                requested_trust: TrustLevel::Verified,
            },
        );
        hello.sign(&self.keypair)?;
        transport.send(&hello).await?;
        info!("Sent Hello");

        // Wait for Welcome/Rejected
        let response = transport.recv().await?;
        response.verify()?;

        match response.payload {
            Payload::Welcome {
                trust_level,
                teambook_name,
                teambook_id,
                server_version,
            } => {
                info!(
                    "Connected to teambook '{}' (v{}) at trust level {:?}",
                    teambook_name, server_version, trust_level
                );
                self.trust_level = trust_level;
                self.teambook_name = Some(teambook_name.clone());
                self.teambook_id = Some(teambook_id);

                // Update identity with teambook
                if let Some(ref mut id) = self.identity {
                    id.teambook = teambook_name;
                    id.trust_level = trust_level;
                }
            }
            Payload::Rejected { reason, banned } => {
                error!("Connection rejected: {} (banned: {})", reason, banned);
                return Err(AFPError::AuthenticationFailed(reason));
            }
            _ => {
                return Err(AFPError::HandshakeFailed(
                    "Unexpected response to Hello".to_string(),
                ));
            }
        }

        self.transport = Some(transport);
        Ok(())
    }

    /// Send a direct message to another AI
    pub async fn send_dm(&mut self, to_ai: &str, content: &str) -> Result<()> {
        let transport = self.transport.as_mut().ok_or(AFPError::ConnectionClosed)?;
        let identity = self.identity.as_ref().ok_or(AFPError::Internal("No identity".to_string()))?;

        let mut msg = AFPMessage::new(
            MessageType::Notification,
            identity,
            Some(to_ai.to_string()),
            Payload::DirectMessage {
                content: content.to_string(),
            },
        );
        msg.sign(&self.keypair)?;
        transport.send(&msg).await
    }

    /// Broadcast a message to a channel
    pub async fn broadcast(&mut self, channel: &str, content: &str) -> Result<()> {
        let transport = self.transport.as_mut().ok_or(AFPError::ConnectionClosed)?;
        let identity = self.identity.as_ref().ok_or(AFPError::Internal("No identity".to_string()))?;

        let mut msg = AFPMessage::new(
            MessageType::Broadcast,
            identity,
            None,
            Payload::Broadcast {
                channel: channel.to_string(),
                content: content.to_string(),
            },
        );
        msg.sign(&self.keypair)?;
        transport.send(&msg).await
    }

    /// Send a ping and measure latency
    pub async fn ping(&mut self) -> Result<u64> {
        let transport = self.transport.as_mut().ok_or(AFPError::ConnectionClosed)?;
        let identity = self.identity.as_ref().ok_or(AFPError::Internal("No identity".to_string()))?;

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let mut msg = AFPMessage::new(
            MessageType::Request,
            identity,
            None,
            Payload::Ping { timestamp },
        );
        msg.sign(&self.keypair)?;
        transport.send(&msg).await?;

        // Wait for pong
        let response = transport.recv().await?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        match response.payload {
            Payload::Pong {
                request_timestamp,
                response_timestamp,
            } => {
                let latency = now - request_timestamp;
                Ok(latency)
            }
            _ => Err(AFPError::ReceiveFailed("Expected Pong".to_string())),
        }
    }

    /// Receive next message
    pub async fn recv(&mut self) -> Result<AFPMessage> {
        let transport = self.transport.as_mut().ok_or(AFPError::ConnectionClosed)?;
        let msg = transport.recv().await?;
        msg.verify()?;
        Ok(msg)
    }

    /// Close connection
    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(mut transport) = self.transport.take() {
            transport.close().await?;
        }
        self.teambook_name = None;
        self.teambook_id = None;
        self.trust_level = TrustLevel::Anonymous;
        Ok(())
    }

    /// Get current trust level
    pub fn trust_level(&self) -> TrustLevel {
        self.trust_level
    }

    /// Get AI ID
    pub fn ai_id(&self) -> &str {
        &self.ai_id
    }

    /// Get connected teambook name
    pub fn teambook(&self) -> Option<&str> {
        self.teambook_name.as_deref()
    }

    /// Check if connected
    pub fn is_connected(&self) -> bool {
        self.transport.is_some()
    }

    /// Get identity fingerprint
    pub fn fingerprint(&self) -> String {
        self.identity
            .as_ref()
            .map(|i| i.fingerprint())
            .unwrap_or_else(|| "unknown".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = AFPClient::new("test-client-123").unwrap();
        assert_eq!(client.ai_id(), "test-client-123");
        assert!(!client.is_connected());
        assert_eq!(client.trust_level(), TrustLevel::Anonymous);
    }
}
