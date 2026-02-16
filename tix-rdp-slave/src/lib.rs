//! # tix-rdp-slave — Remote Desktop Slave Service
//!
//! Background service that captures the local screen via DXGI Desktop
//! Duplication, encodes delta frames with zstd, and streams them over
//! UDP to the connected master (tix-rdp-gui).
//!
//! Also receives mouse/keyboard events from the master and injects
//! them into the local input stream.
//!
//! ## Modes
//!
//! - **Console**: Run in the foreground for debugging (`--console`).
//! - **Service**: Run as a Windows service (default when launched by SCM).
//! - **Install / Uninstall**: Register or remove the Windows service.

pub mod config;
pub mod service;

#[cfg(target_os = "windows")]
pub mod win_service;
