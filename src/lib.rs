//! AI Foundation - Shared library for MCP and HTTP servers
//! Both binaries call the same CLI executables via subprocess.

pub mod cli_wrapper;
pub mod http_api;
pub mod pairing;
pub mod sse;
