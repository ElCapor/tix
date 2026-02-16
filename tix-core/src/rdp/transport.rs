//! UDP transport for screen data with chunked framing.
//!
//! Screen frames are split into MTU-sized UDP packets so they can
//! traverse the direct RJ-45 link without IP fragmentation. A thin
//! framing layer lets the receiver reassemble frames in order.
//!
//! ## Wire format
//!
//! **Frame header packet** (33 bytes):
//! ```text
//! sequence:       u32  (4)
//! frame_number:   u64  (8)
//! timestamp_us:   u64  (8)
//! width:          u32  (4)
//! height:         u32  (4)
//! is_full_frame:  u8   (1)
//! total_chunks:   u32  (4)
//! ```
//!
//! **Chunk packet** (12 byte header + payload):
//! ```text
//! sequence:       u32  (4)
//! chunk_index:    u32  (4)
//! chunk_size:     u32  (4)
//! data:           [u8] (variable, ≤ MTU − 12)
//! ```

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use tokio::net::UdpSocket;

use crate::error::TixError;
use crate::rdp::encoder::EncodedFrame;

// ── Constants ────────────────────────────────────────────────────

/// Maximum transmission unit minus IP (20) + UDP (8) headers.
const DEFAULT_MTU: usize = 1400;

// ── FrameHeader ──────────────────────────────────────────────────

/// Per-frame metadata sent as the first datagram of each frame.
#[derive(Debug, Clone, Copy)]
pub struct FrameHeader {
    pub sequence: u32,
    pub frame_number: u64,
    pub timestamp_us: u64,
    pub width: u32,
    pub height: u32,
    pub is_full_frame: bool,
    pub total_chunks: u32,
}

impl FrameHeader {
    /// Encoded size on the wire.
    pub const SIZE: usize = 33;

    /// Serialize to bytes (little-endian).
    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.sequence.to_le_bytes());
        buf[4..12].copy_from_slice(&self.frame_number.to_le_bytes());
        buf[12..20].copy_from_slice(&self.timestamp_us.to_le_bytes());
        buf[20..24].copy_from_slice(&self.width.to_le_bytes());
        buf[24..28].copy_from_slice(&self.height.to_le_bytes());
        buf[28] = self.is_full_frame as u8;
        buf[29..33].copy_from_slice(&self.total_chunks.to_le_bytes());
        buf
    }

    /// Deserialize from bytes.
    pub fn decode(data: &[u8]) -> Result<Self, TixError> {
        if data.len() < Self::SIZE {
            return Err(TixError::Other(format!(
                "FrameHeader too short: {} < {}",
                data.len(),
                Self::SIZE,
            )));
        }
        Ok(Self {
            sequence: u32::from_le_bytes(data[0..4].try_into().unwrap()),
            frame_number: u64::from_le_bytes(data[4..12].try_into().unwrap()),
            timestamp_us: u64::from_le_bytes(data[12..20].try_into().unwrap()),
            width: u32::from_le_bytes(data[20..24].try_into().unwrap()),
            height: u32::from_le_bytes(data[24..28].try_into().unwrap()),
            is_full_frame: data[28] != 0,
            total_chunks: u32::from_le_bytes(data[29..33].try_into().unwrap()),
        })
    }
}

// ── ChunkHeader ──────────────────────────────────────────────────

/// Per-chunk metadata prepended to each data datagram.
#[derive(Debug, Clone, Copy)]
pub struct ChunkHeader {
    pub sequence: u32,
    pub chunk_index: u32,
    pub chunk_size: u32,
}

impl ChunkHeader {
    /// Encoded size on the wire.
    pub const SIZE: usize = 12;

    /// Serialize to bytes (little-endian).
    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.sequence.to_le_bytes());
        buf[4..8].copy_from_slice(&self.chunk_index.to_le_bytes());
        buf[8..12].copy_from_slice(&self.chunk_size.to_le_bytes());
        buf
    }

    /// Deserialize from bytes.
    pub fn decode(data: &[u8]) -> Result<Self, TixError> {
        if data.len() < Self::SIZE {
            return Err(TixError::Other(format!(
                "ChunkHeader too short: {} < {}",
                data.len(),
                Self::SIZE,
            )));
        }
        Ok(Self {
            sequence: u32::from_le_bytes(data[0..4].try_into().unwrap()),
            chunk_index: u32::from_le_bytes(data[4..8].try_into().unwrap()),
            chunk_size: u32::from_le_bytes(data[8..12].try_into().unwrap()),
        })
    }
}

