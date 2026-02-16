//! # tix-core
//!
//! Core protocol library for the TIX command-and-control framework.
//!
//! This crate contains:
//! - **Protocol types**: `PacketHeader`, `Packet`, `Command`, `MessageType`, `ProtocolFlags`
//! - **Protocol payloads**: Structured request/response types for shell, file, and screen
//! - **Codec**: `TixCodec` for framed TCP I/O via `tokio_util`
//! - **Network**: `Connection` for managed TCP connections with heartbeat
//! - **State**: Connection state machines for master and slave
//! - **Task**: `TaskPool` for tracking spawned async work with cancellation
//! - **Error**: `TixError` — typed, `thiserror`-based error hierarchy

pub mod codec;
pub mod error;
pub mod flags;
pub mod header;
pub mod message;
pub mod network;
pub mod packet;
pub mod protocol;
pub mod rdp;
pub mod state;
pub mod task;

// ── Re-exports for ergonomic usage ───────────────────────────────

pub use codec::TixCodec;
pub use error::{TaskError, TixError};
pub use flags::ProtocolFlags;
pub use header::{HEADER_SIZE, PacketHeader};
pub use message::{Command, MessageType};
pub use network::{Connection, ConnectionInfo, ConnectionSender};
pub use packet::{MAX_FRAME_SIZE, MAX_PAYLOAD_SIZE, Packet};
pub use state::{ConnectionPhase, MasterState, PeerCapabilities, SlaveState, TrackedRequest};
pub use task::{Task, TaskEvent, TaskEventSender, TaskOptions, TaskPool};

// ── RDP (Phase 7) re-exports ─────────────────────────────────────
pub use rdp::{
    BandwidthEstimator, DeltaDetector, DxgiCapturer, FrameDecoder, InputInjector,
    ScreenClient, ScreenService, ScreenServiceConfig, ScreenTransport,
};
