//! # tix-rdp — Ultra-Fast Remote Desktop Protocol
//!
//! Phase 7 implementation of TixRP: a low-latency remote desktop system
//! optimised for direct RJ-45 Ethernet connections (~100 MB/s).
//!
//! ## Architecture
//!
//! ```text
//! SLAVE (Target)                              MASTER (Controller)
//! ┌─────────────────────────┐                ┌──────────────────────┐
//! │ DxgiCapturer            │                │ FrameDecoder         │
//! │   ↓                     │                │   ↓                  │
//! │ DeltaDetector           │   UDP/TCP      │ FrameAssembler       │
//! │   ↓                     │ ──────────►    │   ↓                  │
//! │ AdaptiveEncoder         │                │ Display / render     │
//! │   ↓                     │                │                      │
//! │ ScreenTransport::send   │                │ ScreenTransport::recv│
//! └─────────────────────────┘                └──────────────────────┘
//!
//! Input: Master ──[MouseEvent/KeyEvent]──► Slave InputInjector
//! ```
//!
//! ## Sub-modules
//!
//! | Module       | Purpose                                          |
//! |------------- |--------------------------------------------------|
//! | `types`      | Shared frame / pixel types used across the pipeline |
//! | `capture`    | DXGI Desktop Duplication screen capture (Windows) |
//! | `delta`      | Block-level change detection between frames       |
//! | `encoder`    | Adaptive zstd-based frame encoder                 |
//! | `decoder`    | Frame decoder / decompressor                      |
//! | `transport`  | UDP transport with chunked framing                |
//! | `input`      | Win32 `SendInput` mouse / keyboard injection      |
//! | `bandwidth`  | Bandwidth estimator for adaptive quality           |
//! | `service`    | Slave-side capture service orchestrator            |
//! | `client`     | Master-side frame consumer                        |

pub mod bandwidth;
pub mod capture;
pub mod client;
pub mod decoder;
pub mod delta;
pub mod encoder;
pub mod input;
pub mod service;
pub mod transport;
pub mod types;

// ── Re-exports ───────────────────────────────────────────────────

pub use bandwidth::BandwidthEstimator;
pub use capture::DxgiCapturer;
pub use client::ScreenClient;
pub use decoder::FrameDecoder;
pub use delta::{Block, DeltaDetector, DeltaFrame};
pub use encoder::{AdaptiveEncoder, EncodedFrame};
pub use input::InputInjector;
pub use service::{ScreenService, ScreenServiceConfig};
pub use transport::{ChunkHeader, FrameHeader, ScreenTransport};
pub use types::{PixelFormat, RawScreenFrame};
