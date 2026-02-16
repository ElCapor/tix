//! File transfer protocol — metadata, chunked transfer, delta-sync, integrity.
//!
//! # Wire Protocol
//!
//! ## File Read (download from slave)
//! ```text
//! Master ──[FileRead]───────────────────────► Slave
//!   Payload: FileTransferRequest (bincode)
//!
//! Slave  ──[FileRead + STREAMING]───────────► Master   (header)
//!   Payload: FileTransferHeader (bincode)
//!
//! Slave  ──[FileRead + STREAMING]───────────► Master   (repeated)
//!   Payload: FileChunk (bincode)
//!
//! Slave  ──[FileRead + FINAL_FRAGMENT]──────► Master
//!   Payload: FileHashVerification (bincode)
//! ```
//!
//! ## File Write (upload to slave)
//! ```text
//! Master ──[FileWrite + STREAMING]──────────► Slave    (header)
//!   Payload: FileTransferHeader (bincode)
//!
//! Master ──[FileWrite + STREAMING]──────────► Slave    (repeated)
//!   Payload: FileChunk (bincode)
//!
//! Master ──[FileWrite + FINAL_FRAGMENT]─────► Slave
//!   Payload: FileHashVerification (bincode)
//!
//! Slave  ──[FileWrite]──────────────────────► Master   (ack)
//!   Payload: FileTransferAck (bincode)
//! ```
//!
//! ## Delta Sync
//! ```text
//! Master ──[FileRead + ACK_REQUESTED]───────► Slave
//!   Payload: DeltaSyncRequest (bincode)
//!
//! Slave  ──[FileRead + STREAMING]───────────► Master
//!   Payload: DeltaChunkInfo[] (only changed chunks)
//! ```

use serde::{Deserialize, Serialize};

use crate::error::TixError;
use crate::flags::ProtocolFlags;
use crate::message::Command;
use crate::packet::Packet;

/// Default chunk size for file transfers (64 KiB).
pub const DEFAULT_CHUNK_SIZE: usize = 64 * 1024;

/// Maximum chunk size (256 KiB — matches MAX_PAYLOAD_SIZE minus overhead).
pub const MAX_CHUNK_SIZE: usize = 200 * 1024;

// ── File Transfer Request ─────────────────────────────────────────

/// Request to read/download a file from the remote.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileTransferRequest {
    /// Remote path to read from.
    pub path: String,

    /// Requested chunk size in bytes (0 = use default).
    pub chunk_size: u32,

    /// If true, request delta-sync (only changed chunks).
    pub delta_sync: bool,

    /// Optional: local Blake3 hash for delta comparison.
    pub local_hash: Option<[u8; 32]>,
}

impl FileTransferRequest {
    /// Simple file download request.
    pub fn download(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            chunk_size: DEFAULT_CHUNK_SIZE as u32,
            delta_sync: false,
            local_hash: None,
        }
    }

    /// Request with delta sync enabled.
    pub fn with_delta_sync(mut self, local_hash: [u8; 32]) -> Self {
        self.delta_sync = true;
        self.local_hash = Some(local_hash);
        self
    }

    /// Set custom chunk size.
    pub fn with_chunk_size(mut self, size: u32) -> Self {
        self.chunk_size = size;
        self
    }

    /// Serialize to bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>, TixError> {
        bincode::serialize(self).map_err(|e| TixError::Encoding(e.to_string()))
    }

    /// Deserialize from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TixError> {
        bincode::deserialize(bytes).map_err(|e| TixError::Encoding(e.to_string()))
    }

    /// Build a command `Packet`.
    pub fn into_packet(self, request_id: u64) -> Result<Packet, TixError> {
        let payload = self.to_bytes()?;
        Packet::new_command(request_id, Command::FileRead, payload)
    }
}

// ── File Transfer Header ──────────────────────────────────────────

/// Metadata sent at the start of a file transfer (before data chunks).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileTransferHeader {
    /// File path (UTF-8).
    pub path: String,

    /// Total file size in bytes.
    pub size: u64,

    /// Last modification time as Unix timestamp (seconds).
    pub modified: u64,

    /// File permissions (Unix-style, 0o644 etc.).
    pub permissions: u32,

    /// Whether this is a directory (for metadata-only transfer).
    pub is_directory: bool,

    /// Total number of chunks that will follow.
    pub total_chunks: u64,

    /// Chunk size used for this transfer.
    pub chunk_size: u32,
}

impl FileTransferHeader {
    /// Serialize to bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>, TixError> {
        bincode::serialize(self).map_err(|e| TixError::Encoding(e.to_string()))
    }

    /// Deserialize from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TixError> {
        bincode::deserialize(bytes).map_err(|e| TixError::Encoding(e.to_string()))
    }

    /// Build a streaming response `Packet` (sent before data chunks).
    pub fn into_packet(self, request_id: u64, command: Command) -> Result<Packet, TixError> {
        let payload = self.to_bytes()?;
        Packet::new_response_with_flags(request_id, command, payload, ProtocolFlags::STREAMING)
    }

    /// Compute the expected number of chunks for a file of given size.
    pub fn compute_total_chunks(file_size: u64, chunk_size: u32) -> u64 {
        if file_size == 0 {
            return 0;
        }
        let cs = chunk_size as u64;
        file_size.div_ceil(cs)
    }
}

