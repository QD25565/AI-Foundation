//! STUN Message Handler
//!
//! Processes STUN Binding requests and generates responses.
//! The core of NAT discovery - tells clients their public IP:port.

use std::net::SocketAddr;
use thiserror::Error;
use tracing::{debug, trace};

/// STUN magic cookie (RFC 5389)
const MAGIC_COOKIE: u32 = 0x2112A442;

/// STUN message types
const BINDING_REQUEST: u16 = 0x0001;
const BINDING_RESPONSE: u16 = 0x0101;
#[allow(dead_code)]
const BINDING_ERROR_RESPONSE: u16 = 0x0111;

/// STUN attribute types
#[allow(dead_code)]
const ATTR_MAPPED_ADDRESS: u16 = 0x0001;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
const ATTR_SOFTWARE: u16 = 0x8022;
const ATTR_FINGERPRINT: u16 = 0x8028;

/// Address family
const FAMILY_IPV4: u8 = 0x01;
const FAMILY_IPV6: u8 = 0x02;

/// Software identifier
const SOFTWARE: &str = "AI-Foundation-STUN/0.1";

#[derive(Error, Debug)]
pub enum StunError {
    #[error("Message too short: {0} bytes")]
    TooShort(usize),

    #[error("Invalid magic cookie")]
    InvalidMagicCookie,

    #[error("Unsupported message type: 0x{0:04x}")]
    UnsupportedType(u16),

    #[allow(dead_code)]
    #[error("Malformed message: {0}")]
    Malformed(String),
}

/// STUN message handler
pub struct StunHandler {
    /// Primary server address (used for RFC 5780 CHANGE-REQUEST responses)
    #[allow(dead_code)]
    primary_addr: SocketAddr,
    /// Secondary server address (for NAT behavior discovery via RFC 5780)
    #[allow(dead_code)]
    alt_addr: Option<SocketAddr>,
}

impl StunHandler {
    pub fn new(primary_addr: SocketAddr, alt_addr: Option<SocketAddr>) -> Self {
        Self {
            primary_addr,
            alt_addr,
        }
    }

    /// Handle an incoming STUN message
    ///
    /// Returns the response bytes if a response should be sent, or None for indications
    pub fn handle_message(
        &self,
        data: &[u8],
        src_addr: SocketAddr,
    ) -> Result<Option<Vec<u8>>, StunError> {
        // Minimum STUN header is 20 bytes
        if data.len() < 20 {
            return Err(StunError::TooShort(data.len()));
        }

        // Parse header
        let msg_type = u16::from_be_bytes([data[0], data[1]]);
        let msg_len = u16::from_be_bytes([data[2], data[3]]) as usize;
        let magic = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        let transaction_id = &data[8..20];

        trace!(
            "STUN message: type=0x{:04x} len={} magic=0x{:08x}",
            msg_type, msg_len, magic
        );

        // Verify magic cookie (RFC 5389)
        if magic != MAGIC_COOKIE {
            // Could be RFC 3489 (old STUN) - we don't support it
            return Err(StunError::InvalidMagicCookie);
        }

        // Handle based on message type
        match msg_type {
            BINDING_REQUEST => {
                debug!("Binding Request from {}", src_addr);
                let response = self.create_binding_response(transaction_id, src_addr);
                Ok(Some(response))
            }
            _ => {
                // We only handle Binding requests
                Err(StunError::UnsupportedType(msg_type))
            }
        }
    }

    /// Create a Binding Response with XOR-MAPPED-ADDRESS
    fn create_binding_response(&self, transaction_id: &[u8], client_addr: SocketAddr) -> Vec<u8> {
        let mut response = Vec::with_capacity(128);

        // We'll fill in the header after we know the total length
        // Reserve 20 bytes for header
        response.extend_from_slice(&[0u8; 20]);

        // Add XOR-MAPPED-ADDRESS attribute
        let xma_attr = self.create_xor_mapped_address(client_addr, transaction_id);
        response.extend_from_slice(&xma_attr);

        // Add SOFTWARE attribute
        let sw_attr = self.create_software_attribute();
        response.extend_from_slice(&sw_attr);

        // Calculate message length (excluding 20-byte header)
        let msg_len = (response.len() - 20) as u16;

        // Fill in header
        response[0] = (BINDING_RESPONSE >> 8) as u8;
        response[1] = (BINDING_RESPONSE & 0xff) as u8;
        // Message length
        response[2] = (msg_len >> 8) as u8;
        response[3] = (msg_len & 0xff) as u8;
        // Magic cookie
        response[4] = 0x21;
        response[5] = 0x12;
        response[6] = 0xA4;
        response[7] = 0x42;
        // Transaction ID (copy from request)
        response[8..20].copy_from_slice(transaction_id);

        // Optionally add FINGERPRINT (CRC-32 XOR 0x5354554e)
        // This helps distinguish STUN from other protocols
        let fingerprint = self.calculate_fingerprint(&response);
        response.extend_from_slice(&self.create_fingerprint_attribute(fingerprint));

        // Update message length to include fingerprint
        let final_len = (response.len() - 20) as u16;
        response[2] = (final_len >> 8) as u8;
        response[3] = (final_len & 0xff) as u8;

        response
    }

