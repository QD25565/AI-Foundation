//! Mailbox system for AI-to-AI communication
//!
//! Each AI has a dedicated mailbox (ring buffer) where other AIs can
//! send messages. Messages are serialized using rkyv for zero-copy
//! deserialization.

use serde::{Deserialize, Serialize};
use crate::ring_buffer::{RingBufferHeader, SpscRingBuffer};
use crate::zerocopy::ZcMessage;
use crate::MAX_MESSAGE_SIZE;

/// Message types for AI communication
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum MessageType {
    /// Direct message between AIs
    DirectMessage = 1,
    /// Broadcast to all AIs
    Broadcast = 2,
    /// Presence update
    Presence = 3,
    /// File claim notification
    FileClaim = 4,
    /// Vote notification
    Vote = 5,
    /// Task assignment
    Task = 6,
    /// Acknowledgment
    Ack = 7,
    /// Ping/heartbeat
    Ping = 8,
    /// Custom/extension
    Custom = 255,
}

impl TryFrom<u8> for MessageType {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(MessageType::DirectMessage),
            2 => Ok(MessageType::Broadcast),
            3 => Ok(MessageType::Presence),
            4 => Ok(MessageType::FileClaim),
            5 => Ok(MessageType::Vote),
            6 => Ok(MessageType::Task),
            7 => Ok(MessageType::Ack),
            8 => Ok(MessageType::Ping),
            255 => Ok(MessageType::Custom),
            _ => Err(()),
        }
    }
}

/// Message header in wire format
#[repr(C, packed)]
pub struct MessageHeader {
    /// Message type
    pub msg_type: u8,
    /// Flags (reserved)
    pub flags: u8,
    /// Sender AI ID length
    pub sender_len: u8,
    /// Payload length (up to 64KB)
    pub payload_len: u16,
    /// Timestamp (unix millis, lower 32 bits)
    pub timestamp: u32,
}

impl MessageHeader {
    pub const SIZE: usize = std::mem::size_of::<Self>();
}

/// A message in the mailbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub msg_type: MessageType,
    pub sender: String,
    pub payload: Vec<u8>,
    pub timestamp: u64,
}

impl Message {
    /// Create a new message
    pub fn new(msg_type: MessageType, sender: &str, payload: Vec<u8>) -> Self {
        Self {
            msg_type,
            sender: sender.to_string(),
            payload,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    /// Create a direct message
    pub fn direct(sender: &str, content: &str) -> Self {
        Self::new(MessageType::DirectMessage, sender, content.as_bytes().to_vec())
    }

    /// Create a broadcast message
    pub fn broadcast(sender: &str, content: &str) -> Self {
        Self::new(MessageType::Broadcast, sender, content.as_bytes().to_vec())
    }

    /// Create a ping message
    pub fn ping(sender: &str) -> Self {
        Self::new(MessageType::Ping, sender, vec![])
    }

    /// Serialize to wire format
    pub fn to_bytes(&self) -> Vec<u8> {
        let sender_bytes = self.sender.as_bytes();
        let total_len = MessageHeader::SIZE + sender_bytes.len() + self.payload.len();

        let mut buf = Vec::with_capacity(total_len);

        // Write header
        buf.push(self.msg_type as u8);
        buf.push(0); // flags
        buf.push(sender_bytes.len() as u8);
        buf.extend_from_slice(&(self.payload.len() as u16).to_le_bytes());
        buf.extend_from_slice(&(self.timestamp as u32).to_le_bytes());

        // Write sender
        buf.extend_from_slice(sender_bytes);

        // Write payload
        buf.extend_from_slice(&self.payload);

        buf
    }

    /// Deserialize from wire format
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < MessageHeader::SIZE {
            return None;
        }

        let msg_type = MessageType::try_from(data[0]).ok()?;
        let _flags = data[1];
        let sender_len = data[2] as usize;
        let payload_len = u16::from_le_bytes([data[3], data[4]]) as usize;
        let timestamp = u32::from_le_bytes([data[5], data[6], data[7], data[8]]) as u64;

        let expected_len = MessageHeader::SIZE + sender_len + payload_len;
        if data.len() < expected_len {
            return None;
        }

        let sender_start = MessageHeader::SIZE;
        let sender = String::from_utf8(data[sender_start..sender_start + sender_len].to_vec()).ok()?;

        let payload_start = sender_start + sender_len;
        let payload = data[payload_start..payload_start + payload_len].to_vec();

        Some(Self {
            msg_type,
            sender,
            payload,
            timestamp,
        })
    }

    /// Get payload as string (for text messages)
    pub fn payload_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.payload).ok()
    }
}

/// Mailbox metadata stored in shared memory
#[repr(C)]
pub struct MailboxMeta {
    /// AI ID (null-terminated, max 63 chars + null)
    pub ai_id: [u8; 64],
    /// Process ID of owner
    pub owner_pid: u32,
    /// Last activity timestamp
    pub last_activity: u64,
    /// Mailbox status (0 = inactive, 1 = active)
    pub status: u32,
    /// Ring buffer capacity
    pub buffer_capacity: u32,
    /// Padding
    _padding: [u8; 8],
}

