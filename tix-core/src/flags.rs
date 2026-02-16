//! Protocol flags using the `bitflags` crate.
//!
//! Flags are a 64-bit bitmask carried in every packet header and can
//! be combined with the `|` operator.

use bitflags::bitflags;

bitflags! {
    /// Protocol-level flags describing properties of a packet.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct ProtocolFlags: u64 {
        /// No special flags set.
        const NONE          = 0x0000_0000_0000_0000;
        /// Payload is compressed with Zstandard.
        const COMPRESSED    = 0x0000_0000_0000_0001;
        /// Payload is encrypted (reserved for future use).
        const ENCRYPTED     = 0x0000_0000_0000_0002;
        /// This is the final fragment of a multi-part message.
        const FINAL_FRAGMENT = 0x0000_0000_0000_0004;
        /// Request acknowledgement from the peer.
        const ACK_REQUESTED = 0x0000_0000_0000_0008;
        /// This packet is a streaming chunk (shell output, file chunk).
        const STREAMING     = 0x0000_0000_0000_0010;
    }
}

impl Default for ProtocolFlags {
    fn default() -> Self {
        ProtocolFlags::NONE
    }
}

impl From<u64> for ProtocolFlags {
    fn from(value: u64) -> Self {
        // Truncate unknown bits — we don't panic on future flags.
        ProtocolFlags::from_bits_truncate(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_none() {
        assert_eq!(ProtocolFlags::default(), ProtocolFlags::NONE);
    }

    #[test]
    fn combine_flags() {
        let flags = ProtocolFlags::COMPRESSED | ProtocolFlags::ENCRYPTED;
        assert!(flags.contains(ProtocolFlags::COMPRESSED));
        assert!(flags.contains(ProtocolFlags::ENCRYPTED));
        assert!(!flags.contains(ProtocolFlags::STREAMING));
    }

    #[test]
    fn from_raw_truncates_unknown() {
        let raw: u64 = 0xFFFF_FFFF_FFFF_FFFF;
        let flags = ProtocolFlags::from(raw);
        // Should only contain the known bits
        assert!(flags.contains(ProtocolFlags::COMPRESSED));
        assert!(flags.contains(ProtocolFlags::STREAMING));
    }

    #[test]
    fn roundtrip() {
        let flags = ProtocolFlags::COMPRESSED | ProtocolFlags::ACK_REQUESTED;
        let raw = flags.bits();
        assert_eq!(ProtocolFlags::from(raw), flags);
    }
}
