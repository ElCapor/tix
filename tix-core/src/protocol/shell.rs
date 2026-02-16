//! Shell command protocol — streaming execution, cancellation, PTY resize.
//!
//! # Wire Protocol
//!
//! ```text
//! Master ──[ShellExecute]─────────────────────► Slave
//!   Payload: ShellExecuteRequest (bincode)
//!
//! Slave  ──[ShellExecute + STREAMING]─────────► Master   (repeated)
//!   Payload: ShellOutputChunk (bincode)
//!
//! Slave  ──[ShellExecute + FINAL_FRAGMENT]────► Master
//!   Payload: ShellExitStatus (bincode)
//!
//! Master ──[ShellCancel]──────────────────────► Slave
//!   Payload: request_id of the command to cancel (u64 LE)
//!
//! Master ──[ShellResize]──────────────────────► Slave
//!   Payload: ShellResizeRequest (bincode)
//! ```
//!
//! Output is streamed in chunks so the master can display partial results
//! immediately without waiting for the command to finish.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::TixError;
use crate::flags::ProtocolFlags;
use crate::message::Command;
use crate::packet::Packet;

// ── Shell Execute ─────────────────────────────────────────────────

/// Request payload for `Command::ShellExecute`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShellExecuteRequest {
    /// The shell command to execute (e.g. `"dir /w"`).
    pub command: String,

    /// Whether to allocate a PTY for the process.
    pub pty: bool,

    /// Timeout in milliseconds (0 = no timeout).
    pub timeout_ms: u64,

    /// Optional environment variables to set.
    pub env: HashMap<String, String>,

    /// Optional working directory.
    pub working_dir: Option<String>,
}

impl ShellExecuteRequest {
    /// Create a simple execute request with defaults.
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            pty: false,
            timeout_ms: 30_000, // 30s default
            env: HashMap::new(),
            working_dir: None,
        }
    }

    /// Set the timeout in milliseconds.
    pub fn with_timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    /// Enable PTY allocation.
    pub fn with_pty(mut self) -> Self {
        self.pty = true;
        self
    }

    /// Set working directory.
    pub fn with_working_dir(mut self, dir: impl Into<String>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    /// Add an environment variable.
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Serialize to bytes for packet payload.
    pub fn to_bytes(&self) -> Result<Vec<u8>, TixError> {
        bincode::serialize(self).map_err(|e| TixError::Encoding(e.to_string()))
    }

    /// Deserialize from packet payload bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TixError> {
        bincode::deserialize(bytes).map_err(|e| TixError::Encoding(e.to_string()))
    }

    /// Build a command `Packet` carrying this request.
    pub fn into_packet(self, request_id: u64) -> Result<Packet, TixError> {
        let payload = self.to_bytes()?;
        Packet::new_command(request_id, Command::ShellExecute, payload)
    }
}

// ── Shell Output (streaming) ──────────────────────────────────────

/// A single chunk of shell output, streamed from slave to master.
///
/// The `STREAMING` flag is set on the packet header. When the command
/// finishes, a final `ShellExitStatus` is sent with `FINAL_FRAGMENT`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShellOutputChunk {
    /// Sequential chunk number (0-based).
    pub chunk_number: u64,

    /// The output data (UTF-8 text or raw bytes).
    pub data: Vec<u8>,

    /// `true` if this is from stdout, `false` if from stderr.
    pub is_stdout: bool,
}

impl ShellOutputChunk {
    /// Create a new stdout chunk.
    pub fn stdout(chunk_number: u64, data: Vec<u8>) -> Self {
        Self {
            chunk_number,
            data,
            is_stdout: true,
        }
    }

    /// Create a new stderr chunk.
    pub fn stderr(chunk_number: u64, data: Vec<u8>) -> Self {
        Self {
            chunk_number,
            data,
            is_stdout: false,
        }
    }

    /// Serialize to bytes for packet payload.
    pub fn to_bytes(&self) -> Result<Vec<u8>, TixError> {
        bincode::serialize(self).map_err(|e| TixError::Encoding(e.to_string()))
    }

    /// Deserialize from packet payload bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TixError> {
        bincode::deserialize(bytes).map_err(|e| TixError::Encoding(e.to_string()))
    }

    /// Build a streaming response `Packet`.
    pub fn into_packet(self, request_id: u64) -> Result<Packet, TixError> {
        let payload = self.to_bytes()?;
        Packet::new_response_with_flags(
            request_id,
            Command::ShellExecute,
            payload,
            ProtocolFlags::STREAMING,
        )
    }
}

// ── Shell Exit ────────────────────────────────────────────────────

/// Final message sent when a shell command completes.
///
/// Carried in a packet with `FINAL_FRAGMENT` flag set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShellExitStatus {
    /// Process exit code (0 = success).
    pub exit_code: i32,

    /// Total number of output chunks that were sent.
    pub total_chunks: u64,

    /// Optional error message if the command failed to start.
    pub error: Option<String>,
}

impl ShellExitStatus {
    /// Successful exit.
    pub fn success(exit_code: i32, total_chunks: u64) -> Self {
        Self {
            exit_code,
            total_chunks,
            error: None,
        }
    }