// ── ScreenTransport ──────────────────────────────────────────────

/// Bidirectional UDP transport for screen frames.
///
/// The sender splits each [`EncodedFrame`] into MTU-sized chunks and
/// transmits them. The receiver reassembles frames using sequence
/// numbers.
pub struct ScreenTransport {
    socket: UdpSocket,
    remote_addr: SocketAddr,
    sequence: AtomicU32,
    mtu: usize,
    /// Total bytes sent since construction (for bandwidth estimation).
    bytes_sent: std::sync::atomic::AtomicU64,
}

impl ScreenTransport {
    /// Wrap an already-bound `UdpSocket` targeting `remote_addr`.
    pub fn new(socket: UdpSocket, remote_addr: SocketAddr) -> Self {
        Self {
            socket,
            remote_addr,
            sequence: AtomicU32::new(0),
            mtu: DEFAULT_MTU,
            bytes_sent: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Override the effective MTU (must be > [`ChunkHeader::SIZE`]).
    pub fn with_mtu(mut self, mtu: usize) -> Self {
        assert!(mtu > ChunkHeader::SIZE + 1);
        self.mtu = mtu;
        self
    }

    /// Total bytes sent across all frames.
    pub fn bytes_sent(&self) -> u64 {
        self.bytes_sent.load(Ordering::Relaxed)
    }

    /// Send an encoded frame as a sequence of UDP datagrams.
    pub async fn send_frame(&self, frame: &EncodedFrame) -> Result<(), TixError> {
        let seq = self.sequence.fetch_add(1, Ordering::SeqCst);
        let chunk_payload_max = self.mtu - ChunkHeader::SIZE;
        let total_chunks = (frame.data.len() + chunk_payload_max - 1) / chunk_payload_max;

        // 1. Frame header datagram.
        let header = FrameHeader {
            sequence: seq,
            frame_number: frame.frame_number,
            timestamp_us: frame.timestamp.elapsed().as_micros() as u64,
            width: frame.width,
            height: frame.height,
            is_full_frame: frame.is_full_frame,
            total_chunks: total_chunks as u32,
        };
        let header_bytes = header.encode();
        self.socket
            .send_to(&header_bytes, self.remote_addr)
            .await
            .map_err(|e| TixError::Other(format!("UDP send header: {e}")))?;

        // 2. Data chunk datagrams.
        let mut sent_total = header_bytes.len();
        for (idx, chunk_data) in frame.data.chunks(chunk_payload_max).enumerate() {
            let ch = ChunkHeader {
                sequence: seq,
                chunk_index: idx as u32,
                chunk_size: chunk_data.len() as u32,
            };

            let mut pkt = Vec::with_capacity(ChunkHeader::SIZE + chunk_data.len());
            pkt.extend_from_slice(&ch.encode());
            pkt.extend_from_slice(chunk_data);

            self.socket
                .send_to(&pkt, self.remote_addr)
                .await
                .map_err(|e| TixError::Other(format!("UDP send chunk {idx}: {e}")))?;

            sent_total += pkt.len();
        }

        self.bytes_sent
            .fetch_add(sent_total as u64, Ordering::Relaxed);
        Ok(())
    }

    /// Receive the next complete frame.
    ///
    /// Waits for a frame header and then collects all chunks belonging
    /// to that sequence number. Out-of-sequence datagrams are silently
    /// dropped.
    pub async fn receive_frame(&self) -> Result<EncodedFrame, TixError> {
        let mut buf = vec![0u8; self.mtu + FrameHeader::SIZE];

        // Wait for a frame header.
        let header = loop {
            let (len, _) = self
                .socket
                .recv_from(&mut buf)
                .await
                .map_err(|e| TixError::Other(format!("UDP recv: {e}")))?;

            if len >= FrameHeader::SIZE {
                if let Ok(h) = FrameHeader::decode(&buf[..len]) {
                    break h;
                }
            }
        };

        // Collect data chunks.
        let total = header.total_chunks as usize;
        let mut chunks: Vec<Option<Vec<u8>>> = vec![None; total];
        let mut received = 0usize;

        while received < total {
            let (len, _) = self
                .socket
                .recv_from(&mut buf)
                .await
                .map_err(|e| TixError::Other(format!("UDP recv chunk: {e}")))?;

            if len < ChunkHeader::SIZE {
                continue;
            }

            let ch = match ChunkHeader::decode(&buf[..ChunkHeader::SIZE]) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Ignore chunks from other sequences.
            if ch.sequence != header.sequence {
                continue;
            }

            let idx = ch.chunk_index as usize;
            if idx >= total {
                continue;
            }
            if chunks[idx].is_some() {
                continue; // duplicate
            }

            let payload = buf[ChunkHeader::SIZE..len].to_vec();
            chunks[idx] = Some(payload);
            received += 1;
        }

        // Reassemble.
        let mut data = Vec::new();
        for chunk in chunks.into_iter().flatten() {
            data.extend_from_slice(&chunk);
        }

        Ok(EncodedFrame {
            frame_number: header.frame_number,
            timestamp: Instant::now(),
            width: header.width,
            height: header.height,
            data,
            is_full_frame: header.is_full_frame,
            block_count: 0,
        })
    }

    /// Returns a reference to the underlying socket.
    pub fn socket(&self) -> &UdpSocket {
        &self.socket
    }

    /// The remote address this transport targets.
    pub fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_header_roundtrip() {
        let hdr = FrameHeader {
            sequence: 42,
            frame_number: 100,
            timestamp_us: 123456,
            width: 1920,
            height: 1080,
            is_full_frame: true,
            total_chunks: 8,
        };

        let encoded = hdr.encode();
        let decoded = FrameHeader::decode(&encoded).unwrap();

        assert_eq!(decoded.sequence, 42);
        assert_eq!(decoded.frame_number, 100);
        assert_eq!(decoded.timestamp_us, 123456);
        assert_eq!(decoded.width, 1920);
        assert_eq!(decoded.height, 1080);
        assert!(decoded.is_full_frame);
        assert_eq!(decoded.total_chunks, 8);
    }

    #[test]
    fn chunk_header_roundtrip() {
        let ch = ChunkHeader {
            sequence: 7,
            chunk_index: 3,
            chunk_size: 1024,
        };

        let encoded = ch.encode();
        let decoded = ChunkHeader::decode(&encoded).unwrap();

        assert_eq!(decoded.sequence, 7);
        assert_eq!(decoded.chunk_index, 3);
        assert_eq!(decoded.chunk_size, 1024);
    }

    #[test]
    fn frame_header_too_short() {
        let short = [0u8; 10];
        assert!(FrameHeader::decode(&short).is_err());
    }

    #[test]
    fn chunk_header_too_short() {
        let short = [0u8; 4];
        assert!(ChunkHeader::decode(&short).is_err());
    }

    #[tokio::test]
    async fn udp_transport_send_receive() {
        // Bind two sockets on localhost.
        let sender_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let receiver_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        let sender_addr = sender_sock.local_addr().unwrap();
        let receiver_addr = receiver_sock.local_addr().unwrap();

        let transport_send = ScreenTransport::new(sender_sock, receiver_addr);
        let transport_recv = ScreenTransport::new(receiver_sock, sender_addr);

        let frame = EncodedFrame {
            frame_number: 99,
            timestamp: Instant::now(),
            width: 320,
            height: 240,
            data: vec![0xAB; 5000], // will need several chunks
            is_full_frame: true,
            block_count: 0,
        };

        let send_handle = tokio::spawn(async move {
            transport_send.send_frame(&frame).await.unwrap();
        });

        let recv_handle = tokio::spawn(async move {
            transport_recv.receive_frame().await.unwrap()
        });

        send_handle.await.unwrap();
        let received = recv_handle.await.unwrap();

        assert_eq!(received.frame_number, 99);
        assert_eq!(received.width, 320);
        assert_eq!(received.height, 240);
        assert!(received.is_full_frame);
        assert_eq!(received.data.len(), 5000);
        assert!(received.data.iter().all(|&b| b == 0xAB));
    }
}