// ── File Chunk ────────────────────────────────────────────────────

/// A single chunk of file data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileChunk {
    /// Byte offset within the file.
    pub offset: u64,

    /// Sequential chunk index (0-based).
    pub chunk_index: u64,

    /// The data for this chunk.
    pub data: Vec<u8>,
}

impl FileChunk {
    /// Create a new file chunk.
    pub fn new(offset: u64, chunk_index: u64, data: Vec<u8>) -> Self {
        Self {
            offset,
            chunk_index,
            data,
        }
    }

    /// Serialize to bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>, TixError> {
        bincode::serialize(self).map_err(|e| TixError::Encoding(e.to_string()))
    }

    /// Deserialize from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TixError> {
        bincode::deserialize(bytes).map_err(|e| TixError::Encoding(e.to_string()))
    }

    /// Build a streaming response `Packet`.
    pub fn into_packet(self, request_id: u64, command: Command) -> Result<Packet, TixError> {
        let payload = self.to_bytes()?;
        Packet::new_response_with_flags(request_id, command, payload, ProtocolFlags::STREAMING)
    }
}

// ── File Metadata ─────────────────────────────────────────────────

/// Lightweight file metadata for directory listings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileMetadata {
    /// File or directory name.
    pub name: String,

    /// Full path.
    pub path: String,

    /// Size in bytes (0 for directories).
    pub size: u64,

    /// Last modification time as Unix timestamp.
    pub modified: u64,

    /// Whether this entry is a directory.
    pub is_directory: bool,

    /// Optional Blake3 hash of contents.
    pub hash: Option<[u8; 32]>,
}

impl FileMetadata {
    /// Serialize to bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>, TixError> {
        bincode::serialize(self).map_err(|e| TixError::Encoding(e.to_string()))
    }

    /// Deserialize from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TixError> {
        bincode::deserialize(bytes).map_err(|e| TixError::Encoding(e.to_string()))
    }
}

// ── File Hash Verification ────────────────────────────────────────

/// Final verification payload sent after all file chunks.
///
/// Carried with `FINAL_FRAGMENT` flag set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileHashVerification {
    /// Blake3 hash of the complete file contents.
    pub blake3_hash: [u8; 32],

    /// Total bytes transferred.
    pub total_bytes: u64,

    /// Total number of chunks sent.
    pub total_chunks: u64,
}

impl FileHashVerification {
    /// Create a new verification payload.
    pub fn new(blake3_hash: [u8; 32], total_bytes: u64, total_chunks: u64) -> Self {
        Self {
            blake3_hash,
            total_bytes,
            total_chunks,
        }
    }

    /// Serialize to bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>, TixError> {
        bincode::serialize(self).map_err(|e| TixError::Encoding(e.to_string()))
    }

    /// Deserialize from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TixError> {
        bincode::deserialize(bytes).map_err(|e| TixError::Encoding(e.to_string()))
    }

    /// Build the final response `Packet` with FINAL_FRAGMENT flag.
    pub fn into_packet(self, request_id: u64, command: Command) -> Result<Packet, TixError> {
        let payload = self.to_bytes()?;
        Packet::new_response_with_flags(request_id, command, payload, ProtocolFlags::FINAL_FRAGMENT)
    }
}

// ── Delta Sync ────────────────────────────────────────────────────

/// Request for delta-based file synchronization.
///
/// The receiver compares chunk hashes and only transmits changed chunks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeltaSyncRequest {
    /// Remote file path.
    pub path: String,

    /// Chunk size used for splitting.
    pub chunk_size: u32,

    /// Hashes of each local chunk (index → Blake3 hash).
    pub chunk_hashes: Vec<DeltaChunkInfo>,
}

impl DeltaSyncRequest {
    /// Serialize to bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>, TixError> {
        bincode::serialize(self).map_err(|e| TixError::Encoding(e.to_string()))
    }

    /// Deserialize from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TixError> {
        bincode::deserialize(bytes).map_err(|e| TixError::Encoding(e.to_string()))
    }
}

/// Information about a single chunk for delta comparison.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeltaChunkInfo {
    /// Chunk index.
    pub index: u64,

    /// Byte offset in the file.
    pub offset: u64,

    /// Chunk length in bytes.
    pub length: u32,

    /// Blake3 hash of this chunk's data.
    pub hash: [u8; 32],
}