    /// Create XOR-MAPPED-ADDRESS attribute
    ///
    /// Format:
    /// - 2 bytes: Attribute type (0x0020)
    /// - 2 bytes: Attribute length
    /// - 1 byte: Reserved (0x00)
    /// - 1 byte: Family (0x01 = IPv4, 0x02 = IPv6)
    /// - 2 bytes: X-Port (port XOR'd with magic cookie high bits)
    /// - 4/16 bytes: X-Address (address XOR'd with magic cookie + transaction ID)
    fn create_xor_mapped_address(&self, addr: SocketAddr, transaction_id: &[u8]) -> Vec<u8> {
        let mut attr = Vec::with_capacity(12);

        // Attribute type
        attr.push((ATTR_XOR_MAPPED_ADDRESS >> 8) as u8);
        attr.push((ATTR_XOR_MAPPED_ADDRESS & 0xff) as u8);

        match addr {
            SocketAddr::V4(v4) => {
                // Attribute length: 8 bytes (1 reserved + 1 family + 2 port + 4 addr)
                attr.push(0x00);
                attr.push(0x08);

                // Reserved
                attr.push(0x00);

                // Family: IPv4
                attr.push(FAMILY_IPV4);

                // X-Port: port XOR'd with high 16 bits of magic cookie
                let port = v4.port();
                let xport = port ^ 0x2112; // High 16 bits of MAGIC_COOKIE
                attr.push((xport >> 8) as u8);
                attr.push((xport & 0xff) as u8);

                // X-Address: address XOR'd with magic cookie
                let ip_bytes = v4.ip().octets();
                let magic_bytes = MAGIC_COOKIE.to_be_bytes();
                for i in 0..4 {
                    attr.push(ip_bytes[i] ^ magic_bytes[i]);
                }
            }
            SocketAddr::V6(v6) => {
                // Attribute length: 20 bytes (1 reserved + 1 family + 2 port + 16 addr)
                attr.push(0x00);
                attr.push(0x14);

                // Reserved
                attr.push(0x00);

                // Family: IPv6
                attr.push(FAMILY_IPV6);

                // X-Port
                let port = v6.port();
                let xport = port ^ 0x2112;
                attr.push((xport >> 8) as u8);
                attr.push((xport & 0xff) as u8);

                // X-Address: XOR with magic cookie (4 bytes) + transaction ID (12 bytes)
                let ip_bytes = v6.ip().octets();
                let magic_bytes = MAGIC_COOKIE.to_be_bytes();

                // First 4 bytes XOR with magic cookie
                for i in 0..4 {
                    attr.push(ip_bytes[i] ^ magic_bytes[i]);
                }
                // Remaining 12 bytes XOR with transaction ID
                for i in 0..12 {
                    attr.push(ip_bytes[4 + i] ^ transaction_id[i]);
                }
            }
        }

        attr
    }

    /// Create SOFTWARE attribute
    fn create_software_attribute(&self) -> Vec<u8> {
        let sw_bytes = SOFTWARE.as_bytes();
        let padded_len = (sw_bytes.len() + 3) & !3; // Pad to 4-byte boundary

        let mut attr = Vec::with_capacity(4 + padded_len);

        // Attribute type
        attr.push((ATTR_SOFTWARE >> 8) as u8);
        attr.push((ATTR_SOFTWARE & 0xff) as u8);

        // Attribute length (unpadded)
        attr.push((sw_bytes.len() >> 8) as u8);
        attr.push((sw_bytes.len() & 0xff) as u8);

        // Value
        attr.extend_from_slice(sw_bytes);

        // Padding
        while attr.len() < 4 + padded_len {
            attr.push(0x00);
        }

        attr
    }

    /// Calculate CRC-32 fingerprint
    fn calculate_fingerprint(&self, data: &[u8]) -> u32 {
        // CRC-32 (ISO 3309) XOR 0x5354554e
        let crc = crc32_iso(data);
        crc ^ 0x5354554e
    }

    /// Create FINGERPRINT attribute
    fn create_fingerprint_attribute(&self, fingerprint: u32) -> Vec<u8> {
        let mut attr = Vec::with_capacity(8);

        // Attribute type
        attr.push((ATTR_FINGERPRINT >> 8) as u8);
        attr.push((ATTR_FINGERPRINT & 0xff) as u8);

        // Attribute length: 4 bytes
        attr.push(0x00);
        attr.push(0x04);

        // Value
        attr.extend_from_slice(&fingerprint.to_be_bytes());

        attr
    }
}

/// CRC-32 (ISO 3309 / ITU-T V.42)
/// Polynomial: 0x04C11DB7 (reversed: 0xEDB88320)
fn crc32_iso(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;

    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }

    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn test_xor_mapped_address_ipv4() {
        let handler = StunHandler::new(
            "0.0.0.0:3478".parse().unwrap(),
            None,
        );

        let addr: SocketAddr = "192.168.1.100:12345".parse().unwrap();
        let transaction_id = [0u8; 12];

        let attr = handler.create_xor_mapped_address(addr, &transaction_id);

        // Verify attribute type
        assert_eq!(attr[0], 0x00);
        assert_eq!(attr[1], 0x20); // XOR-MAPPED-ADDRESS

        // Verify length
        assert_eq!(attr[2], 0x00);
        assert_eq!(attr[3], 0x08);

        // Verify family
        assert_eq!(attr[5], FAMILY_IPV4);
    }

    #[test]
    fn test_binding_response() {
        let handler = StunHandler::new(
            "0.0.0.0:3478".parse().unwrap(),
            None,
        );

        let transaction_id = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let client_addr: SocketAddr = "203.0.113.50:54321".parse().unwrap();

        let response = handler.create_binding_response(&transaction_id, client_addr);

        // Verify it's a Binding Response
        assert_eq!(response[0], 0x01);
        assert_eq!(response[1], 0x01);

        // Verify magic cookie
        assert_eq!(&response[4..8], &[0x21, 0x12, 0xA4, 0x42]);

        // Verify transaction ID
        assert_eq!(&response[8..20], &transaction_id);
    }

    #[test]
    fn test_crc32() {
        // Test vector from RFC 5769
        let data = b"test";
        let crc = crc32_iso(data);
        assert_eq!(crc, 0xD87F7E0C);
    }
}
