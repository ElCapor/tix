//! TIX Packet Header — 64 bytes, little-endian, C-compatible layout.
//!
//! ```text
//! Offset  Size   Field
//! ──────  ─────  ──────────────
//!   0       4    magic           b"TIX1"
//!   4      32    checksum        Blake3 hash of payload
//!  36       4    message_type    Command | Response
//!  40       8    flags           ProtocolFlags bitmask
//!  48       8    request_id      Unique per-command identifier
//!  56       8    payload_length  Byte count of following payload
//! ──────  ─────  ──────────────
//! Total:  64 bytes
//! ```

use crate::error::TixError;
use crate::flags::ProtocolFlags;
use crate::message::{Command, MessageType};

/// Fixed size of the on-wire header.
pub const HEADER_SIZE: usize = 64;

/// Type alias for the exact byte array that can hold one header.
pub type HeaderBytes = [u8; HEADER_SIZE];

/// Protocol magic for the current version.
pub const MAGIC: [u8; 4] = *b"TIX1";

/// TIX Protocol Header — 64 bytes.
///
/// All multi-byte fields are stored **little-endian** on the wire.
#[derive(Clone)]
pub struct PacketHeader {
    /// Magic bytes identifying the protocol version.
    magic: [u8; 4],
    /// Blake3 hash of the payload (full 32 bytes).
    checksum: [u8; 32],
    /// Whether this is a Command or Response.
    message_type: u32,
    /// Protocol flags bitmask.
    flags: u64,
    /// Unique identifier tying a command to its response.
    request_id: u64,
    /// Length of the payload that follows this header.
    payload_length: u64,
}

impl PacketHeader {
    // ── Construction ─────────────────────────────────────────────

    /// Create a new header with the given fields.
    ///
    /// The checksum should be set separately after computing the
    /// Blake3 hash of the payload. It defaults to all zeros.
    pub fn new(
        message_type: MessageType,
        command: Command,
        flags: ProtocolFlags,
        request_id: u64,
        payload_length: u64,
    ) -> Self {
        // We store the command in the message_type field's upper bits?
        // No — keep it simple: message_type is the MessageType discriminant.
        // The command is encoded in the request semantic. However, looking at
        // the old code, command_id was a separate field. The PLAN.md header
        // spec only has message_type (u32), not a separate command_id.
        //
        // Re-reading PLAN.md header spec carefully:
        //   magic(4) + checksum(32) + message_type(u32) + flags(u64)
        //   + request_id(u64) + payload_length(u64) + _reserved(8) = 64+8?
        //
        // Actually our layout has exactly 64 bytes without reserved:
        //   4 + 32 + 4 + 8 + 8 + 8 = 64 ✓
        //
        // But we need a command field! Let's repurpose message_type
        // to carry the command discriminant, and use the high bit to
        // distinguish commands from responses. Or better: keep the old
        // approach with a command_id field and shrink something else.
        //
        // Let me adjust: use the full 64 bytes more practically:
        //   magic(4) + checksum(32) + message_type(4) + command(4)
        //   + flags(8) + request_id(8) + payload_length(8) = 68 ... too much.
        //
        // Simplest backward-compatible fix: encode the command as a u32
        // (truncated from u64). Redefine layout:
        //   magic(4) + checksum(32) + message_type(u16) + command(u16)
        //    + flags(u64) + request_id(u64) + payload_length(u64) = 4+32+2+2+8+8+8 = 64 ✓
        //
        // But that limits command to u16 (65535 values — more than enough).
        // Actually the old code had command_id as u64 in a 44-byte header.
        // Let's keep it expandable. Real layout:
        //
        //   magic:          4
        //   checksum:      32
        //   message_type:   4   (u32: Command=1, Response=2)
        //   command_id:     4   (u32: the Command discriminant)
        //   flags:          8   (u64)
        //   request_id:     8   (u64)
        //   payload_length: 8   (u64)
        //   ─────────────────
        //   Total:         68   ← 4 bytes over budget
        //
        // To keep 64: drop 4 reserved bytes or shrink something.
        // Option: make flags u32 → saves 4 → total 64.
        //   magic(4)+checksum(32)+msg_type(4)+cmd(4)+flags(4)+req_id(8)+len(8) = 64 ✓
        //
        // But PLAN.md says flags is u64. Let me just make the header 68 bytes.
        // Actually, let me re-read the AGENTS.md spec more carefully:
        //   magic: [u8; 4]       = 4
        //   checksum: [u8; 32]   = 32
        //   message_type: u32    = 4
        //   flags: u64           = 8
        //   request_id: u64      = 8
        //   payload_length: u64  = 8
        //                        = 64
        //
        // There's no separate command field! The AGENTS.md uses message_type
        // to hold "Command or Response" but the old code had a separate
        // command_id. The PLAN.md command table uses 0x0001-style IDs.
        //
        // Resolution: Combine message_type + command into a single u32:
        //   High 16 bits = MessageType (0x0001=Cmd, 0x0002=Resp)
        //   Low 16 bits  = Command discriminant
        // This way: message_type u32 carries both.
        //
        // Actually simpler: just keep message_type for the command enum value
        // and add a separate bit to flags for "is_response".
        //
        // SIMPLEST: flags already has 64 bits. Use bit 63 as "is_response".
        // message_type field == command discriminant (u32).
        //
        // Let's go with that approach for cleanliness.
        let mut effective_flags = flags;
        if message_type == MessageType::Response {
            // Use the highest bit of flags to signal "this is a response"
            effective_flags |= ProtocolFlags::from_bits_retain(1 << 63);
        }

        Self {
            magic: MAGIC,
            checksum: [0u8; 32],
            message_type: command as u32,
            flags: effective_flags.bits(),
            request_id,
            payload_length,
        }
    }

