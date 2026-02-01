//! Transport Layer
//!
//! Provides the networking layer for AFP with multiple transport options:
//! - QUIC (primary): Fast, multiplexed, encrypted by default
//! - WebSocket (fallback): For restrictive firewalls
//! - Unix socket (local): For same-machine communication
//!
//! All transports implement the same `Transport` trait for consistency.

pub mod quic;
pub mod websocket;

use async_trait::async_trait;
use std::net::SocketAddr;

use crate::error::Result;
use crate::message::AFPMessage;

/// Connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Connected,
    Authenticated,
    Disconnected,
}

/// Transport abstraction
#[async_trait]
pub trait Transport: Send + Sync {
    /// Get transport name
    fn name(&self) -> &'static str;

    /// Connect to a remote endpoint
    async fn connect(&mut self, addr: SocketAddr) -> Result<()>;

    /// Send a message
    async fn send(&mut self, message: &AFPMessage) -> Result<()>;

    /// Receive a message (blocking)
    async fn recv(&mut self) -> Result<AFPMessage>;

    /// Close the connection
    async fn close(&mut self) -> Result<()>;

    /// Get connection state
    fn state(&self) -> ConnectionState;

    /// Get remote address
    fn remote_addr(&self) -> Option<SocketAddr>;

    /// Get local address
    fn local_addr(&self) -> Option<SocketAddr>;
}

/// Server abstraction
#[async_trait]
pub trait TransportServer: Send + Sync {
    /// Get server name
    fn name(&self) -> &'static str;

    /// Start listening
    async fn bind(&mut self, addr: SocketAddr) -> Result<()>;

    /// Accept a new connection
    async fn accept(&mut self) -> Result<Box<dyn Transport>>;

    /// Stop the server
    async fn shutdown(&mut self) -> Result<()>;

    /// Get local address
    fn local_addr(&self) -> Option<SocketAddr>;
}

// Re-export main types
pub use quic::{QuicTransport, QuicServer};
pub use websocket::{WebSocketTransport, WebSocketServer};
