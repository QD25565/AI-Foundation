//! AI Foundation - Shared library for MCP and HTTP servers
//!
//! Both binaries call the same CLI executables via subprocess.
//!
//! # Federation Architecture (Two Layers)
//!
//! **P2P Core** (`federation_core` / `crates/federation/`):
//! The canonical federation implementation — QUIC transport (iroh), mDNS LAN
//! discovery, Ed25519 handshake protocol, cursor-tracked replication with
//! content-addressed dedup, permission manifests, consent records. 161 tests.
//! Use this for all new federation work.
//!
//! **HTTP Layer** (`federation`, `federation_sync`, `federation_gateway`):
//! REST API adapter for mobile clients and legacy HTTP-based federation.
//! Uses its own types (`[u8; 32]` arrays, JSON serde, curl subprocess).
//! Will be incrementally migrated to delegate to `federation_core`.

pub mod cli_wrapper;
pub mod crypto;
#[cfg(feature = "in-process")]
pub mod direct;
pub mod federation;
pub mod federation_gateway;
pub mod federation_sync;
pub mod hlc;
pub mod http_api;
pub mod pairing;
pub mod profile;
pub mod sse;

/// Re-export the P2P federation crate for use within the workspace.
/// Access via `federation_core::TeambookIdentity`, `federation_core::QuicTransport`, etc.
pub use federation_core;
