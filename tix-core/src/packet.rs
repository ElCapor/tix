//! High-level `Packet` type — header + payload with Blake3 integrity.
//!
//! Provides builder methods for constructing command/response packets
//! and full checksum validation on decode.

use crate::error::TixError;
use crate::flags::ProtocolFlags;
use crate::header::{HEADER_SIZE, PacketHeader};
use crate::message::{Command, MessageType};

/// Maximum payload size (256 KiB).
pub const MAX_PAYLOAD_SIZE: usize = 256 * 1024;

/// Maximum total frame size (header + payload).
pub const MAX_FRAME_SIZE: usize = HEADER_SIZE + MAX_PAYLOAD_SIZE;

/// A fully assembled TIX packet (header + payload).
#[derive(Clone)]
pub struct Packet {
    header: PacketHeader,
    payload: Vec<u8>,
}

impl Packet {
    // ── Constructors ─────────────────────────────────────────────

    /// Create a heartbeat packet (no payload, no response expected).
    pub fn heartbeat() -> Self {
        let header = PacketHeader::new(
            MessageType::Command,
            Command::Heartbeat,
            ProtocolFlags::NONE,
            0,
            0,
        );
        Self {
            header,
            payload: Vec::new(),
        }
    }

    /// Build a command packet (no special flags).
    pub fn new_command(
        request_id: u64,
        command: Command,
        payload: Vec<u8>,
    ) -> Result<Self, TixError> {
        Self::build(
            MessageType::Command,
            request_id,
            command,
            payload,
            ProtocolFlags::NONE,
        )
    }

    /// Build a response packet (no special flags).
    pub fn new_response(
        request_id: u64,
        command: Command,
        payload: Vec<u8>,
    ) -> Result<Self, TixError> {
        Self::build(
            MessageType::Response,
            request_id,
            command,
            payload,
            ProtocolFlags::NONE,
        )
    }

    /// Build a command packet with explicit protocol flags.
    pub fn new_command_with_flags(
        request_id: u64,
        command: Command,
        payload: Vec<u8>,
        flags: ProtocolFlags,
    ) -> Result<Self, TixError> {
        Self::build(MessageType::Command, request_id, command, payload, flags)
    }

    /// Build a response packet with explicit protocol flags.
    pub fn new_response_with_flags(
        request_id: u64,
        command: Command,
        payload: Vec<u8>,
        flags: ProtocolFlags,
    ) -> Result<Self, TixError> {
        Self::build(MessageType::Response, request_id, command, payload, flags)
    }

    /// Internal builder that computes the Blake3 checksum.
    fn build(
        msg_type: MessageType,
        request_id: u64,
        command: Command,
        payload: Vec<u8>,
        flags: ProtocolFlags,
    ) -> Result<Self, TixError> {
        if payload.len() > MAX_PAYLOAD_SIZE {
            return Err(TixError::PayloadTooLarge {
                size: payload.len(),
                max: MAX_PAYLOAD_SIZE,
            });
        }

        let mut header =
            PacketHeader::new(msg_type, command, flags, request_id, payload.len() as u64);

        if !payload.is_empty() {
            let hash = blake3::hash(&payload);
            header.set_checksum(*hash.as_bytes());
        }

        Ok(Self { header, payload })
    }

    // ── Accessors ────────────────────────────────────────────────

    /// Returns a reference to the underlying packet header.
    pub fn header(&self) -> &PacketHeader {
        &self.header
    }

    /// Returns the raw payload bytes.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Returns the message type (Command or Response).
    pub fn message_type(&self) -> MessageType {
        self.header.message_type()
    }

    /// Returns the command encoded in this packet.
    pub fn command(&self) -> Result<Command, TixError> {
        self.header.command()
    }

    /// Returns the protocol flags.
    pub fn flags(&self) -> ProtocolFlags {
        self.header.flags()
    }

    /// Returns the request ID for correlating responses.
    pub fn request_id(&self) -> u64 {
        self.header.request_id()
    }

    /// Returns the declared payload length from the header.
    pub fn payload_length(&self) -> u64 {
        self.header.payload_length()
    }

    /// Returns the 32-byte Blake3 checksum from the header.
    pub fn checksum(&self) -> &[u8; 32] {
        self.header.checksum()
    }

    // ── Serialization ────────────────────────────────────────────

