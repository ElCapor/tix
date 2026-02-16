//! Domain-specific error types for the TIX protocol.
//!
//! All fallible operations return `Result<T, TixError>`.
//! No panics on invalid input — every error is typed and recoverable.

use std::time::Duration;
use thiserror::Error;

/// The canonical error type for the TIX protocol.
#[derive(Debug, Error)]
pub enum TixError {
    // ── Protocol Errors ──────────────────────────────────────────
    /// Received bytes that do not start with a valid TIX magic sequence.
    #[error("invalid magic bytes: expected TIX0 or TIX1")]
    InvalidMagic,

    /// A field in the packet header could not be parsed.
    #[error("invalid header: {0}")]
    InvalidHeader(&'static str),

    /// The packet payload failed checksum verification.
    #[error("checksum mismatch")]
    ChecksumMismatch,

    /// A numeric value did not map to any known enum variant.
    #[error("unknown {type_name} discriminant: {value:#x}")]
    UnknownVariant { type_name: &'static str, value: u64 },

    /// The protocol version offered by the peer is not supported.
    #[error("unsupported protocol version: {0}")]
    UnsupportedVersion(u32),

    /// A packet violated protocol rules.
    #[error("protocol violation: {0}")]
    ProtocolViolation(&'static str),

    // ── Packet Errors ────────────────────────────────────────────
    /// The payload exceeds the configured maximum size.
    #[error("payload too large: {size} bytes (max {max})")]
    PayloadTooLarge { size: usize, max: usize },

    /// The received frame is shorter or longer than expected.
    #[error("invalid packet length: expected {expected}, got {actual}")]
    InvalidPacketLength { expected: usize, actual: usize },

    /// Frame size exceeded the codec limit.
    #[error("frame too large: {size} bytes (max {max})")]
    FrameTooLarge { size: usize, max: usize },

    // ── Connection Errors ────────────────────────────────────────
    /// The TCP/IO layer reported an error.
    #[error("connection error: {0}")]
    Connection(#[from] std::io::Error),

    /// An mpsc channel was closed unexpectedly.
    #[error("channel closed")]
    ChannelClosed,

    /// An operation exceeded its deadline.
    #[error("timeout after {0:?}")]
    Timeout(Duration),

    // ── Serialization Errors ─────────────────────────────────────
    /// Encoding or decoding of a payload failed.
    #[error("encoding error: {0}")]
    Encoding(String),

    /// UTF-8 conversion failed.
    #[error("invalid utf-8: {0}")]
    InvalidUtf8(#[from] std::string::FromUtf8Error),

    // ── Application Errors ───────────────────────────────────────
    /// A command string could not be parsed.
    #[error("invalid command: {0}")]
    InvalidCommand(String),

    /// File integrity check failed after transfer.
    #[error("file integrity check failed")]
    FileIntegrityFailed,

    // ── Task Errors ─────────────────────────────────────────────
    /// A spawned task failed.
    #[error("task error: {0}")]
    Task(#[from] TaskError),

    /// Catch-all for errors that do not fit another variant.
    #[error("{0}")]
    Other(String),
}

// ── TaskError ─────────────────────────────────────────────────────

/// Typed error for spawned async tasks.
///
/// Replaces the previous `String`-based `TaskEvent::Error` with a
/// structured enum that enables intelligent recovery.
#[derive(Debug, Error)]
pub enum TaskError {
    /// The task exceeded its deadline and was cancelled.
    #[error("task timed out after {0:?}")]
    Timeout(Duration),

    /// The task was explicitly cancelled via `CancellationToken`.
    #[error("task was cancelled")]
    Cancelled,

    /// The task's async work returned an I/O error.
    #[error("task I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Generic task failure with a human-readable message.
    #[error("task failed: {0}")]
    Failed(String),
}

// ── Convenient From implementations ──────────────────────────────

impl From<String> for TixError {
    fn from(s: String) -> Self {
        TixError::Other(s)
    }
}

impl From<&str> for TixError {
    fn from(s: &str) -> Self {
        TixError::Other(s.to_string())
    }
}

impl<T> From<tokio::sync::mpsc::error::SendError<T>> for TixError {
    fn from(_: tokio::sync::mpsc::error::SendError<T>) -> Self {
        TixError::ChannelClosed
    }
}

impl From<Box<bincode::ErrorKind>> for TixError {
    fn from(e: Box<bincode::ErrorKind>) -> Self {
        TixError::Encoding(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages() {
        let e = TixError::InvalidMagic;
        assert!(e.to_string().contains("magic"));

        let e = TixError::PayloadTooLarge {
            size: 1000,
            max: 500,
        };
        assert!(e.to_string().contains("1000"));
        assert!(e.to_string().contains("500"));
    }

    #[test]
    fn from_string() {
        let e: TixError = "something broke".into();
        assert!(matches!(e, TixError::Other(_)));
    }

    #[test]
    fn from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe broke");
        let e: TixError = io_err.into();
        assert!(matches!(e, TixError::Connection(_)));
    }
}
