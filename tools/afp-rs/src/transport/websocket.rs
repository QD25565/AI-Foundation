//! WebSocket Transport
//!
//! Fallback transport for environments where QUIC is blocked.
//! Uses tokio-tungstenite for WebSocket support.
//!
//! Less efficient than QUIC but more firewall-friendly.

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{
    accept_async, connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream,
};

use super::{ConnectionState, Transport, TransportServer};
use crate::error::{AFPError, Result};
use crate::message::AFPMessage;
use crate::MAX_MESSAGE_SIZE;

/// WebSocket client/connection transport
pub struct WebSocketTransport {
    stream: Option<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    server_stream: Option<WebSocketStream<TcpStream>>,
    state: ConnectionState,
    remote_addr: Option<SocketAddr>,
    local_addr: Option<SocketAddr>,
}

impl WebSocketTransport {
    pub fn new() -> Self {
        Self {
            stream: None,
            server_stream: None,
            state: ConnectionState::Disconnected,
            remote_addr: None,
            local_addr: None,
        }
    }

    /// Create from an accepted connection (server side)
    pub fn from_accepted(
        stream: WebSocketStream<TcpStream>,
        remote_addr: SocketAddr,
        local_addr: SocketAddr,
    ) -> Self {
        Self {
            stream: None,
            server_stream: Some(stream),
            state: ConnectionState::Connected,
            remote_addr: Some(remote_addr),
            local_addr: Some(local_addr),
        }
    }
}

impl Default for WebSocketTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Transport for WebSocketTransport {
    fn name(&self) -> &'static str {
        "WebSocket"
    }

    async fn connect(&mut self, addr: SocketAddr) -> Result<()> {
        self.state = ConnectionState::Connecting;

        let url = format!("ws://{}/afp", addr);
        let (stream, _response) = connect_async(&url)
            .await
            .map_err(|e| AFPError::ConnectionFailed(e.to_string()))?;

        self.stream = Some(stream);
        self.remote_addr = Some(addr);
        self.state = ConnectionState::Connected;

        Ok(())
    }

    async fn send(&mut self, message: &AFPMessage) -> Result<()> {
        let data = message.to_cbor()?;

        // Use binary WebSocket message
        let ws_message = Message::Binary(data);

        if let Some(stream) = &mut self.stream {
            stream
                .send(ws_message)
                .await
                .map_err(|e| AFPError::SendFailed(e.to_string()))?;
        } else if let Some(stream) = &mut self.server_stream {
            stream
                .send(ws_message)
                .await
                .map_err(|e| AFPError::SendFailed(e.to_string()))?;
        } else {
            return Err(AFPError::ConnectionClosed);
        }

        Ok(())
    }

    async fn recv(&mut self) -> Result<AFPMessage> {
        let msg = if let Some(stream) = &mut self.stream {
            stream
                .next()
                .await
                .ok_or(AFPError::ConnectionClosed)?
                .map_err(|e| AFPError::ReceiveFailed(e.to_string()))?
        } else if let Some(stream) = &mut self.server_stream {
            stream
                .next()
                .await
                .ok_or(AFPError::ConnectionClosed)?
                .map_err(|e| AFPError::ReceiveFailed(e.to_string()))?
        } else {
            return Err(AFPError::ConnectionClosed);
        };

        match msg {
            Message::Binary(data) => {
                if data.len() > MAX_MESSAGE_SIZE {
                    return Err(AFPError::MessageTooLarge {
                        size: data.len(),
                        max: MAX_MESSAGE_SIZE,
                    });
                }
                AFPMessage::from_cbor(&data)
            }
            Message::Close(_) => Err(AFPError::ConnectionClosed),
            Message::Ping(_) | Message::Pong(_) => {
                // Handle ping/pong automatically, recurse for next real message
                Box::pin(self.recv()).await
            }
            _ => Err(AFPError::ReceiveFailed("Unexpected message type".to_string())),
        }
    }

    async fn close(&mut self) -> Result<()> {
        if let Some(mut stream) = self.stream.take() {
            let _ = stream.close(None).await;
        }
        if let Some(mut stream) = self.server_stream.take() {
            let _ = stream.close(None).await;
        }
        self.state = ConnectionState::Disconnected;
        Ok(())
    }

    fn state(&self) -> ConnectionState {
        self.state
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        self.remote_addr
    }

    fn local_addr(&self) -> Option<SocketAddr> {
        self.local_addr
    }
}

/// WebSocket Server
pub struct WebSocketServer {
    listener: Option<TcpListener>,
    local_addr: Option<SocketAddr>,
}

impl WebSocketServer {
    pub fn new() -> Self {
        Self {
            listener: None,
            local_addr: None,
        }
    }
}

impl Default for WebSocketServer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TransportServer for WebSocketServer {
    fn name(&self) -> &'static str {
        "WebSocket Server"
    }

    async fn bind(&mut self, addr: SocketAddr) -> Result<()> {
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| AFPError::BindFailed(e.to_string()))?;

        self.local_addr = listener.local_addr().ok();
        self.listener = Some(listener);
        Ok(())
    }

    async fn accept(&mut self) -> Result<Box<dyn Transport>> {
        let listener = self.listener.as_ref().ok_or(AFPError::ServerNotRunning)?;
        let local_addr = self.local_addr.ok_or(AFPError::ServerNotRunning)?;

        let (tcp_stream, remote_addr) = listener
            .accept()
            .await
            .map_err(|e| AFPError::ConnectionFailed(e.to_string()))?;

        let ws_stream = accept_async(tcp_stream)
            .await
            .map_err(|e| AFPError::HandshakeFailed(e.to_string()))?;

        let transport = WebSocketTransport::from_accepted(ws_stream, remote_addr, local_addr);
        Ok(Box::new(transport))
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.listener = None;
        Ok(())
    }

    fn local_addr(&self) -> Option<SocketAddr> {
        self.local_addr
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{generate_ai_id, AIIdentity};
    use crate::keys::KeyPair;
    use crate::message::{MessageType, Payload};

    #[tokio::test]
    async fn test_websocket_connection() {
        // Start server
        let mut server = WebSocketServer::new();
        server.bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let server_addr = server.local_addr().unwrap();
        println!("WebSocket server on {}", server_addr);

        // Spawn server accept
        let server_task = tokio::spawn(async move {
            let mut conn = server.accept().await.unwrap();
            println!("Server accepted connection");

            // Receive
            let msg = conn.recv().await.unwrap();
            println!("Server received message");

            conn.close().await.unwrap();
        });

        // Give server time to start
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Connect client
        let mut client = WebSocketTransport::new();
        client.connect(server_addr).await.unwrap();
        println!("Client connected");

        // Create and send test message
        let keypair = KeyPair::generate();
        let identity = AIIdentity::new(
            generate_ai_id("test"),
            keypair.public_key(),
            "local".to_string(),
        );

        let mut msg = AFPMessage::new(
            MessageType::Request,
            &identity,
            None,
            Payload::Ping { timestamp: 12345 },
        );
        msg.sign(&keypair).unwrap();

        client.send(&msg).await.unwrap();
        println!("Client sent message");

        client.close().await.unwrap();
        server_task.await.unwrap();
    }
}