impl MailboxMeta {
    pub const SIZE: usize = std::mem::size_of::<Self>();

    /// Get AI ID as string
    pub fn ai_id_str(&self) -> &str {
        let end = self.ai_id.iter().position(|&b| b == 0).unwrap_or(64);
        std::str::from_utf8(&self.ai_id[..end]).unwrap_or("")
    }

    /// Set AI ID
    pub fn set_ai_id(&mut self, id: &str) {
        let bytes = id.as_bytes();
        let len = bytes.len().min(63);
        self.ai_id[..len].copy_from_slice(&bytes[..len]);
        self.ai_id[len] = 0;
    }

    /// Check if mailbox is active
    pub fn is_active(&self) -> bool {
        self.status == 1
    }
}

/// A mailbox instance for sending/receiving messages
pub struct Mailbox {
    meta: *mut MailboxMeta,
    header: *mut RingBufferHeader,
    data: *mut u8,
    capacity: usize,
}

impl Mailbox {
    /// Create a mailbox from raw pointers
    ///
    /// # Safety
    /// Pointers must be valid for the lifetime of the Mailbox
    pub unsafe fn from_raw(
        meta: *mut MailboxMeta,
        header: *mut RingBufferHeader,
        data: *mut u8,
        capacity: usize,
    ) -> Self {
        Self {
            meta,
            header,
            data,
            capacity,
        }
    }

    /// Get the AI ID for this mailbox
    pub fn ai_id(&self) -> &str {
        unsafe { (*self.meta).ai_id_str() }
    }

    /// Send a message to this mailbox
    pub fn send(&mut self, msg: &Message) -> Result<(), &'static str> {
        let bytes = msg.to_bytes();
        if bytes.len() > MAX_MESSAGE_SIZE {
            return Err("Message too large");
        }

        let mut rb = unsafe { SpscRingBuffer::from_raw(self.header, self.data, self.capacity) };

        if rb.try_write(&bytes) == 0 {
            return Err("Mailbox full");
        }

        // Update activity timestamp
        unsafe {
            (*self.meta).last_activity = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
        }

        Ok(())
    }

    /// Receive a message from this mailbox
    pub fn receive(&mut self) -> Option<Message> {
        let mut rb = unsafe { SpscRingBuffer::from_raw(self.header, self.data, self.capacity) };

        rb.try_read().and_then(|bytes| Message::from_bytes(&bytes))
    }

    /// Check if mailbox has pending messages
    pub fn has_messages(&self) -> bool {
        let rb = unsafe { SpscRingBuffer::from_raw(self.header, self.data, self.capacity) };
        !rb.is_empty()
    }

    // =========================================================================
    // Zero-Copy API (rkyv)
    // =========================================================================

    /// Send a zero-copy message (rkyv serialization)
    pub fn send_zc(&mut self, msg: &ZcMessage) -> Result<(), &'static str> {
        let bytes = msg.to_bytes();
        if bytes.len() > MAX_MESSAGE_SIZE {
            return Err("Message too large");
        }

        let mut rb = unsafe { SpscRingBuffer::from_raw(self.header, self.data, self.capacity) };

        if rb.try_write(&bytes) == 0 {
            return Err("Mailbox full");
        }

        // Update activity timestamp
        unsafe {
            (*self.meta).last_activity = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
        }

        Ok(())
    }

    /// Receive a message with zero-copy access
    ///
    /// Returns the raw bytes that can be accessed with `access_message()`
    /// for zero-copy reading, or fully deserialized with `ZcMessage::from_bytes()`.
    pub fn receive_zc_bytes(&mut self) -> Option<Vec<u8>> {
        let mut rb = unsafe { SpscRingBuffer::from_raw(self.header, self.data, self.capacity) };
        rb.try_read()
    }

    /// Receive and deserialize a zero-copy message
    pub fn receive_zc(&mut self) -> Option<ZcMessage> {
        self.receive_zc_bytes()
            .and_then(|bytes| ZcMessage::from_bytes(&bytes))
    }
}

// Mailbox is Send but not Sync (single consumer)
unsafe impl Send for Mailbox {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_serialization() {
        let msg = Message::direct("beta-002", "Hello from Lyra!");

        let bytes = msg.to_bytes();
        let decoded = Message::from_bytes(&bytes).expect("Should decode");

        assert_eq!(decoded.sender, "beta-002");
        assert_eq!(decoded.payload_str(), Some("Hello from Lyra!"));
        assert_eq!(decoded.msg_type, MessageType::DirectMessage);
    }

    #[test]
    fn test_message_types() {
        let ping = Message::ping("test-ai");
        assert_eq!(ping.msg_type, MessageType::Ping);
        assert!(ping.payload.is_empty());

        let broadcast = Message::broadcast("test-ai", "Announcement!");
        assert_eq!(broadcast.msg_type, MessageType::Broadcast);
    }
}
