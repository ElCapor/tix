//! # tix-rdp-gui — Remote Desktop GUI Client
//!
//! Runs on the **master** machine. Connects to `tix-rdp-slave`,
//! receives screen frames over UDP, renders them into a native
//! Win32 window, and forwards local mouse/keyboard input back
//! to the slave via TCP.

pub mod config;
pub mod connection;
pub mod display;
pub mod input;
pub mod window;
