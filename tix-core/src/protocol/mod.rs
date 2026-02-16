//! High-level protocol payload definitions for TIX services.
//!
//! Each sub-module defines the structured request/response payloads for a
//! specific protocol domain (shell, file transfer, remote desktop). Payloads
//! are serialized with `serde` + `bincode` and carried inside [`Packet`]
//! bodies.
//!
//! [`Packet`]: crate::packet::Packet

pub mod file;
pub mod screen;
pub mod shell;

// Re-export the most commonly used types at the protocol level.
pub use file::{
    DeltaChunkInfo, DeltaSyncRequest, FileChunk, FileHashVerification, FileMetadata,
    FileTransferHeader, FileTransferRequest,
};
pub use screen::{
    KeyAction, KeyEvent, MouseButton, MouseEvent, MouseEventKind, ScreenConfig, ScreenFrame,
    ScreenStartRequest, ScreenStopRequest,
};
pub use shell::{ShellExecuteRequest, ShellExitStatus, ShellOutputChunk, ShellResizeRequest};
