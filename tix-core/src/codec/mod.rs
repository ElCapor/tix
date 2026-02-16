//! TIX wire codec — Decoder / Encoder for `tokio_util::codec::Framed`.
//!
//! The codec reads/writes complete `Packet` values from a TCP stream.
//! Framing is done by first reading the fixed 64-byte header, extracting
//! the payload length, then waiting for the full payload before yielding.

use bytes::BytesMut;
use tokio_util::codec::{Decoder, Encoder};

use crate::error::TixError;
use crate::header::HEADER_SIZE;
use crate::packet::{MAX_FRAME_SIZE, MAX_PAYLOAD_SIZE, Packet};

/// Stateless codec for TIX packets.
pub struct TixCodec;

impl Decoder for TixCodec {
    type Item = Packet;
    type Error = TixError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // Guard: total buffered data must not exceed the frame limit.
        if src.len() > MAX_FRAME_SIZE {
            return Err(TixError::FrameTooLarge {
                size: src.len(),
                max: MAX_FRAME_SIZE,
            });
        }

        // Need at least a full header to proceed.
        if src.len() < HEADER_SIZE {
            return Ok(None);
        }

        // Peek at the header to learn the payload length.
        let header = crate::header::PacketHeader::from_bytes(&src[..HEADER_SIZE])?;
        let payload_len = header.payload_length() as usize;

        if payload_len > MAX_PAYLOAD_SIZE {
            return Err(TixError::PayloadTooLarge {
                size: payload_len,
                max: MAX_PAYLOAD_SIZE,
            });
        }

        // Non-zero payload must have a non-zero checksum.
        if payload_len > 0 && header.checksum() == &[0u8; 32] {
            return Err(TixError::ProtocolViolation(
                "non-empty payload with zero checksum",
            ));
        }

        let total = HEADER_SIZE + payload_len;
        if src.len() < total {
            // Reserve capacity to avoid repeated allocations.
            src.reserve(total - src.len());
            return Ok(None);
        }

        // We have a complete frame — split it off.
        let frame = src.split_to(total);
        let packet = Packet::from_bytes(&frame)?;

        // Validate checksum.
        if !packet.validate_checksum() {
            return Err(TixError::ChecksumMismatch);
        }

        Ok(Some(packet))
    }
}

impl Encoder<Packet> for TixCodec {
    type Error = TixError;

    fn encode(&mut self, item: Packet, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let bytes = item.to_bytes()?;
        dst.extend_from_slice(&bytes);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Command;

    #[test]
    fn decode_requires_full_header() {
        let mut codec = TixCodec;
        let mut buf = BytesMut::from(&[0u8; 10][..]);
        assert!(codec.decode(&mut buf).unwrap().is_none());
    }

    #[test]
    fn roundtrip_through_codec() {
        let mut codec = TixCodec;
        let pkt = Packet::new_command(1, Command::Ping, Vec::new()).unwrap();

        let mut buf = BytesMut::new();
        codec.encode(pkt.clone(), &mut buf).unwrap();

        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded.command().unwrap(), Command::Ping);
        assert_eq!(decoded.request_id(), 1);
    }

    #[test]
    fn roundtrip_with_payload() {
        let mut codec = TixCodec;
        let payload = b"test payload data".to_vec();
        let pkt = Packet::new_command(42, Command::ShellExecute, payload.clone()).unwrap();

        let mut buf = BytesMut::new();
        codec.encode(pkt, &mut buf).unwrap();

        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded.payload(), payload.as_slice());
        assert!(decoded.validate_checksum());
    }
}
