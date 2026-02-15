//! AI Foundation - Shared library for MCP and HTTP servers
//! Both binaries call the same CLI executables via subprocess.

pub mod cli_wrapper;
pub mod crypto;
pub mod federation;
pub mod federation_sync;
pub mod hlc;
pub mod http_api;
pub mod pairing;
pub mod profile;
pub mod sse;