    /// Serialize the full packet (header + payload) to bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>, TixError> {
        if self.payload.len() > MAX_PAYLOAD_SIZE {
            return Err(TixError::PayloadTooLarge {
                size: self.payload.len(),
                max: MAX_PAYLOAD_SIZE,
            });
        }
        let mut buf = Vec::with_capacity(HEADER_SIZE + self.payload.len());
        buf.extend_from_slice(&self.header.to_bytes());
        buf.extend_from_slice(&self.payload);
        Ok(buf)
    }

    /// Deserialize a packet from raw bytes (header + payload).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TixError> {
        if bytes.len() < HEADER_SIZE {
            return Err(TixError::InvalidPacketLength {
                expected: HEADER_SIZE,
                actual: bytes.len(),
            });
        }

        let header = PacketHeader::from_bytes(&bytes[..HEADER_SIZE])?;

        let expected_total = HEADER_SIZE + header.payload_length() as usize;
        if bytes.len() != expected_total {
            return Err(TixError::InvalidPacketLength {
                expected: expected_total,
                actual: bytes.len(),
            });
        }

        if header.payload_length() as usize > MAX_PAYLOAD_SIZE {
            return Err(TixError::PayloadTooLarge {
                size: header.payload_length() as usize,
                max: MAX_PAYLOAD_SIZE,
            });
        }

        let payload = bytes[HEADER_SIZE..].to_vec();

        Ok(Self { header, payload })
    }

    // ── Validation ───────────────────────────────────────────────

    /// Verify the Blake3 checksum of the payload.
    ///
    /// Returns `Ok(true)` if the checksum matches, `Ok(false)` if it
    /// does not, and `Ok(true)` for empty payloads (no checksum needed).
    pub fn validate_checksum(&self) -> bool {
        if self.payload.is_empty() {
            return true;
        }
        let computed = blake3::hash(&self.payload);
        computed.as_bytes() == self.header.checksum()
    }
}

impl std::fmt::Debug for Packet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Packet")
            .field("header", &self.header)
            .field("payload_len", &self.payload.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_is_empty() {
        let pkt = Packet::heartbeat();
        assert!(pkt.payload().is_empty());
        assert_eq!(pkt.request_id(), 0);
        assert!(pkt.validate_checksum());
    }

    #[test]
    fn command_roundtrip() {
        let payload = b"hello world".to_vec();
        let pkt = Packet::new_command(1, Command::ShellExecute, payload.clone()).unwrap();

        let bytes = pkt.to_bytes().unwrap();
        let decoded = Packet::from_bytes(&bytes).unwrap();

        assert_eq!(decoded.message_type(), MessageType::Command);
        assert_eq!(decoded.command().unwrap(), Command::ShellExecute);
        assert_eq!(decoded.request_id(), 1);
        assert_eq!(decoded.payload(), payload.as_slice());
        assert!(decoded.validate_checksum());
    }

    #[test]
    fn response_roundtrip() {
        let payload = b"result data".to_vec();
        let pkt = Packet::new_response(7, Command::ListDir, payload.clone()).unwrap();

        let bytes = pkt.to_bytes().unwrap();
        let decoded = Packet::from_bytes(&bytes).unwrap();

        assert_eq!(decoded.message_type(), MessageType::Response);
        assert_eq!(decoded.command().unwrap(), Command::ListDir);
        assert_eq!(decoded.request_id(), 7);
        assert_eq!(decoded.payload(), payload.as_slice());
        assert!(decoded.validate_checksum());
    }

    #[test]
    fn payload_too_large() {
        let big = vec![0u8; MAX_PAYLOAD_SIZE + 1];
        let err = Packet::new_command(1, Command::Ping, big).unwrap_err();
        assert!(matches!(err, TixError::PayloadTooLarge { .. }));
    }

    #[test]
    fn tampered_payload_fails_validation() {
        let pkt = Packet::new_command(1, Command::ShellExecute, b"data".to_vec()).unwrap();
        let mut bytes = pkt.to_bytes().unwrap();
        // Tamper with the last payload byte
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        let decoded = Packet::from_bytes(&bytes).unwrap();
        assert!(!decoded.validate_checksum());
    }

    #[test]
    fn empty_payload_command() {
        let pkt = Packet::new_command(5, Command::Ping, Vec::new()).unwrap();
        assert!(pkt.validate_checksum());
        let bytes = pkt.to_bytes().unwrap();
        assert_eq!(bytes.len(), HEADER_SIZE);
    }
}