impl DeltaChunkInfo {
    pub fn new(index: u64, offset: u64, length: u32, hash: [u8; 32]) -> Self {
        Self {
            index,
            offset,
            length,
            hash,
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────

/// Classify a file transfer response packet by its flags.
pub fn classify_file_response(packet: &Packet) -> FileResponseKind {
    let flags = packet.flags();
    if flags.contains(ProtocolFlags::FINAL_FRAGMENT) {
        FileResponseKind::HashVerification
    } else if flags.contains(ProtocolFlags::STREAMING) {
        FileResponseKind::StreamingChunk
    } else {
        FileResponseKind::SingleResponse
    }
}

/// Classification of a file transfer response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileResponseKind {
    /// A streaming chunk (header or data chunk).
    StreamingChunk,
    /// The final hash verification packet.
    HashVerification,
    /// A single non-streaming response (ack, error, etc.).
    SingleResponse,
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_transfer_request_roundtrip() {
        let req = FileTransferRequest::download("/home/user/file.txt").with_chunk_size(32768);

        let bytes = req.to_bytes().unwrap();
        let decoded = FileTransferRequest::from_bytes(&bytes).unwrap();
        assert_eq!(req, decoded);
        assert_eq!(decoded.path, "/home/user/file.txt");
        assert_eq!(decoded.chunk_size, 32768);
        assert!(!decoded.delta_sync);
    }

    #[test]
    fn file_transfer_request_with_delta() {
        let hash = [0xABu8; 32];
        let req = FileTransferRequest::download("file.bin").with_delta_sync(hash);

        let bytes = req.to_bytes().unwrap();
        let decoded = FileTransferRequest::from_bytes(&bytes).unwrap();
        assert!(decoded.delta_sync);
        assert_eq!(decoded.local_hash.unwrap(), hash);
    }

    #[test]
    fn file_transfer_header_roundtrip() {
        let header = FileTransferHeader {
            path: "C:\\data\\report.pdf".to_string(),
            size: 1_048_576,
            modified: 1700000000,
            permissions: 0o644,
            is_directory: false,
            total_chunks: 16,
            chunk_size: DEFAULT_CHUNK_SIZE as u32,
        };

        let bytes = header.to_bytes().unwrap();
        let decoded = FileTransferHeader::from_bytes(&bytes).unwrap();
        assert_eq!(header, decoded);
    }

    #[test]
    fn file_chunk_roundtrip() {
        let data = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let chunk = FileChunk::new(0, 0, data.clone());

        let bytes = chunk.to_bytes().unwrap();
        let decoded = FileChunk::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.data, data);
        assert_eq!(decoded.offset, 0);
    }

    #[test]
    fn file_metadata_roundtrip() {
        let meta = FileMetadata {
            name: "report.pdf".to_string(),
            path: "C:\\docs\\report.pdf".to_string(),
            size: 2048,
            modified: 1700000000,
            is_directory: false,
            hash: Some([0x42; 32]),
        };

        let bytes = meta.to_bytes().unwrap();
        let decoded = FileMetadata::from_bytes(&bytes).unwrap();
        assert_eq!(meta, decoded);
    }

    #[test]
    fn file_hash_verification_roundtrip() {
        let hash = blake3::hash(b"test content");
        let verify = FileHashVerification::new(*hash.as_bytes(), 12, 1);

        let bytes = verify.to_bytes().unwrap();
        let decoded = FileHashVerification::from_bytes(&bytes).unwrap();
        assert_eq!(verify, decoded);
    }

    #[test]
    fn delta_sync_request_roundtrip() {
        let req = DeltaSyncRequest {
            path: "data.bin".to_string(),
            chunk_size: DEFAULT_CHUNK_SIZE as u32,
            chunk_hashes: vec![
                DeltaChunkInfo::new(0, 0, 65536, [0x11; 32]),
                DeltaChunkInfo::new(1, 65536, 65536, [0x22; 32]),
            ],
        };

        let bytes = req.to_bytes().unwrap();
        let decoded = DeltaSyncRequest::from_bytes(&bytes).unwrap();
        assert_eq!(req, decoded);
        assert_eq!(decoded.chunk_hashes.len(), 2);
    }

    #[test]
    fn compute_total_chunks() {
        assert_eq!(FileTransferHeader::compute_total_chunks(0, 65536), 0);
        assert_eq!(FileTransferHeader::compute_total_chunks(1, 65536), 1);
        assert_eq!(FileTransferHeader::compute_total_chunks(65536, 65536), 1);
        assert_eq!(FileTransferHeader::compute_total_chunks(65537, 65536), 2);
        assert_eq!(
            FileTransferHeader::compute_total_chunks(1_000_000, 65536),
            16
        );
    }

    #[test]
    fn file_transfer_into_packet() {
        let req = FileTransferRequest::download("test.txt");
        let packet = req.into_packet(42).unwrap();

        assert_eq!(packet.command().unwrap(), Command::FileRead);
        assert_eq!(packet.request_id(), 42);

        let decoded = FileTransferRequest::from_bytes(packet.payload()).unwrap();
        assert_eq!(decoded.path, "test.txt");
    }
}