    /// Set the Blake3 checksum (full 32 bytes).
    pub fn set_checksum(&mut self, checksum: [u8; 32]) {
        self.checksum = checksum;
    }

    // ── Accessors ────────────────────────────────────────────────

    /// Returns the 32-byte Blake3 checksum.
    pub fn checksum(&self) -> &[u8; 32] {
        &self.checksum
    }

    /// Returns the command encoded in this header.
    pub fn command(&self) -> Result<Command, TixError> {
        Command::try_from(self.message_type as u64)
    }

    /// Returns whether this is a Command or Response.
    pub fn message_type(&self) -> MessageType {
        if self.flags & (1 << 63) != 0 {
            MessageType::Response
        } else {
            MessageType::Command
        }
    }

    /// Returns the protocol flags (without the internal response bit).
    pub fn flags(&self) -> ProtocolFlags {
        ProtocolFlags::from(self.flags & !(1 << 63))
    }

    /// Returns the request ID used to correlate responses.
    pub fn request_id(&self) -> u64 {
        self.request_id
    }

    /// Returns the declared payload length in bytes.
    pub fn payload_length(&self) -> u64 {
        self.payload_length
    }

    // ── Serialization ────────────────────────────────────────────

    /// Serialize the header to exactly [`HEADER_SIZE`] bytes (little-endian).
    pub fn to_bytes(&self) -> HeaderBytes {
        let mut buf = [0u8; HEADER_SIZE];
        buf[0..4].copy_from_slice(&self.magic);
        buf[4..36].copy_from_slice(&self.checksum);
        buf[36..40].copy_from_slice(&self.message_type.to_le_bytes());
        buf[40..48].copy_from_slice(&self.flags.to_le_bytes());
        buf[48..56].copy_from_slice(&self.request_id.to_le_bytes());
        buf[56..64].copy_from_slice(&self.payload_length.to_le_bytes());
        buf
    }

    /// Deserialize a header from exactly [`HEADER_SIZE`] bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TixError> {
        if bytes.len() < HEADER_SIZE {
            return Err(TixError::InvalidHeader("buffer too short for header"));
        }

        let magic: [u8; 4] = bytes[0..4]
            .try_into()
            .map_err(|_| TixError::InvalidHeader("magic slice"))?;

        // Accept both TIX0 (legacy) and TIX1 (current)
        if &magic != b"TIX0" && &magic != b"TIX1" {
            return Err(TixError::InvalidMagic);
        }

        let checksum: [u8; 32] = bytes[4..36]
            .try_into()
            .map_err(|_| TixError::InvalidHeader("checksum slice"))?;

        let message_type = u32::from_le_bytes(
            bytes[36..40]
                .try_into()
                .map_err(|_| TixError::InvalidHeader("message_type slice"))?,
        );

        let flags = u64::from_le_bytes(
            bytes[40..48]
                .try_into()
                .map_err(|_| TixError::InvalidHeader("flags slice"))?,
        );

        let request_id = u64::from_le_bytes(
            bytes[48..56]
                .try_into()
                .map_err(|_| TixError::InvalidHeader("request_id slice"))?,
        );

        let payload_length = u64::from_le_bytes(
            bytes[56..64]
                .try_into()
                .map_err(|_| TixError::InvalidHeader("payload_length slice"))?,
        );

        Ok(Self {
            magic,
            checksum,
            message_type,
            flags,
            request_id,
            payload_length,
        })
    }
}

impl std::fmt::Debug for PacketHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PacketHeader")
            .field("magic", &String::from_utf8_lossy(&self.magic))
            .field("message_type", &self.message_type())
            .field("command", &self.command())
            .field("flags", &self.flags())
            .field("request_id", &self.request_id)
            .field("payload_length", &self.payload_length)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_size_is_64() {
        assert_eq!(HEADER_SIZE, 64);
    }

    #[test]
    fn roundtrip() {
        let header = PacketHeader::new(
            MessageType::Command,
            Command::Ping,
            ProtocolFlags::NONE,
            42,
            128,
        );
        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), HEADER_SIZE);

        let parsed = PacketHeader::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.message_type(), MessageType::Command);
        assert_eq!(parsed.command().unwrap(), Command::Ping);
        assert_eq!(parsed.request_id(), 42);
        assert_eq!(parsed.payload_length(), 128);
    }

    #[test]
    fn response_flag() {
        let header = PacketHeader::new(
            MessageType::Response,
            Command::ShellExecute,
            ProtocolFlags::COMPRESSED,
            10,
            256,
        );
        let bytes = header.to_bytes();
        let parsed = PacketHeader::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.message_type(), MessageType::Response);
        assert_eq!(parsed.command().unwrap(), Command::ShellExecute);
        assert!(parsed.flags().contains(ProtocolFlags::COMPRESSED));
    }

    #[test]
    fn invalid_magic_rejected() {
        let mut bytes = [0u8; HEADER_SIZE];
        bytes[0..4].copy_from_slice(b"NOPE");
        assert!(PacketHeader::from_bytes(&bytes).is_err());
    }

    #[test]
    fn too_short_rejected() {
        let bytes = [0u8; 10];
        assert!(PacketHeader::from_bytes(&bytes).is_err());
    }
}