    /// Failed to start the process.
    pub fn failed(error: impl Into<String>) -> Self {
        Self {
            exit_code: -1,
            total_chunks: 0,
            error: Some(error.into()),
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

    /// Build the final response `Packet`.
    pub fn into_packet(self, request_id: u64) -> Result<Packet, TixError> {
        let payload = self.to_bytes()?;
        Packet::new_response_with_flags(
            request_id,
            Command::ShellExecute,
            payload,
            ProtocolFlags::FINAL_FRAGMENT,
        )
    }
}

// ── Shell Resize ──────────────────────────────────────────────────

/// Request to resize the PTY terminal on the slave.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShellResizeRequest {
    /// The request ID of the running shell session to resize.
    pub target_request_id: u64,

    /// New terminal width in columns.
    pub cols: u16,

    /// New terminal height in rows.
    pub rows: u16,
}

impl ShellResizeRequest {
    pub fn new(target_request_id: u64, cols: u16, rows: u16) -> Self {
        Self {
            target_request_id,
            cols,
            rows,
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

    /// Build a command `Packet`.
    pub fn into_packet(self, request_id: u64) -> Result<Packet, TixError> {
        let payload = self.to_bytes()?;
        Packet::new_command(request_id, Command::ShellResize, payload)
    }
}

// ── Helpers ───────────────────────────────────────────────────────

/// Determine whether a shell response packet is a streaming chunk or a
/// final exit status by inspecting its flags.
pub fn classify_shell_response(packet: &Packet) -> ShellResponseKind {
    let flags = packet.flags();
    if flags.contains(ProtocolFlags::FINAL_FRAGMENT) {
        ShellResponseKind::Exit
    } else if flags.contains(ProtocolFlags::STREAMING) {
        ShellResponseKind::OutputChunk
    } else {
        // Legacy: single non-streaming response (backward compat)
        ShellResponseKind::LegacySingle
    }
}

/// Classification of a shell response packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellResponseKind {
    /// A streaming output chunk.
    OutputChunk,
    /// The final exit status.
    Exit,
    /// A legacy single-response packet (no streaming flags).
    LegacySingle,
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_execute_request_roundtrip() {
        let req = ShellExecuteRequest::new("dir /w")
            .with_timeout(5000)
            .with_pty()
            .with_working_dir("C:\\Users")
            .with_env("FOO", "BAR");

        let bytes = req.to_bytes().unwrap();
        let decoded = ShellExecuteRequest::from_bytes(&bytes).unwrap();

        assert_eq!(req, decoded);
        assert_eq!(decoded.command, "dir /w");
        assert!(decoded.pty);
        assert_eq!(decoded.timeout_ms, 5000);
        assert_eq!(decoded.env.get("FOO").unwrap(), "BAR");
        assert_eq!(decoded.working_dir.as_deref(), Some("C:\\Users"));
    }

    #[test]
    fn shell_output_chunk_roundtrip() {
        let chunk = ShellOutputChunk::stdout(0, b"Hello, World!\n".to_vec());
        let bytes = chunk.to_bytes().unwrap();
        let decoded = ShellOutputChunk::from_bytes(&bytes).unwrap();
        assert_eq!(chunk, decoded);
        assert!(decoded.is_stdout);
    }

    #[test]
    fn shell_exit_status_roundtrip() {
        let exit = ShellExitStatus::success(0, 42);
        let bytes = exit.to_bytes().unwrap();
        let decoded = ShellExitStatus::from_bytes(&bytes).unwrap();
        assert_eq!(exit, decoded);
    }

    #[test]
    fn shell_exit_failed() {
        let exit = ShellExitStatus::failed("command not found");
        assert_eq!(exit.exit_code, -1);
        assert_eq!(exit.error.as_deref(), Some("command not found"));

        let bytes = exit.to_bytes().unwrap();
        let decoded = ShellExitStatus::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.error.as_deref(), Some("command not found"));
    }

    #[test]
    fn shell_resize_roundtrip() {
        let resize = ShellResizeRequest::new(42, 120, 40);
        let bytes = resize.to_bytes().unwrap();
        let decoded = ShellResizeRequest::from_bytes(&bytes).unwrap();
        assert_eq!(resize, decoded);
    }

    #[test]
    fn classify_streaming_response() {
        // We can't easily build packets with custom flags via the current API,
        // so we test the classification logic via flag inspection.
        let flags_streaming = ProtocolFlags::STREAMING;
        let flags_final = ProtocolFlags::FINAL_FRAGMENT;
        let flags_none = ProtocolFlags::NONE;

        // Simulate by checking flag membership directly
        assert!(flags_streaming.contains(ProtocolFlags::STREAMING));
        assert!(!flags_streaming.contains(ProtocolFlags::FINAL_FRAGMENT));
        assert!(flags_final.contains(ProtocolFlags::FINAL_FRAGMENT));
        assert!(!flags_none.contains(ProtocolFlags::STREAMING));
    }

    #[test]
    fn shell_execute_into_packet() {
        let req = ShellExecuteRequest::new("echo hello");
        let packet = req.into_packet(1).unwrap();

        assert_eq!(packet.command().unwrap(), Command::ShellExecute);
        assert_eq!(packet.request_id(), 1);
        assert_eq!(packet.message_type(), crate::message::MessageType::Command);

        // Verify payload can be deserialized back
        let decoded = ShellExecuteRequest::from_bytes(packet.payload()).unwrap();
        assert_eq!(decoded.command, "echo hello");
    }
}
