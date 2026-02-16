# TIX - Complete Rewrite Plan

A high-performance command-and-control framework for dedicated two-machine peer-to-peer connections over direct RJ-45 Ethernet.

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [Current State Analysis](#current-state-analysis)
3. [Architecture Overview](#architecture-overview)
4. [Rewrite Goals](#rewrite-goals)
5. [Core Components](#core-components)
6. [Protocol Design](#protocol-design)
7. [Implementation Plan](#implementation-plan)
8. [Testing Strategy](#testing-strategy)
9. [Performance Considerations](#performance-considerations)
10. [Migration Path](#migration-path)
11. [Phase 7: tix-rdp Ultra-Fast Remote Desktop](#phase-7-tix-rdp-ultra-fast-remote-desktop)
12. [Phase 8: GUI Client and Slave RDP Service](#phase-8-gui-client-and-slave-rdp-service)
13. [Phase 9: Slave Update Protocol and Installer](#phase-9-slave-update-protocol-and-installer)

---

## Executive Summary

This document outlines a complete rewrite of the TIX framework to achieve:
- **Production-grade code quality** - No hacks, proper abstractions, idiomatic Rust
- **Complete feature set** - File transfer, shell commands, remote desktop, auto-update
- **Robust protocol** - Version negotiation, proper error handling, security
- **Maintainable architecture** - Clear module boundaries, testable components
- **Performance optimization** - Zero-copy where possible, efficient memory management

---

## Current State Analysis

### Critical Issues Identified

| Category | Issue | Severity |
|----------|-------|----------|
| **Protocol** | Checksum uses only 4 bytes of Blake3 | Critical |
| **Protocol** | No version negotiation (hardcoded TIX0) | Critical |
| **Error Handling** | `panic!` in From implementations | Critical |
| **Error Handling** | `expect` calls that can crash on malformed input | High |
| **Memory** | Heartbeat packet cloned every 5 seconds | Medium |
| **Architecture** | No separation between protocol and transport | Medium |
| **Architecture** | No shell command streaming protocol | High |
| **Architecture** | No file transfer protocol (delta-sync) | High |
| **Architecture** | No remote desktop protocol (TixRP) | High |
| **Testing** | No unit tests | Critical |
| **Documentation** | Missing API documentation | Medium |

### Code Problems

```rust
// PROBLEM: Panic on invalid input - crashes entire process
impl From<u32> for MessageType {
    fn from(value: u32) -> Self {
        match value {
            0x1 => MessageType::Command,
            0x2 => MessageType::Response,
            _ => panic!("Invalid MessageType value"), // BAD
        }
    }
}

// PROBLEM: Using only 4 bytes of 32-byte Blake3 hash
let checksum = blake3::hash(&payload);
header.set_checksum(u32::from_le_bytes(
    checksum.as_bytes()[0..4].try_into().expect("Failed") // BAD
));

// PROBLEM: Inefficient heartbeat - clones packet every tick
let heartbeat_packet = crate::Packet::heartbeat();
tokio::spawn(async move {
    loop {
        tick();
        if let Err(_) = tx.send(heartbeat_packet.clone()).await { // BAD
            break;
        }
    }
});
```

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                         TIX Architecture                         │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                      tix-core                            │   │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────────┐ │   │
│  │  │ Protocol │ │  Codec   │ │ Network  │ │    State     │ │   │
│  │  │   Layer  │ │          │ │   Layer  │ │   Machines   │ │   │
│  │  └──────────┘ └──────────┘ └──────────┘ └──────────────┘ │   │
│  │       │              │             │            │          │   │
│  │       └──────────────┴─────────────┴────────────┘          │   │
│  │                        │                                     │   │
│  │              ┌─────────▼─────────┐                          │   │
│  │              │   Service Layer   │                          │   │
│  │              │ (Shell, File,     │                          │   │
│  │              │  Screen, Update)  │                          │   │
│  │              └───────────────────┘                          │   │
│  └──────────────────────────────────────────────────────────┘   │
│                              │                                    │
│              ┌───────────────┼───────────────┐                   │
│              │               │               │                   │
│              ▼               ▼               ▼                   │
│      ┌─────────────┐ ┌─────────────┐ ┌─────────────┐             │
│      │  tix-master │ │  tix-slave  │ │  tix-cli   │             │
│      │  (GUI/CLI)  │ │  (Service)  │ │  (Bootstrap)│             │
│      └─────────────┘ └─────────────┘ └─────────────┘             │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Module Structure

```
tix/
├── Cargo.toml                    # Workspace manifest
├── AGENTS.md                     # Developer guidelines
├── PLAN.md                       # This file
│
├── tix-core/                     # Core protocol library
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs               # Public exports
│   │   ├── error.rs              # Domain errors (thiserror)
│   │   ├── types/                # Protocol types
│   │   │   ├── mod.rs
│   │   │   ├── header.rs         # Packet header (packed C struct)
│   │   │   ├── command.rs        # Command enum
│   │   │   ├── response.rs       # Response enum
│   │   │   ├── flags.rs          # Protocol flags
│   │   │   └── version.rs        # Protocol version
│   │   ├── packet/               # Packet structures
│   │   │   ├── mod.rs
│   │   │   ├── builder.rs        # Packet builder
│   │   │   ├── validator.rs      # Packet validation
│   │   │   └── raw.rs            # Raw packet types
│   │   ├── codec/                 # Encoding/decoding
│   │   │   ├── mod.rs
│   │   │   ├── decoder.rs         # Length-delimited decoder
│   │   │   └── encoder.rs         # Length-prefixed encoder
│   │   ├── crypto/                # Cryptography
│   │   │   ├── mod.rs
│   │   │   ├── checksum.rs        # Blake3 verification
│   │   │   └── handshake.rs       # Version negotiation
│   │   ├── network/               # Connection management
│   │   │   ├── mod.rs
│   │   │   ├── connection.rs      # Main connection struct
│   │   │   ├── transport.rs       # Low-level transport
│   │   │   ├── heartbeat.rs       # Keep-alive
│   │   │   └── channels.rs        # MPSC channels
│   │   ├── protocol/              # High-level protocols
│   │   │   ├── mod.rs
│   │   │   ├── shell.rs           # Shell command protocol
│   │   │   ├── file.rs            # File transfer protocol
│   │   │   ├── screen.rs          # Remote desktop protocol
│   │   │   └── update.rs          # Update protocol
│   │   ├── executor/              # Command execution
│   │   │   ├── mod.rs
│   │   │   ├── shell.rs           # PTY-based execution
│   │   │   ├── file_ops.rs        # File operations
│   │   │   └── system.rs          # System actions
│   │   ├── compression/           # Data compression
│   │   │   ├── mod.rs
│   │   │   └── zstd.rs           # Zstandard wrapper
│   │   ├── state/                 # State machines
│   │   │   ├── mod.rs
│   │   │   ├── master.rs          # Master state
│   │   │   ├── slave.rs           # Slave state
│   │   │   └── connection.rs      # Connection state
│   │   ├── task/                   # Async task management
│   │   │   ├── mod.rs
│   │   │   ├── pool.rs            # Task pool
│   │   │   ├── handler.rs         # Task handlers
│   │   │   └── tracker.rs         # Task tracking
│   │   └── tests/                  # Unit tests
│   │       ├── mod.rs
│   │       ├── codec_tests.rs
│   │       ├── packet_tests.rs
│   │       └── protocol_tests.rs
│   └── benches/                    # Benchmarks
│       ├── packet_bench.rs
│       └── codec_bench.rs
│
├── tix-master/                    # Master application
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs               # Entry point
│   │   ├── app.rs                # Application struct
│   │   ├── ui/                    # TUI components
│   │   │   ├── mod.rs
│   │   │   ├── terminal.rs        # Terminal setup
│   │   │   ├── screens/           # UI screens
│   │   │   │   ├── mod.rs
│   │   │   │   ├── main.rs
│   │   │   │   ├── file_browser.rs
│   │   │   │   └── terminal.rs
│   │   │   ├── components/        # Reusable components
│   │   │   └── widgets/
│   │   ├── controller/            # UI controller
│   │   ├── events/                # Event handling
│   │   └── master.rs             # Master logic
│   └── tests/                     # Integration tests
│
├── tix-slave/                     # Slave application
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs               # Entry point
│   │   ├── service.rs            # Windows service (optional)
│   │   ├── slave.rs              # Slave logic
│   │   ├── handlers/              # Command handlers
│   │   │   ├── mod.rs
│   │   │   ├── shell_handler.rs
│   │   │   ├── file_handler.rs
│   │   │   └── system_handler.rs
│   │   └── registry.rs            # Windows registry (if needed)
│   └── tests/
│
├── tix-cli/                       # CLI tool (optional)
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       └── commands/
│
└── scripts/                       # Build/development scripts
```

---

## Rewrite Goals

### Non-Goals

- ❌ Hackish quick fixes
- ❌ Over-abstraction for the sake of architecture
- ❌ Premature optimization
- ❌ Reinventing the wheel (use established crates)

### Goals

| Goal | Description | Priority |
|------|-------------|----------|
| **Correctness** | Zero panics on invalid input | P0 |
| **Type Safety** | Proper enums, no magic numbers | P0 |
| **Testability** | 80%+ unit test coverage | P0 |
| **Extensibility** | Easy to add new commands | P1 |
| **Performance** | < 1ms latency for commands | P1 |
| **Maintainability** | Clear module boundaries | P1 |
| **Documentation** | All public APIs documented | P2 |

---

## Core Components

### 1. Error Handling

```rust
// error.rs - Using thiserror for proper error types
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TixError {
    #[error("Connection lost: {source}")]
    ConnectionLost { source: std::io::Error },

    #[error("Protocol violation: {message}")]
    ProtocolViolation { message: &'static str },

    #[error("Invalid packet: {reason}")]
    InvalidPacket { reason: &'static str },

    #[error("Checksum mismatch - expected {expected:?}, got {actual:?}")]
    ChecksumMismatch { expected: [u8; 32], actual: [u8; 32] },

    #[error("Unsupported protocol version: {version}")]
    UnsupportedVersion { version: u32 },

    #[error("Command {command} not implemented")]
    UnimplementedCommand { command: u64 },

    #[error("Timeout after {duration:?}")]
    Timeout { duration: std::time::Duration },

    #[error("IO error: {source}")]
    Io { source: std::io::Error },

    #[error("Encoding error: {source}")]
    Encoding { source: bincode::Error },

    #[error("Compression error: {source}")]
    Compression { source: zstd::Error },
}

// Convenient From implementations
impl From<std::io::Error> for TixError {
    fn from(source: std::io::Error) -> Self {
        Self::Io { source }
    }
}
```

### 2. Protocol Header (Fixed)

```rust
// types/header.rs - Packed C struct for wire compatibility
use bytemuck::{Zeroable, Pod};

/// TIX Protocol Header - 64 bytes total
/// All fields are little-endian
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Zeroable, Pod)]
pub struct PacketHeader {
    /// Magic bytes: b'TIX0' or b'TIX1'
    pub magic: [u8; 4],

    /// Blake3 checksum of payload (full 32 bytes)
    pub checksum: [u8; 32],

    /// Message type: Command (0x1) or Response (0x2)
    pub message_type: u32,

    /// Protocol flags (compression, encryption, etc.)
    pub flags: u64,

    /// Unique request/response identifier
    pub request_id: u64,

    /// Payload length in bytes
    pub payload_length: u64,

    /// Protocol version for negotiation
    pub version: u32,

    /// Reserved for future use (padding)
    _reserved: [u8; 8],
}

impl PacketHeader {
    /// Current protocol version
    pub const CURRENT_VERSION: u32 = 0x1;

    /// Magic bytes for TIX version 0
    pub const MAGIC_TIX0: [u8; 4] = *b"TIX0";

    /// Magic bytes for TIX version 1
    pub const MAGIC_TIX1: [u8; 4] = *b"TIX1";

    /// Total header size
    pub const SIZE: usize = std::mem::size_of::<Self>();

    /// Validates magic bytes
    pub fn is_valid_magic(&self) -> bool {
        self.magic == Self::MAGIC_TIX0 || self.magic == Self::MAGIC_TIX1
    }

    /// Extracts version from magic bytes
    pub fn version_from_magic(&self) -> Option<u32> {
        match self.magic {
            Self::MAGIC_TIX0 => Some(0),
            Self::MAGIC_TIX1 => Some(1),
            _ => None,
        }
    }
}
```

### 3. Commands and Responses

```rust
// types/command.rs
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use std::fmt;

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromPrimitive)]
pub enum Command {
    /// Protocol-level commands
    Ping = 0x0001,
    Hello = 0x0002,
    Goodbye = 0x0003,
    Heartbeat = 0x0004,

    /// Shell commands
    ShellExecute = 0x0101,
    ShellCancel = 0x0102,
    ShellResize = 0x0103,

    /// File operations
    FileList = 0x0201,
    FileRead = 0x0202,
    FileWrite = 0x0203,
    FileDelete = 0x0204,
    FileCopy = 0x0205,
    FileMove = 0x0206,
    FileMkdir = 0x0207,

    /// System commands
    SystemInfo = 0x0301,
    SystemAction = 0x0302,
    ProcessList = 0x0303,

    /// Screen capture (TixRP)
    ScreenStart = 0x0401,
    ScreenStop = 0x0402,
    ScreenFrame = 0x0403,
    InputMouse = 0x0404,
    InputKeyboard = 0x0405,

    /// Update protocol (TixUpdate)
    UpdateCheck = 0x0501,
    UpdateDownload = 0x0502,
    UpdateApply = 0x0503,
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Command {
    /// Returns true if this command expects a response
    pub fn expects_response(self) -> bool {
        !matches!(self, Command::Heartbeat)
    }
}
```

### 4. Protocol Flags

```rust
// types/flags.rs
use bitflags::bitflags;

bitflags! {
    #[repr(C)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct ProtocolFlags: u64 {
        /// Payload is compressed with Zstandard
        const COMPRESSED = 0x0001_0000_0000_0001;

        /// Payload is encrypted
        const ENCRYPTED = 0x0002_0000_0000_0000;

        /// Payload is chunked (for large transfers)
        const CHUNKED = 0x0004_0000_0000_0000;

        /// This is the last chunk
        const LAST_CHUNK = 0x0008_0000_0000_0000;

        /// Shell output is streaming
        const SHELL_STREAMING = 0x0010_0000_0000_0000;

        /// Screen capture is high quality
        const SCREEN_HQ = 0x0020_0000_0000_0000;

        /// Request acknowledgment
        const ACK_REQUIRED = 0x0040_0000_0000_0000;
    }
}
```

---

## Protocol Design

### 1. Connection Lifecycle

```
┌─────────────────────────────────────────────────────────────────┐
│                    Connection Lifecycle                          │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  Master                    │  Slave                               │
│                           │                                      │
│  ──────────────────────────┼───────────────────────────────────  │
│                           │                                      │
│  1. Listen on port        │                                      │
│  ◄────────────────────────│── Accept connection                 │
│                           │                                      │
│  2. Send Hello (v1)       │                                      │
│  ─────────────────────────►│                                      │
│                           │                                      │
│                           │  3. Verify version                   │
│  ◄────────────────────────│── Send HelloResponse                 │
│                           │                                      │
│  4. Connection ready     │  4. Connection ready                 │
│                           │                                      │
│  ─────────────────────────┼───────────────────────────────────  │
│                           │                                      │
│  5. Send Command ────────►│  6. Execute command                 │
│                           │                                      │
│  ◄──── Send Response ─────│                                      │
│                           │                                      │
│  ─────────────────────────┼───────────────────────────────────  │
│                           │                                      │
│  7. Heartbeat (optional)  │  7. Heartbeat (optional)             │
│  ◄────────────────────────│──►                                   │
│                           │                                      │
│  ─────────────────────────┼───────────────────────────────────  │
│                           │                                      │
│  8. Goodbye ─────────────►│  8. Goodbye                          │
│  ◄────────────────────────│──►                                   │
│                           │                                      │
│  9. Close connection      │  9. Close connection                 │
│                           │                                      │
└─────────────────────────────────────────────────────────────────┘
```

### 2. Hello Handshake

```rust
// Wire format:
// [Header: 64 bytes]
//   - magic: b"TIX1"
//   - message_type: Command::Hello as u32
//   - flags: ProtocolFlags::empty()
//   - request_id: 0
//   - payload_length: size of hello payload
//   - version: CURRENT_VERSION
//
// [Payload: HelloPayload]
//   - client_name: String (UTF-8)
//   - supported_versions: Vec<u32>
//   - capabilities: Capabilities struct
//   - timestamp: u64 (Unix timestamp)

#[derive(Debug, Serialize, Deserialize)]
pub struct HelloPayload {
    /// Name identifying this client
    pub client_name: String,

    /// Protocol versions this client supports
    pub supported_versions: Vec<u32>,

    /// Client capabilities
    pub capabilities: Capabilities,

    /// Unix timestamp of when this was sent
    pub timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Capabilities {
    /// Supports shell streaming
    pub shell_streaming: bool,

    /// Supports file delta-sync
    pub file_delta_sync: bool,

    /// Supports screen capture
    pub screen_capture: bool,

    /// Supports compression
    pub compression: bool,

    /// Maximum payload size in bytes
    pub max_payload_size: u64,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            shell_streaming: true,
            file_delta_sync: true,
            screen_capture: true,
            compression: true,
            max_payload_size: 1024 * 1024 * 10, // 10MB default
        }
    }
}
```

### 3. Shell Command Protocol

```
┌─────────────────────────────────────────────────────────────────┐
│                  Shell Command Protocol                         │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  Master ──[ShellExecute]─────────────────────────────► Slave    │
│  Payload: {                                                      │
│    "command": "dir /w",                                         │
│    "pty": true,                                                  │
│    "timeout": 30000,                                            │
│    "env": {"VAR": "value"}                                      │
│  }                                                               │
│                                                                  │
│  Slave ──[ShellOutput (streaming)]──────────────────► Master   │
│  Payload: {                                                      │
│    "chunk_number": 0,                                           │
│    "data": "C:\\Users\\...",                                    │
│    "is_stdout": true                                             │
│  }                                                               │
│                                                                  │
│  Slave ──[ShellExit]────────────────────────────────► Master     │
│  Payload: {                                                      │
│    "exit_code": 0,                                              │
│    "total_chunks": 42                                            │
│  }                                                               │
│                                                                  │
│  Master ──[ShellCancel]─────────────────────────────► Slave    │
│  Payload: {}                                                     │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### 4. File Transfer Protocol

```
┌─────────────────────────────────────────────────────────────────┐
│                   File Transfer Protocol                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  File Metadata:                                                  │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │ Path: String (UTF-8)                                    │   │
│  │ Size: u64 (bytes)                                       │   │
│  │ Modified: u64 (Unix timestamp)                          │   │
│  │ Permissions: u32 (Unix-style permissions)               │   │
│  │ IsDirectory: bool                                        │   │
│  │ Blake3 Hash (optional): [u8; 32]                        │   │
│  └─────────────────────────────────────────────────────────┘   │
│                                                                  │
│  Delta Sync (if both ends have Blake3 hashes):                  │
│  1. Send chunk list with hashes                                 │
│  2. Receiver identifies matching chunks                         │
│  3. Sender only transmits new/modified chunks                    │
│  4. Receiver reconstructs file                                  │
│                                                                  │
│  Chunked Transfer:                                              │
│  - Header chunk (metadata)                                      │
│  - Data chunks (max 64KB each)                                  │
│  - Footer chunk (final hash for verification)                   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### 5. Remote Desktop Protocol (TixRP)

```
┌─────────────────────────────────────────────────────────────────┐
│                Remote Desktop Protocol (TixRP)                   │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  Screen Start:                                                   │
│  Master ──[ScreenStart]─────────────────────────────► Slave     │
│  Payload: {                                                      │
│    "quality": 0-100,                                            │
│    "fps": 1-60,                                                  │
│    "region": optional (x, y, width, height)                      │
│  }                                                               │
│                                                                  │
│  Screen Frame (compressed):                                       │
│  Slave ──[ScreenFrame]─────────────────────────────► Master     │
│  Payload: {                                                      │
│    "frame_number": u64,                                          │
│    "timestamp": u64,                                             │
│    "width": u32,                                                 │
│    "height": u32,                                                │
│    "format": "jpeg" | "png" | "raw",                             │
│    "data": [...],                                                │
│    "cursor": optional ({x, y, visible})                          │
│  }                                                               │
│                                                                  │
│  Input Injection:                                                │
│  Master ──[InputMouse]─────────────────────────────► Slave      │
│  Master ──[InputKeyboard]──────────────────────────► Slave       │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Implementation Plan

### Phase 1: Foundation (Week 1)

#### 1.1 Error Handling
- [ ] Implement `TixError` with `thiserror`
- [ ] Remove all `panic!` calls
- [ ] Add proper `From` implementations
- [ ] Create error conversion utilities

#### 1.2 Protocol Types
- [ ] Implement `PacketHeader` with `bytemuck`
- [ ] Add proper endianness handling
- [ ] Create command/response enums with `num_derive`
- [ ] Implement flags with `bitflags`
- [ ] Add version negotiation types

#### 1.3 Packet Layer
- [ ] Create packet builder pattern
- [ ] Implement packet validation
- [ ] Add Blake3 checksum (full 32 bytes)
- [ ] Write unit tests for packet operations

### Phase 2: Codec & Network (Week 2)

#### 2.1 Codec Implementation
- [ ] Implement `LengthDelimitedCodec` wrapper
- [ ] Add proper error handling in decoder
- [ ] Implement encoder with zero-copy where possible
- [ ] Write codec unit tests

#### 2.2 Connection Management
- [ ] Refactor `Connection` struct
- [ ] Implement graceful shutdown
- [ ] Add connection state machine
- [ ] Optimize heartbeat (no cloning)
- [ ] Add backpressure handling

#### 2.3 Channel Management
- [ ] Design proper MPSC channel architecture
- [ ] Add channel capacity limits
- [ ] Implement proper error propagation

### Phase 3: Protocols (Week 3)

#### 3.1 Shell Protocol
- [ ] Implement streaming shell execution
- [ ] Add PTY support
- [ ] Handle shell resize events
- [ ] Implement output streaming

#### 3.2 File Protocol
- [ ] Implement metadata transfer
- [ ] Add chunked file transfer
- [ ] Implement delta-sync algorithm
- [ ] Add Blake3 verification

#### 3.3 Remote Desktop Protocol
- [ ] Design frame format
- [ ] Implement capture abstraction
- [ ] Add input injection
- [ ] Implement compression

### Phase 4: State & Tasks (Week 4)

#### 4.1 State Machines
- [ ] Refactor master state machine
- [ ] Refactor slave state machine
- [ ] Implement connection state tracking
- [ ] Add proper state transitions

#### 4.2 Task System
- [ ] Simplify task pool design
- [ ] Add proper error handling
- [ ] Implement task cancellation
- [ ] Add task timeouts

### Phase 5: Integration (Week 5)

#### 5.1 Master Application
- [ ] Update TUI with new protocol
- [ ] Add proper event handling
- [ ] Implement command queue
- [ ] Add status updates

#### 5.2 Slave Application
- [ ] Implement command handlers
- [ ] Add proper cleanup
- [ ] Implement auto-reconnect
- [ ] Add Windows service support

#### 5.3 Integration Testing
- [ ] Test full connection lifecycle
- [ ] Test all command types
- [ ] Test error scenarios
- [ ] Test performance benchmarks

### Phase 6: Polish (Week 6)

#### 6.1 Documentation
- [ ] Add API documentation
- [ ] Write architecture guide
- [ ] Add example usage

#### 6.2 Performance
- [ ] Profile critical paths
- [ ] Optimize hot paths
- [ ] Add benchmarks
- [ ] Memory profiling

#### 6.3 Cleanup
- [ ] Remove dead code
- [ ] Simplify where possible
- [ ] Final code review
- [ ] Lint passes

---

## Phase 7: tix-rdp Ultra-Fast Remote Desktop

### Overview

tix-rdp is an ultra-low latency remote desktop protocol designed specifically for direct RJ-45 Ethernet connections (100MB/s). Unlike traditional remote desktop solutions that must account for variable network conditions, tix-rdp is optimized for a dedicated, high-bandwidth, low-latency peer-to-peer connection.

### Design Philosophy

**Key Principles:**

1. **Latency First**: Every design decision prioritizes minimizing end-to-end latency over bandwidth efficiency
2. **Zero-Copy Pipeline**: Minimize memory copies from capture to display
3. **Hardware Acceleration**: Leverage DXGI Desktop Duplication API for Windows screen capture
4. **Adaptive Quality**: Dynamically adjust encoding based on available bandwidth and motion
5. **Direct Connection**: No intermediate servers, no NAT traversal, no encryption overhead (trusted network)

### Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    tix-rdp Architecture                          │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  SLAVE (Target)                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  Screen Capture Pipeline                                 │   │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌─────────┐ │   │
│  │  │   DXGI   │→ │  Frame   │→ │  Delta   │→ │  Encode │ │   │
│  │  │ Capture  │  │  Buffer  │  │  Detect  │  │         │ │   │
│  │  └──────────┘  └──────────┘  └──────────┘  └─────────┘ │   │
│  │       │              │              │            │        │   │
│  │       └──────────────┴──────────────┴────────────┘        │   │
│  │                      │                                     │   │
│  │              ┌───────▼────────┐                            │   │
│  │              │  Frame Queue   │                            │   │
│  │              │  (bounded)     │                            │   │
│  │              └───────┬────────┘                            │   │
│  │                      │                                     │   │
│  │              ┌───────▼────────┐                            │   │
│  │              │ Network Sender │                            │   │
│  │              │ (TCP/UDP)      │                            │   │
│  │              └───────┬────────┘                            │   │
│  └──────────────────────┼────────────────────────────────────┘   │
│                         │ 100MB/s Direct RJ45                     │
│  ┌──────────────────────┼────────────────────────────────────┐   │
│  │                      │                                     │   │
│  │              ┌───────▼────────┐                            │   │
│  │              │Network Receiver│                            │   │
│  │              └───────┬────────┘                            │   │
│  │                      │                                     │   │
│  │              ┌───────▼────────┐                            │   │
│  │              │  Frame Decoder │                            │   │
│  │              └───────┬────────┘                            │   │
│  │                      │                                     │   │
│  │              ┌───────▼────────┐                            │   │
│  │              │  Display       │                            │   │
│  │              │  Renderer      │                            │   │
│  │              └────────────────┘                            │   │
│  │                                                      MASTER │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                  │
│  Input Injection (Master → Slave):                               │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐                      │
│  │  Mouse   │→ │  Input   │→ │  Inject  │                      │
│  │  Events  │  │  Queue   │  │  Events  │                      │
│  └──────────┘  └──────────┘  └──────────┘                      │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Performance Targets

| Metric | Target | Rationale |
|--------|--------|-----------|
| **End-to-end latency** | < 16ms (60 FPS) | Human perception threshold |
| **Capture latency** | < 2ms | DXGI Desktop Duplication |
| **Encode latency** | < 4ms | Hardware-accelerated encoding |
| **Network latency** | < 1ms | Direct RJ45, same switch |
| **Decode latency** | < 2ms | Optimized decoder |
| **Display latency** | < 1ms | Direct rendering |
| **Frame rate** | 60 FPS (adaptive) | Smooth interaction |
| **Bandwidth usage** | Up to 80 MB/s | Leave headroom for other traffic |

### Phase 7.1: Screen Capture (Week 7)

#### 7.1.1 DXGI Desktop Duplication

```rust
// tix-core/src/protocol/screen/capture.rs

use windows::{
    core::*,
    Win32::Graphics::Dxgi::*,
    Win32::Graphics::Direct3D::*,
    Win32::Graphics::Direct3D11::*,
};

/// DXGI-based screen capturer for Windows
/// Provides lowest latency capture method
pub struct DxgiCapturer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    duplication: IDXGIOutputDuplication,
    staging_texture: ID3D11Texture2D,
    output_desc: DXGI_OUTPUT_DESC,
}

impl DxgiCapturer {
    /// Initialize capturer for the primary monitor
    pub fn new() -> Result<Self> {
        unsafe {
            // Create D3D11 device
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                None,
                D3D11_CREATE_DEVICE_DEBUG,
                &[],
                D3D11_SDK_VERSION,
                &mut device,
                None,
                &mut context,
            )?;

            let device = device.unwrap();
            let context = context.unwrap();

            // Get DXGI device
            let dxgi_device: IDXGIDevice = device.cast()?;
            let adapter = dxgi_device.GetAdapter()?;
            let output = adapter.EnumOutputs(0)?;

            // Get output description
            let output_desc = output.GetDesc()?;

            // Create duplication
            let output1: IDXGIOutput1 = output.cast()?;
            let duplication = output1.DuplicateOutput(&device)?;

            // Create staging texture for CPU access
            let staging_desc = D3D11_TEXTURE2D_DESC {
                Width: output_desc.DesktopCoordinates.right - output_desc.DesktopCoordinates.left,
                Height: output_desc.DesktopCoordinates.bottom - output_desc.DesktopCoordinates.top,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ,
                MiscFlags: 0,
            };

            let mut staging_texture = None;
            device.CreateTexture2D(&staging_desc, None, &mut staging_texture)?;
            let staging_texture = staging_texture.unwrap();

            Ok(Self {
                device,
                context,
                duplication,
                staging_texture,
                output_desc,
            })
        }
    }

    /// Capture next frame with timeout
    pub fn capture_frame(&mut self, timeout_ms: u32) -> Result<ScreenFrame> {
        unsafe {
            // Acquire next frame
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource = None;
            
            match self.duplication.AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource) {
                Ok(()) => {
                    let resource = resource.unwrap();
                    let texture: ID3D11Texture2D = resource.cast()?;

                    // Copy to staging texture
                    self.context.CopyResource(&self.staging_texture, &texture);

                    // Release frame
                    let _ = self.duplication.ReleaseFrame();

                    // Map for CPU access
                    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                    self.context.Map(
                        &self.staging_texture,
                        0,
                        D3D11_MAP_READ,
                        0,
                        &mut mapped,
                    )?;

                    let width = self.output_desc.DesktopCoordinates.right - self.output_desc.DesktopCoordinates.left;
                    let height = self.output_desc.DesktopCoordinates.bottom - self.output_desc.DesktopCoordinates.top;
                    let stride = mapped.RowPitch as usize;
                    let data = std::slice::from_raw_parts(
                        mapped.pData as *const u8,
                        (stride * height as usize),
                    );

                    // Create frame
                    let frame = ScreenFrame {
                        width: width as u32,
                        height: height as u32,
                        stride: stride as u32,
                        format: PixelFormat::Bgra8,
                        data: data.to_vec(),
                        timestamp: std::time::Instant::now(),
                    };

                    self.context.Unmap(&self.staging_texture, 0);
                    Ok(frame)
                }
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                    Err(TixError::Timeout {
                        duration: std::time::Duration::from_millis(timeout_ms as u64),
                    })
                }
                Err(e) => Err(e.into()),
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScreenFrame {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: PixelFormat,
    pub data: Vec<u8>,
    pub timestamp: std::time::Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Bgra8,
    Rgba8,
    Rgb8,
}
```

#### 7.1.2 Delta Detection

```rust
// tix-core/src/protocol/screen/delta.rs

use std::cmp;

/// Detects changes between consecutive frames
/// For 100MB/s connection, we can afford to send more data
pub struct DeltaDetector {
    previous_frame: Option<ScreenFrame>,
    block_size: usize,
}

impl DeltaDetector {
    pub fn new(block_size: usize) -> Self {
        Self {
            previous_frame: None,
            block_size,
        }
    }

    /// Compute delta between current and previous frame
    pub fn detect_delta(&mut self, current: &ScreenFrame) -> DeltaFrame {
        if let Some(previous) = &self.previous_frame {
            // Compute block-level differences
            let mut changed_blocks = Vec::new();
            
            let blocks_x = (current.width as usize + self.block_size - 1) / self.block_size;
            let blocks_y = (current.height as usize + self.block_size - 1) / self.block_size;

            for by in 0..blocks_y {
                for bx in 0..blocks_x {
                    let start_x = bx * self.block_size;
                    let start_y = by * self.block_size;
                    let end_x = cmp::min(start_x + self.block_size, current.width as usize);
                    let end_y = cmp::min(start_y + self.block_size, current.height as usize);

                    if self.block_changed(current, previous, start_x, start_y, end_x, end_y) {
                        changed_blocks.push(Block {
                            x: start_x as u32,
                            y: start_y as u32,
                            width: (end_x - start_x) as u32,
                            height: (end_y - start_y) as u32,
                        });
                    }
                }
            }

            let delta = DeltaFrame {
                frame_number: 0, // Set by caller
                timestamp: current.timestamp,
                width: current.width,
                height: current.height,
                changed_blocks,
                full_frame: changed_blocks.is_empty(), // Send full if no changes detected (edge case)
            };

            self.previous_frame = Some(current.clone());
            delta
        } else {
            // First frame - send full frame
            let delta = DeltaFrame {
                frame_number: 0,
                timestamp: current.timestamp,
                width: current.width,
                height: current.height,
                changed_blocks: vec![Block {
                    x: 0,
                    y: 0,
                    width: current.width,
                    height: current.height,
                }],
                full_frame: true,
            };

            self.previous_frame = Some(current.clone());
            delta
        }
    }

    fn block_changed(
        &self,
        current: &ScreenFrame,
        previous: &ScreenFrame,
        start_x: usize,
        start_y: usize,
        end_x: usize,
        end_y: usize,
    ) -> bool {
        for y in start_y..end_y {
            let current_row = &current.data[y * current.stride as usize..];
            let previous_row = &previous.data[y * previous.stride as usize..];
            
            for x in start_x..end_x {
                let current_pixel = &current_row[x * 4..x * 4 + 4];
                let previous_pixel = &previous_row[x * 4..x * 4 + 4];
                
                if current_pixel != previous_pixel {
                    return true;
                }
            }
        }
        false
    }
}

#[derive(Debug, Clone)]
pub struct DeltaFrame {
    pub frame_number: u64,
    pub timestamp: std::time::Instant,
    pub width: u32,
    pub height: u32,
    pub changed_blocks: Vec<Block>,
    pub full_frame: bool,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
```

### Phase 7.2: Encoding (Week 7)

#### 7.2.1 Adaptive Encoder

```rust
// tix-core/src/protocol/screen/encoder.rs

use zstd::stream::Encoder;

/// Adaptive encoder that adjusts quality based on bandwidth
/// For 100MB/s direct connection, we can use high quality
pub struct AdaptiveEncoder {
    compressor: Encoder<'static, Vec<u8>>,
    quality: u8,
    target_bandwidth: u64, // bytes per second
    current_bandwidth: u64,
    frame_count: u64,
}

impl AdaptiveEncoder {
    pub fn new(target_bandwidth: u64) -> Result<Self> {
        let compressor = Encoder::new(Vec::new(), 3)?; // Default compression level
        Ok(Self {
            compressor,
            quality: 90, // Start with high quality
            target_bandwidth,
            current_bandwidth: target_bandwidth,
            frame_count: 0,
        })
    }

    /// Encode a delta frame
    pub fn encode(&mut self, frame: &DeltaFrame, source: &ScreenFrame) -> Result<EncodedFrame> {
        let mut data = Vec::new();

        // For high bandwidth, we can use lossless or near-lossless
        // For 100MB/s, we have ~1.6MB per frame at 60 FPS
        // 1920x1080x4 = ~8.3MB per frame uncompressed
        // We need ~5:1 compression to fit in bandwidth

        if frame.full_frame {
            // Encode full frame
            self.encode_full_frame(source, &mut data)?;
        } else {
            // Encode only changed blocks
            self.encode_delta_frame(frame, source, &mut data)?;
        }

        // Compress
        let compressed = self.compressor.compress(&data)?;

        self.frame_count += 1;

        Ok(EncodedFrame {
            frame_number: frame.frame_number,
            timestamp: frame.timestamp,
            width: frame.width,
            height: frame.height,
            data: compressed,
            is_full_frame: frame.full_frame,
        })
    }

    fn encode_full_frame(&self, frame: &ScreenFrame, output: &mut Vec<u8>) -> Result<()> {
        // Simple raw encoding for now
        // TODO: Add hardware-accelerated encoding (NVENC, QuickSync)
        output.extend_from_slice(&frame.data);
        Ok(())
    }

    fn encode_delta_frame(&self, delta: &DeltaFrame, source: &ScreenFrame, output: &mut Vec<u8>) -> Result<()> {
        // Encode block headers and data
        for block in &delta.changed_blocks {
            // Block header
            output.extend_from_slice(&block.x.to_le_bytes());
            output.extend_from_slice(&block.y.to_le_bytes());
            output.extend_from_slice(&block.width.to_le_bytes());
            output.extend_from_slice(&block.height.to_le_bytes());

            // Block data
            let start_y = block.y as usize;
            let end_y = start_y + block.height as usize;
            let start_x = block.x as usize * 4;
            let end_x = start_x + block.width as usize * 4;

            for y in start_y..end_y {
                let row_start = y * source.stride as usize;
                output.extend_from_slice(&source.data[row_start + start_x..row_start + end_x]);
            }
        }
        Ok(())
    }

    /// Adjust quality based on measured bandwidth
    pub fn adjust_quality(&mut self, measured_bandwidth: u64) {
        self.current_bandwidth = measured_bandwidth;

        // If we're using less than 80% of target bandwidth, increase quality
        if measured_bandwidth < self.target_bandwidth * 8 / 10 {
            self.quality = (self.quality + 5).min(100);
        }
        // If we're exceeding target bandwidth, reduce quality
        else if measured_bandwidth > self.target_bandwidth {
            self.quality = self.quality.saturating_sub(5);
        }
    }
}

#[derive(Debug, Clone)]
pub struct EncodedFrame {
    pub frame_number: u64,
    pub timestamp: std::time::Instant,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    pub is_full_frame: bool,
}
```

### Phase 7.3: Network Transport (Week 8)

#### 7.3.1 UDP for Screen Data

```rust
// tix-core/src/protocol/screen/transport.rs

use tokio::net::UdpSocket;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};

/// UDP transport for screen data
/// UDP is acceptable for screen data because:
/// - Loss is tolerable (next frame will have the data)
/// - Latency is critical
/// - We have 100MB/s bandwidth to spare
pub struct ScreenTransport {
    socket: UdpSocket,
    remote_addr: SocketAddr,
    sequence: AtomicU32,
    mtu: usize,
}

impl ScreenTransport {
    pub fn new(socket: UdpSocket, remote_addr: SocketAddr) -> Self {
        // Calculate MTU (account for UDP header)
        let mtu = 1400 - 28; // Standard Ethernet MTU minus IP+UDP headers

        Self {
            socket,
            remote_addr,
            sequence: AtomicU32::new(0),
            mtu,
        }
    }

    /// Send encoded frame, splitting into MTU-sized packets
    pub async fn send_frame(&self, frame: &EncodedFrame) -> Result<()> {
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);

        // Create frame header
        let header = FrameHeader {
            sequence,
            frame_number: frame.frame_number,
            timestamp: frame.timestamp.elapsed().as_micros() as u64,
            width: frame.width,
            height: frame.height,
            is_full_frame: frame.is_full_frame,
            total_chunks: ((frame.data.len() + self.mtu - 1) / self.mtu) as u32,
        };

        // Send header packet
        let header_bytes = header.encode()?;
        self.socket.send_to(&header_bytes, self.remote_addr).await?;

        // Send data chunks
        for (chunk_index, chunk) in frame.data.chunks(self.mtu).enumerate() {
            let chunk_header = ChunkHeader {
                sequence,
                chunk_index: chunk_index as u32,
                chunk_size: chunk.len() as u32,
            };

            let mut packet = Vec::with_capacity(chunk_header.encoded_size() + chunk.len());
            packet.extend_from_slice(&chunk_header.encode()?);
            packet.extend_from_slice(chunk);

            self.socket.send_to(&packet, self.remote_addr).await?;
        }

        Ok(())
    }

    /// Receive frame
    pub async fn receive_frame(&self) -> Result<EncodedFrame> {
        let mut header_buf = vec![0u8; FrameHeader::MAX_SIZE];
        let (len, _) = self.socket.recv_from(&mut header_buf).await?;
        let header = FrameHeader::decode(&header_buf[..len])?;

        let mut data = Vec::with_capacity((header.total_chunks as usize) * self.mtu);
        let mut received_chunks = 0;

        while received_chunks < header.total_chunks as usize {
            let mut chunk_buf = vec![0u8; ChunkHeader::MAX_SIZE + self.mtu];
            let (len, _) = self.socket.recv_from(&mut chunk_buf).await?;
            
            let chunk_header = ChunkHeader::decode(&chunk_buf[..ChunkHeader::MAX_SIZE])?;
            if chunk_header.sequence != header.sequence {
                continue; // Drop out-of-order chunks
            }

            let chunk_data = &chunk_buf[ChunkHeader::MAX_SIZE..len];
            data.extend_from_slice(chunk_data);
            received_chunks += 1;
        }

        Ok(EncodedFrame {
            frame_number: header.frame_number,
            timestamp: std::time::Instant::now() - std::time::Duration::from_micros(header.timestamp),
            width: header.width,
            height: header.height,
            data,
            is_full_frame: header.is_full_frame,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct FrameHeader {
    sequence: u32,
    frame_number: u64,
    timestamp: u64,
    width: u32,
    height: u32,
    is_full_frame: bool,
    total_chunks: u32,
}

impl FrameHeader {
    const MAX_SIZE: usize = 32;

    fn encode(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(Self::MAX_SIZE);
        buf.extend_from_slice(&self.sequence.to_le_bytes());
        buf.extend_from_slice(&self.frame_number.to_le_bytes());
        buf.extend_from_slice(&self.timestamp.to_le_bytes());
        buf.extend_from_slice(&self.width.to_le_bytes());
        buf.extend_from_slice(&self.height.to_le_bytes());
        buf.push(self.is_full_frame as u8);
        buf.extend_from_slice(&self.total_chunks.to_le_bytes());
        Ok(buf)
    }

    fn decode(data: &[u8]) -> Result<Self> {
        Ok(Self {
            sequence: u32::from_le_bytes(data[0..4].try_into()?),
            frame_number: u64::from_le_bytes(data[4..12].try_into()?),
            timestamp: u64::from_le_bytes(data[12..20].try_into()?),
            width: u32::from_le_bytes(data[20..24].try_into()?),
            height: u32::from_le_bytes(data[24..28].try_into()?),
            is_full_frame: data[28] != 0,
            total_chunks: u32::from_le_bytes(data[29..33].try_into()?),
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct ChunkHeader {
    sequence: u32,
    chunk_index: u32,
    chunk_size: u32,
}

impl ChunkHeader {
    const MAX_SIZE: usize = 12;

    fn encoded_size(&self) -> usize {
        Self::MAX_SIZE
    }

    fn encode(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(Self::MAX_SIZE);
        buf.extend_from_slice(&self.sequence.to_le_bytes());
        buf.extend_from_slice(&self.chunk_index.to_le_bytes());
        buf.extend_from_slice(&self.chunk_size.to_le_bytes());
        Ok(buf)
    }

    fn decode(data: &[u8]) -> Result<Self> {
        Ok(Self {
            sequence: u32::from_le_bytes(data[0..4].try_into()?),
            chunk_index: u32::from_le_bytes(data[4..8].try_into()?),
            chunk_size: u32::from_le_bytes(data[8..12].try_into()?),
        })
    }
}
```

### Phase 7.4: Input Injection (Week 8)

#### 7.4.1 Mouse and Keyboard Input

```rust
// tix-core/src/protocol/screen/input.rs

use windows::Win32::UI::WindowsAndMessaging::*;
use windows::Win32::System::Console::*;

/// Input injector for slave machine
pub struct InputInjector;

impl InputInjector {
    /// Inject mouse event
    pub fn inject_mouse(&self, event: MouseEvent) -> Result<()> {
        unsafe {
            let mut input = INPUT::default();
            input.r#type = INPUT_MOUSE;
            
            input.Anonymous.mi = MOUSEINPUT {
                dx: event.x,
                dy: event.y,
                mouseData: event.mouse_data,
                dwFlags: event.flags,
                time: 0,
                dwExtraInfo: 0,
            };

            SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            Ok(())
        }
    }

    /// Inject keyboard event
    pub fn inject_keyboard(&self, event: KeyEvent) -> Result<()> {
        unsafe {
            let mut input = INPUT::default();
            input.r#type = INPUT_KEYBOARD;
            
            input.Anonymous.ki = KEYBDINPUT {
                wVk: event.virtual_key,
                wScan: event.scan_code,
                dwFlags: event.flags,
                time: 0,
                dwExtraInfo: 0,
            };

            SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            Ok(())
        }
    }
}

#[derive(Debug, Clone)]
pub struct MouseEvent {
    pub x: i32,
    pub y: i32,
    pub flags: MOUSE_EVENT_FLAGS,
    pub mouse_data: u32,
}

#[derive(Debug, Clone)]
pub struct KeyEvent {
    pub virtual_key: VIRTUAL_KEY,
    pub scan_code: u16,
    pub flags: KEYBD_EVENT_FLAGS,
}
```

### Phase 7.5: Master Display (Week 8)

#### 7.5.1 Frame Renderer

```rust
// tix-master/src/display/renderer.rs

use std::time::Instant;

/// Frame renderer for master display
pub struct FrameRenderer {
    window: WindowHandle,
    decoder: FrameDecoder,
    frame_buffer: Vec<u8>,
    last_frame_time: Instant,
    fps_counter: FpsCounter,
}

impl FrameRenderer {
    pub fn new(window: WindowHandle) -> Self {
        Self {
            window,
            decoder: FrameDecoder::new(),
            frame_buffer: Vec::new(),
            last_frame_time: Instant::now(),
            fps_counter: FpsCounter::new(),
        }
    }

    /// Render received frame
    pub fn render(&mut self, frame: &EncodedFrame) -> Result<()> {
        // Decode frame
        let decoded = self.decoder.decode(frame)?;

        // Update frame buffer
        self.frame_buffer = decoded.data;

        // Render to window
        self.window.render(&self.frame_buffer, decoded.width, decoded.height)?;

        // Update FPS counter
        let now = Instant::now();
        let elapsed = now - self.last_frame_time;
        self.fps_counter.record_frame(elapsed);
        self.last_frame_time = now;

        Ok(())
    }

    pub fn fps(&self) -> f64 {
        self.fps_counter.fps()
    }
}

struct FrameDecoder;

impl FrameDecoder {
    fn new() -> Self {
        Self
    }

    fn decode(&self, frame: &EncodedFrame) -> Result<DecodedFrame> {
        // Decompress
        let decompressed = zstd::decode_all(&*frame.data)?;

        Ok(DecodedFrame {
            width: frame.width,
            height: frame.height,
            data: decompressed,
        })
    }
}

#[derive(Debug)]
struct DecodedFrame {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

struct FpsCounter {
    frames: Vec<Duration>,
}

impl FpsCounter {
    fn new() -> Self {
        Self {
            frames: Vec::with_capacity(60),
        }
    }

    fn record_frame(&mut self, elapsed: Duration) {
        self.frames.push(elapsed);
        if self.frames.len() > 60 {
            self.frames.remove(0);
        }
    }

    fn fps(&self) -> f64 {
        if self.frames.is_empty() {
            return 0.0;
        }
        let avg: f64 = self.frames.iter().map(|d| d.as_secs_f64()).sum::<f64>() / self.frames.len() as f64;
        1.0 / avg
    }
}
```

### Phase 7.6: Integration (Week 9)

#### 7.6.1 Slave Screen Service

```rust
// tix-slave/src/screen_service.rs

use tix_core::protocol::screen::*;

pub struct ScreenService {
    capturer: DxgiCapturer,
    delta_detector: DeltaDetector,
    encoder: AdaptiveEncoder,
    transport: ScreenTransport,
    running: bool,
}

impl ScreenService {
    pub fn new(transport: ScreenTransport) -> Result<Self> {
        let capturer = DxgiCapturer::new()?;
        let delta_detector = DeltaDetector::new(64); // 64x64 blocks
        let encoder = AdaptiveEncoder::new(100 * 1024 * 1024)?; // 100MB/s target

        Ok(Self {
            capturer,
            delta_detector,
            encoder,
            transport,
            running: false,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        self.running = true;
        let mut frame_number = 0u64;

        while self.running {
            // Capture frame
            let frame = self.capturer.capture_frame(16)?; // 16ms timeout for 60 FPS

            // Detect delta
            let mut delta = self.delta_detector.detect_delta(&frame);
            delta.frame_number = frame_number;

            // Encode
            let encoded = self.encoder.encode(&delta, &frame)?;

            // Send
            self.transport.send_frame(&encoded).await?;

            frame_number += 1;
        }

        Ok(())
    }

    pub fn stop(&mut self) {
        self.running = false;
    }
}
```

#### 7.6.2 Master Screen Client

```rust
// tix-master/src/screen_client.rs

use tix_core::protocol::screen::*;

pub struct ScreenClient {
    transport: ScreenTransport,
    renderer: FrameRenderer,
    input_queue: InputQueue,
}

impl ScreenClient {
    pub fn new(transport: ScreenTransport, window: WindowHandle) -> Self {
        Self {
            transport,
            renderer: FrameRenderer::new(window),
            input_queue: InputQueue::new(),
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut receive_task = tokio::spawn({
            let transport = self.transport.clone();
            async move {
                loop {
                    let frame = transport.receive_frame().await?;
                    // Send to renderer
                }
            }
        });

        let mut send_input_task = tokio::spawn({
            let transport = self.transport.clone();
            let mut queue = self.input_queue.clone();
            async move {
                while let Some(input) = queue.pop().await {
                    // Send input to slave
                }
            }
        });

        tokio::select! {
            result = receive_task => result??,
            result = send_input_task => result??,
        }

        Ok(())
    }
}
```

### Phase 7.7: Optimization (Week 9)

#### 7.7.1 Performance Checklist

- [ ] Profile capture pipeline with `perf` or `VTune`
- [ ] Optimize delta detection algorithm (SIMD?)
- [ ] Implement hardware-accelerated encoding (NVENC/QuickSync)
- [ ] Use zero-copy for network buffers
- [ ] Optimize frame buffer allocation (object pooling)
- [ ] Implement frame skipping when behind
- [ ] Add adaptive quality based on motion detection
- [ ] Optimize display rendering (GPU acceleration)

#### 7.7.2 Benchmark Targets

```rust
// benches/screen_bench.rs

use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_capture(c: &mut Criterion) {
    let mut capturer = DxgiCapturer::new().unwrap();

    c.bench_function("dxgi_capture", |b| {
        b.iter(|| {
            black_box(&mut capturer).capture_frame(16).unwrap()
        })
    });
}

fn bench_delta_detection(c: &mut Criterion) {
    let frame = create_test_frame(1920, 1080);
    let mut detector = DeltaDetector::new(64);

    c.bench_function("delta_detection_1080p", |b| {
        b.iter(|| {
            black_box(&mut detector).detect_delta(&frame)
        })
    });
}

fn bench_encoding(c: &mut Criterion) {
    let frame = create_test_delta_frame(1920, 1080);
    let source = create_test_frame(1920, 1080);
    let mut encoder = AdaptiveEncoder::new(100 * 1024 * 1024).unwrap();

    c.bench_function("encode_1080p", |b| {
        b.iter(|| {
            black_box(&mut encoder).encode(&frame, &source).unwrap()
        })
    });
}

criterion_group!(benches, bench_capture, bench_delta_detection, bench_encoding);
criterion_main!(benches);
```

### Phase 7.8: Testing (Week 9)

#### 7.8.1 Integration Tests

```rust
// tix-core/tests/screen_integration.rs

#[tokio::test]
async fn test_full_screen_pipeline() {
    // Setup master and slave
    let (master_addr, slave_addr) = setup_test_sockets().await;

    // Start slave screen service
    let slave = ScreenService::new(slave_addr).unwrap();
    tokio::spawn(async move {
        slave.run().await.unwrap();
    });

    // Start master screen client
    let master = ScreenClient::new(master_addr, create_test_window()).unwrap();
    tokio::spawn(async move {
        master.run().await.unwrap();
    });

    // Wait for some frames
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Verify FPS
    assert!(master.fps() > 30.0);
}
```

### Phase 7.9: Documentation (Week 9)

- [ ] Document DXGI capture setup requirements
- [ ] Document network configuration (MTU, QoS)
- [ ] Document performance tuning options
- [ ] Add troubleshooting guide
- [ ] Document input injection security considerations

---

## Phase 8: GUI Client and Slave RDP Service

### Overview

Phase 8 focuses on building the user-facing applications for tix-rdp:
- **GUI Client (tix-rdp-gui)**: A graphical application running on the master machine that displays the remote desktop and captures user input
- **Slave RDP Service (tix-rdp-slave)**: A background service running on the slave machine that captures the screen and sends it to the master

### Design Philosophy

**Key Principles:**

1. **Native Performance**: Use platform-native GUI frameworks (Windows API, WPF, or modern alternatives) for maximum performance
2. **Minimal Overhead**: The GUI should be as lightweight as possible to avoid interfering with screen capture
3. **Responsive UI**: Non-blocking UI with async operations for network I/O
4. **Seamless Integration**: The slave service should integrate with Windows as a background service
5. **Direct Rendering**: Use GPU-accelerated rendering for the remote desktop display

### Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    tix-rdp Architecture                         │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  MASTER (tix-rdp-gui)                                        │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  GUI Layer (Windows/WPF/Modern)                   │   │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────────┐ │   │
│  │  │  Window  │  │  Input   │  │   Display    │ │   │
│  │  │ Manager  │  │  Capture │  │   Renderer   │ │   │
│  │  └──────────┘  └──────────┘  └──────────────┘ │   │
│  │       │              │              │            │          │   │
│  │       └──────────────┴─────────────┴────────────┘          │   │
│  │                      │                                     │   │
│  │              ┌───────▼────────┐                            │   │
│  │              │  RDP Client    │                            │   │
│  │              │  (from tix-   │                            │   │
│  │              │   core)         │                            │   │
│  │              └───────┬────────┘                            │   │
│  └──────────────────────┼────────────────────────────────────┘   │
│                         │ 100MB/s Direct RJ45                     │
│  ┌──────────────────────┼────────────────────────────────────┐   │
│  │                      │                                     │   │
│  │              ┌───────▼────────┐                            │   │
│  │              │  RDP Service   │                            │   │
│  │              │  (from tix-   │                            │   │
│  │              │   core)         │                            │   │
│  │              └───────┬────────┘                            │   │
│  │                      │                                     │   │
│  │              ┌───────▼────────┐                            │   │
│  │              │  Screen        │                            │   │
│  │              │  Capture       │                            │   │
│  │              │  (DXGI)        │                            │   │
│  │              └────────────────┘                            │   │
│  │                                                      SLAVE │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Phase 8.1: GUI Client (Week 10)

#### 8.1.1 Technology Selection

**Options for GUI Framework:**

| Framework | Pros | Cons | Recommendation |
|-----------|-------|-------|----------------|
| **WPF** | Mature, Windows-native, GPU acceleration | Older technology, larger runtime | Good for Windows-only |
| **WinUI 3** | Modern, Fluent Design, Windows 11 native | Newer, smaller ecosystem | Best for modern Windows |
| **Tauri** | Cross-platform, Rust-based, small bundle | Web-based rendering | Good for cross-platform |
| **Slint** | Rust-native, lightweight, GPU-accelerated | Newer, smaller community | Good for performance |

**Recommendation**: Use **WinUI 3** for Windows-only deployment (best native integration) or **Tauri** if cross-platform support is needed.

#### 8.1.2 GUI Client Structure

```rust
// tix-rdp-gui/src/main.rs

use tix_core::rdp::*;
use windows::Win32::UI::WindowsAndMessaging::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize GUI
    let app = RdpGuiApp::new()?;
    
    // Run event loop
    app.run().await?;
    
    Ok(())
}

pub struct RdpGuiApp {
    window: WindowHandle,
    rdp_client: RdpClient,
    display_renderer: DisplayRenderer,
    input_capture: InputCapture,
    connection_state: ConnectionState,
}

impl RdpGuiApp {
    pub fn new() -> Result<Self> {
        let window = WindowHandle::create("TIX RDP", 1920, 1080)?;
        let rdp_client = RdpClient::new();
        let display_renderer = DisplayRenderer::new(&window)?;
        let input_capture = InputCapture::new(&window)?;
        
        Ok(Self {
            window,
            rdp_client,
            display_renderer,
            input_capture,
            connection_state: ConnectionState::Disconnected,
        })
    }
    
    pub async fn run(&mut self) -> Result<()> {
        // Connect to slave
        self.connect_to_slave().await?;
        
        // Start RDP client
        let rdp_task = tokio::spawn({
            let mut client = self.rdp_client.clone();
            async move {
                client.run().await
            }
        });
        
        // Run GUI event loop
        loop {
            // Process GUI events
            self.process_gui_events().await?;
            
            // Render received frames
            if let Some(frame) = self.rdp_client.next_frame().await {
                self.display_renderer.render(&frame)?;
            }
            
            // Check connection state
            if self.connection_state == ConnectionState::Disconnected {
                break;
            }
        }
        
        rdp_task.await??;
        Ok(())
    }
    
    async fn connect_to_slave(&mut self) -> Result<()> {
        let addr = "192.168.1.100:7331".parse()?; // Slave address
        self.rdp_client.connect(addr).await?;
        self.connection_state = ConnectionState::Connected;
        Ok(())
    }
    
    async fn process_gui_events(&mut self) -> Result<()> {
        // Process window events
        while let Some(event) = self.window.next_event()? {
            match event {
                WindowEvent::Close => {
                    self.rdp_client.disconnect().await?;
                    self.connection_state = ConnectionState::Disconnected;
                }
                WindowEvent::Resize(width, height) => {
                    self.display_renderer.resize(width, height)?;
                }
                WindowEvent::KeyPress(key) => {
                    self.rdp_client.send_input(InputEvent::Keyboard(key)).await?;
                }
                WindowEvent::MouseMove(x, y) => {
                    self.rdp_client.send_input(InputEvent::MouseMove(x, y)).await?;
                }
                WindowEvent::MouseButton(button, pressed) => {
                    self.rdp_client.send_input(InputEvent::MouseButton(button, pressed)).await?;
                }
            }
        }
        Ok(())
    }
}

pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}
```

#### 8.1.3 Display Renderer

```rust
// tix-rdp-gui/src/display/renderer.rs

use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::*;

pub struct DisplayRenderer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    swap_chain: IDXGISwapChain,
    texture: ID3D11Texture2D,
    shader_view: ID3D11ShaderResourceView,
}

impl DisplayRenderer {
    pub fn new(window: &WindowHandle) -> Result<Self> {
        unsafe {
            // Create D3D11 device
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                None,
                0,
                &[],
                D3D11_SDK_VERSION,
                &mut device,
                None,
                &mut context,
            )?;

            let device = device.unwrap();
            let context = context.unwrap();

            // Create swap chain
            let dxgi_device: IDXGIDevice = device.cast()?;
            let adapter = dxgi_device.GetAdapter()?;
            let factory = adapter.GetParent::<IDXGIFactory2>()?;

            let swap_chain_desc = DXGI_SWAP_CHAIN_DESC {
                BufferCount: 2,
                Width: window.width(),
                Height: window.height(),
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Windowed: TRUE.into(),
                ..Default::default()
            };

            let mut swap_chain = None;
            factory.CreateSwapChain(
                &dxgi_device,
                &swap_chain_desc,
                &mut swap_chain,
            )?;

            let swap_chain = swap_chain.unwrap();

            // Create texture for frame data
            let texture_desc = D3D11_TEXTURE2D_DESC {
                Width: window.width(),
                Height: window.height(),
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_SHADER_RESOURCE,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
                MiscFlags: 0,
            };

            let mut texture = None;
            device.CreateTexture2D(&texture_desc, None, &mut texture)?;
            let texture = texture.unwrap();

            // Create shader resource view
            let mut shader_view = None;
            device.CreateShaderResourceView(&texture, None, &mut shader_view)?;
            let shader_view = shader_view.unwrap();

            Ok(Self {
                device,
                context,
                swap_chain,
                texture,
                shader_view,
            })
        }
    }

    pub fn render(&mut self, frame: &EncodedFrame) -> Result<()> {
        unsafe {
            // Decode frame
            let decoded = decode_frame(frame)?;

            // Map texture
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(
                &self.texture,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                &mut mapped,
            )?;

            // Copy frame data to texture
            let dst = std::slice::from_raw_parts_mut(
                mapped.pData as *mut u8,
                mapped.RowPitch as usize * frame.height as usize,
            );
            dst.copy_from_slice(&decoded.data);

            // Unmap
            self.context.Unmap(&self.texture, 0);

            // Present
            self.swap_chain.Present(1, 0);
        }

        Ok(())
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        unsafe {
            // Resize swap chain buffers
            self.swap_chain.ResizeBuffers(0, width, height, DXGI_FORMAT_UNKNOWN, 0);
        }
        Ok(())
    }
}
```

#### 8.1.4 Input Capture

```rust
// tix-rdp-gui/src/input/capture.rs

use windows::Win32::UI::WindowsAndMessaging::*;

pub struct InputCapture {
    window: WindowHandle,
}

impl InputCapture {
    pub fn new(window: &WindowHandle) -> Result<Self> {
        Ok(Self {
            window: window.clone(),
        })
    }

    pub fn capture_events(&self) -> Result<Vec<InputEvent>> {
        let mut events = Vec::new();

        unsafe {
            // Peek at message queue
            let mut msg = MSG::default();
            while PeekMessageA(&mut msg, None, 0, 0, PM_REMOVE).into() {
                match msg.message {
                    WM_KEYDOWN | WM_KEYUP => {
                        let key_code = msg.wParam.0 as u16;
                        let pressed = msg.message == WM_KEYDOWN;
                        events.push(InputEvent::Keyboard(KeyEvent {
                            virtual_key: VIRTUAL_KEY(key_code),
                            scan_code: 0,
                            flags: if pressed {
                                KEYBD_EVENT_FLAGS(0)
                            } else {
                                KEYEVENTF_KEYUP
                            },
                        }));
                    }
                    WM_MOUSEMOVE => {
                        let x = (msg.lParam.0 & 0xFFFF) as i16;
                        let y = ((msg.lParam.0 >> 16) & 0xFFFF) as i16;
                        events.push(InputEvent::MouseMove(x as i32, y as i32));
                    }
                    WM_LBUTTONDOWN | WM_LBUTTONUP => {
                        let pressed = msg.message == WM_LBUTTONDOWN;
                        events.push(InputEvent::MouseButton(MouseButton::Left, pressed));
                    }
                    WM_RBUTTONDOWN | WM_RBUTTONUP => {
                        let pressed = msg.message == WM_RBUTTONDOWN;
                        events.push(InputEvent::MouseButton(MouseButton::Right, pressed));
                    }
                    WM_MBUTTONDOWN | WM_MBUTTONUP => {
                        let pressed = msg.message == WM_MBUTTONDOWN;
                        events.push(InputEvent::MouseButton(MouseButton::Middle, pressed));
                    }
                    WM_MOUSEWHEEL => {
                        let delta = (msg.wParam.0 >> 16) as i16;
                        events.push(InputEvent::MouseWheel(delta));
                    }
                    _ => {}
                }
            }
        }

        Ok(events)
    }
}

#[derive(Debug, Clone)]
pub enum InputEvent {
    Keyboard(KeyEvent),
    MouseMove(i32, i32),
    MouseButton(MouseButton, bool),
    MouseWheel(i16),
}

#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    pub virtual_key: VIRTUAL_KEY,
    pub scan_code: u16,
    pub flags: KEYBD_EVENT_FLAGS,
}

#[derive(Debug, Clone, Copy)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}
```

### Phase 8.2: Slave RDP Service (Week 10)

#### 8.2.1 Windows Service Integration

```rust
// tix-rdp-slave/src/main.rs

use tix_core::rdp::*;
use windows::Win32::System::Services::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Check if running as service
    if is_running_as_service() {
        run_as_service().await?;
    } else {
        run_as_console().await?;
    }
    
    Ok(())
}

fn is_running_as_service() -> bool {
    unsafe {
        // Check if parent process is services.exe
        let parent_process = get_parent_process_id();
        parent_process == SERVICES_PROCESS_ID
    }
}

async fn run_as_service() -> Result<()> {
    let service = RdpService::new()?;
    service.run().await?;
    Ok(())
}

async fn run_as_console() -> Result<()> {
    let service = RdpService::new()?;
    service.run().await?;
    Ok(())
}

pub struct RdpService {
    screen_service: ScreenService,
    input_injector: InputInjector,
    status_handle: SERVICE_STATUS_HANDLE,
}

impl RdpService {
    pub fn new() -> Result<Self> {
        let screen_service = ScreenService::new()?;
        let input_injector = InputInjector::new()?;
        
        Ok(Self {
            screen_service,
            input_injector,
            status_handle: None,
        })
    }
    
    pub async fn run(&mut self) -> Result<()> {
        // Register as Windows service
        self.register_service()?;
        
        // Report running status
        self.report_status(SERVICE_RUNNING, 0)?;
        
        // Start screen capture
        let capture_task = tokio::spawn({
            let mut service = self.screen_service.clone();
            async move {
                service.run().await
            }
        });
        
        // Wait for service stop
        self.wait_for_stop().await?;
        
        // Stop screen capture
        self.screen_service.stop();
        capture_task.await?;
        
        // Report stopped status
        self.report_status(SERVICE_STOPPED, 0)?;
        
        Ok(())
    }
    
    fn register_service(&mut self) -> Result<()> {
        unsafe {
            let service_table = [
                SERVICE_TABLE_ENTRYW {
                    lpServiceName: w!("TixRdpService\0").as_ptr(),
                    lpServiceProc: Some(service_main),
                },
                SERVICE_TABLE_ENTRYW::default(),
            ];
            
            StartServiceCtrlDispatcherW(service_table.as_ptr())?;
        }
        
        Ok(())
    }
    
    fn report_status(&self, state: u32, exit_code: u32) -> Result<()> {
        unsafe {
            if let Some(handle) = self.status_handle {
                let status = SERVICE_STATUS {
                    dwServiceType: SERVICE_WIN32_OWN_PROCESS,
                    dwCurrentState: state,
                    dwControlsAccepted: SERVICE_ACCEPT_STOP,
                    dwWin32ExitCode: exit_code,
                    dwServiceSpecificExitCode: 0,
                    dwCheckPoint: 0,
                    dwWaitHint: 0,
                };
                
                SetServiceStatus(handle, &status)?;
            }
        }
        
        Ok(())
    }
    
    async fn wait_for_stop(&self) -> Result<()> {
        // Wait for stop signal from service control manager
        tokio::signal::ctrl_c().await?;
        Ok(())
    }
}

extern "system" fn service_main(argc: u32, argv: *mut *mut u16) {
    // Service entry point
    // This is called by Windows Service Control Manager
}
```

#### 8.2.2 Service Configuration

```toml
# tix-rdp-slave/Cargo.toml

[package]
name = "tix-rdp-slave"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "tix-rdp-slave"
path = "src/main.rs"

[dependencies]
tix-core = { path = "../tix-core" }
tokio = { version = "1", features = ["full"] }
anyhow = "1.0"
tracing = "0.1"
tracing-subscriber = "0.3"

[dependencies.windows]
windows = { version = "0.52", features = [
    "Win32_Foundation",
    "Win32_System_Services",
    "Win32_UI_WindowsAndMessaging",
    "Win32_Graphics_Dxgi",
    "Win32_Graphics_Direct3D11",
] }

[target.'cfg(windows)'.build-dependencies]
winres = "0.1"
```

#### 8.2.3 Service Installation

```rust
// tix-rdp-slave/src/install.rs

use windows::Win32::System::Services::*;

pub fn install_service() -> Result<()> {
    unsafe {
        // Open Service Control Manager
        let scm = OpenSCManagerW(
            None,
            None,
            SC_MANAGER_CREATE_SERVICE,
        )?;

        // Get path to executable
        let exe_path = get_current_exe_path()?;

        // Create service
        let service = CreateServiceW(
            scm,
            w!("TixRdpService"),
            w!("TIX RDP Slave Service"),
            SERVICE_ALL_ACCESS,
            SERVICE_WIN32_OWN_PROCESS,
            SERVICE_AUTO_START,
            SERVICE_ERROR_NORMAL,
            exe_path.as_ptr(),
            None,
            None,
            None,
            None,
            None,
        )?;

        // Set service description
        let description = w!("Ultra-fast remote desktop screen capture service for TIX");
        ChangeServiceConfig2W(
            service,
            SERVICE_CONFIG_DESCRIPTION,
            &SERVICE_DESCRIPTIONW {
                lpDescription: description.as_ptr(),
            },
        )?;

        // Start service
        StartServiceW(service, None, None)?;

        CloseServiceHandle(service);
        CloseServiceHandle(scm);
    }

    Ok(())
}

pub fn uninstall_service() -> Result<()> {
    unsafe {
        // Open Service Control Manager
        let scm = OpenSCManagerW(
            None,
            None,
            SC_MANAGER_CONNECT,
        )?;

        // Open service
        let service = OpenServiceW(
            scm,
            w!("TixRdpService"),
            SERVICE_ALL_ACCESS,
        )?;

        // Stop service
        SERVICE_STATUS status;
        ControlService(service, SERVICE_CONTROL_STOP, &mut status)?;

        // Delete service
        DeleteService(service)?;

        CloseServiceHandle(service);
        CloseServiceHandle(scm);
    }

    Ok(())
}
```

### Phase 8.3: Configuration (Week 10)

#### 8.3.1 GUI Client Configuration

```toml
# tix-rdp-gui/config.toml

[network]
slave_address = "192.168.1.100:7331"
timeout_ms = 5000

[display]
width = 1920
height = 1080
fullscreen = false
vsync = true

[performance]
target_fps = 60
buffer_size = 3
quality = "high"

[input]
capture_mouse = true
capture_keyboard = true
capture_clipboard = false

[logging]
level = "info"
file = "tix-rdp-gui.log"
```

#### 8.3.2 Slave Service Configuration

```toml
# tix-rdp-slave/config.toml

[network]
listen_port = 7331
max_connections = 1

[screen]
capture_quality = "high"
fps = 60
delta_detection = true
block_size = 64

[performance]
target_bandwidth_mbps = 100
adaptive_quality = true

[logging]
level = "info"
file = "tix-rdp-slave.log"
```

### Phase 8.4: Testing (Week 10)

#### 8.4.1 GUI Client Tests

```rust
// tix-rdp-gui/tests/integration.rs

#[tokio::test]
async fn test_gui_client_connection() {
    // Start mock slave
    let mock_slave = MockSlave::new().await;
    
    // Start GUI client
    let mut client = RdpGuiApp::new().unwrap();
    client.connect_to_slave(mock_slave.addr()).await.unwrap();
    
    // Verify connection
    assert_eq!(client.connection_state(), ConnectionState::Connected);
    
    // Send input
    client.send_input(InputEvent::MouseMove(100, 100)).await.unwrap();
    
    // Verify input received
    assert!(mock_slave.received_input().await);
}

#[tokio::test]
async fn test_display_rendering() {
    let renderer = DisplayRenderer::new(&create_test_window()).unwrap();
    let frame = create_test_frame(1920, 1080);
    
    renderer.render(&frame).unwrap();
    
    // Verify rendering (would need to capture output)
}
```

#### 8.4.2 Slave Service Tests

```rust
// tix-rdp-slave/tests/integration.rs

#[tokio::test]
async fn test_service_lifecycle() {
    let service = RdpService::new().unwrap();
    
    // Start service
    let handle = tokio::spawn(async move {
        service.run().await
    });
    
    // Wait for service to start
    tokio::time::sleep(Duration::from_secs(1)).await;
    
    // Verify service is running
    assert!(is_service_running("TixRdpService"));
    
    // Stop service
    service.stop();
    handle.await.unwrap();
    
    // Verify service is stopped
    assert!(!is_service_running("TixRdpService"));
}

#[tokio::test]
async fn test_screen_capture() {
    let mut service = RdpService::new().unwrap();
    
    // Start screen capture
    let capture_task = tokio::spawn(async move {
        service.screen_service.run().await
    });
    
    // Wait for some frames
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Stop capture
    service.screen_service.stop();
    capture_task.await.unwrap();
    
    // Verify frames were captured
    assert!(service.screen_service.frame_count() > 0);
}
```

### Phase 8.5: Documentation (Week 10)

- [ ] Document GUI client installation and setup
- [ ] Document slave service installation (Windows service)
- [ ] Document configuration options
- [ ] Add troubleshooting guide for common issues
- [ ] Document performance tuning
- [ ] Add user manual with screenshots

### Phase 8.6: Deployment (Week 10)

#### 8.6.1 Build Configuration

```toml
# Cargo.toml workspace configuration

[profile.release]
lto = true
codegen-units = 1
opt-level = 3
panic = "abort"

[profile.release.package."tix-rdp-gui"]
strip = true

[profile.release.package."tix-rdp-slave"]
strip = true
```

#### 8.6.2 Installer Creation

```powershell
# scripts/build-installer.ps1

# Build GUI client
cargo build --release --bin tix-rdp-gui

# Build slave service
cargo build --release --bin tix-rdp-slave

# Create installer using WiX or Inno Setup
& "C:\Program Files (x86)\Inno Setup 6\ISCC.exe" installer-script.iss
```

```iss
; installer-script.iss

[Setup]
AppName=TIX RDP
AppVersion=0.1.0
DefaultDirName={pf}\TIX RDP
DefaultGroupName=TIX RDP
OutputBaseFilename=tix-rdp-setup.exe
Compression=lzma2
SolidCompression=yes

[Files]
Source: "target\release\tix-rdp-gui.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "target\release\tix-rdp-slave.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "config\*.toml"; DestDir: "{app}\config"; Flags: ignoreversion

[Icons]
Name: "{group}\TIX RDP Client"; Filename: "{app}\tix-rdp-gui.exe"
Name: "{group}\Uninstall TIX RDP"; Filename: "{uninstallexe}"

[Run]
Filename: "{app}\tix-rdp-slave.exe"; Parameters: "--install"; Description: "Install RDP Slave Service"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{app}\tix-rdp-slave.exe"; Parameters: "--uninstall"; RunOnceId: "UninstallService"
```

---

## Testing Strategy

### Test Pyramid

```
         ┌───────────────┐
         │   Integration │  (End-to-end tests)
         │     Tests     │
         └───────┬───────┘
                 │
         ┌───────▼───────┐
         │     Unit      │  (Individual component tests)
         │    Tests      │
         └───────┬───────┘
                 │
         ┌───────▼───────┐
         │    Property   │  (Property-based tests)
         │    Based      │
         └───────────────┘
```

### Test Coverage Goals

| Component | Target Coverage |
|-----------|----------------|
| Protocol types | 100% |
| Codec | 100% |
| Network layer | 90% |
| Shell protocol | 85% |
| File protocol | 85% |
| State machines | 90% |
| **Overall** | **85%** |

### Example Tests

```rust
// packet_tests.rs

#[cfg(test)]
mod packet_tests {
    use super::*;
    use proptest::prelude::*;

    /// Property-based test: packet roundtrip encoding
    proptest! {
        #[test]
        fn test_packet_roundtrip(packet in any::<Packet>()) {
            let encoded = packet.encode().unwrap();
            let decoded = Packet::decode(&encoded).unwrap();
            prop_assert_eq!(packet, decoded);
        }
    }

    /// Property-based test: header checksum verification
    proptest! {
        #[test]
        fn test_header_checksum(header in any::<PacketHeader>()) {
            let mut header = header;
            let checksum = blake3::hash(&bytemuck::bytes_of(&header));
            header.checksum = checksum;
            assert!(header.verify_checksum());
        }
    }

    /// Unit test: invalid magic bytes
    #[test]
    fn test_invalid_magic_bytes() {
        let mut header = PacketHeader::zeroed();
        header.magic = *b"XXXX";

        assert!(!header.is_valid_magic());
        assert_eq!(header.version_from_magic(), None);
    }

    /// Unit test: command display
    #[test]
    fn test_command_display() {
        assert_eq!(Command::Ping.to_string(), "Ping");
        assert_eq!(Command::ShellExecute.to_string(), "ShellExecute");
    }
}

// integration_tests.rs

#[tokio::test]
async fn test_full_connection_cycle() {
    // Setup
    let master_addr = start_test_master().await;
    let slave = connect_to_master(master_addr).await;

    // Handshake
    let version = slave.handshake().await.unwrap();
    assert_eq!(version, ProtocolVersion::V1);

    // Test ping-pong
    let response = slave.send_command(Command::Ping).await.unwrap();
    assert_eq!(response, Response::Pong);

    // Cleanup
    slave.goodbye().await;
}
```

---

## Performance Considerations

### Memory Management

1. **Zero-Copy Parsing**
   - Use `bytes::Bytes` for payload references
   - Avoid copying in codec layer
   - Use `BufReader` for streaming

2. **Arena Allocation**
   - Use `bumpalo` for transient allocations
   - Pre-allocate buffers for frequent operations

3. **Object Pooling**
   - Pool packet objects for high-frequency operations
   - Reuse buffers where appropriate

### Latency Optimization

1. **Connection Setup**
   - Minimal handshake (2 round-trips maximum)
   - Disable Nagle's algorithm
   - Enable TCP_NODELAY

2. **Data Transfer**
   - Batch small commands where possible
   - Use streaming for large data
   - Compress based on size threshold

3. **CPU Usage**
   - Async I/O for network operations
   - Spawn blocking operations to separate thread pool
   - Profile to identify bottlenecks

### Benchmarks

```rust
// benches/packet_bench.rs

use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_packet_encoding(c: &mut Criterion) {
    let packet = create_test_packet(1024);

    c.bench_function("packet_encode_1kb", |b| {
        b.iter(|| black_box(&packet).encode().unwrap())
    });

    c.bench_function("packet_encode_64kb", |b| {
        let packet = create_test_packet(64 * 1024);
        b.iter(|| black_box(&packet).encode().unwrap())
    });
}

fn bench_checksum(c: &mut Criterion) {
    let data = vec![0u8; 1024];

    c.bench_function("blake3_1kb", |b| {
        b.iter(|| blake3::hash(black_box(&data)))
    });
}

criterion_group!(benches, bench_packet_encoding, bench_checksum);
criterion_main!(benches);
```

---

## Migration Path

### Breaking Changes

| Change | Rationale | Migration Guide |
|--------|-----------|-----------------|
| Header size 44→64 bytes | Full Blake3, version field | Protocol version bump |
| Command enum changes | Better organization | Update command IDs |
| Error type changes | Proper error handling | Update error handling |

### Backward Compatibility

- Protocol version negotiation allows old clients
- Graceful degradation for unknown commands
- Feature flags for optional capabilities

### Rollout Plan

1. **Phase 1**: Deploy new core library
2. **Phase 2**: Update master
3. **Phase 3**: Update slave
4. **Phase 4**: Monitor and iterate

---

## Dependencies

### Core Dependencies

```toml
# tix-core/Cargo.toml

[package]
name = "tix-core"
version = "0.1.0"
edition = "2021"

[dependencies]
# Async runtime
tokio = { version = "1", features = ["full", "tracing"] }

# Serialization
bincode = "1.3"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# Cryptography
blake3 = "1.5"

# Compression
zstd = "0.12"

# Byte manipulation
bytes = "1.5"
bytemuck = "1.14"
bitflags = "2.4"

# Error handling
thiserror = "1.0"

# Concurrency
crossbeam = "0.8"

# Numbers
num-traits = "0.2"
num-derive = "0.4"

[dev-dependencies]
# Testing
proptest = "1.3"
criterion = "0.5"
tempfile = "3.8"

# Async testing
tokio-test = "0.4"
```

### Application Dependencies

```toml
# tix-master/Cargo.toml

[dependencies]
tix-core = { path = "../tix-core" }

# TUI
crossterm = "0.27"
ratatui = "0.23"

# Async runtime
tokio = { version = "1", features = ["full"] }

# Other
anyhow = "1.0"
```

---

## Success Criteria

### Functional

- [ ] All commands work correctly
- [ ] File transfer with delta-sync
- [ ] Shell command streaming
- [ ] Remote desktop capture
- [ ] Auto-update mechanism

### Non-Functional

- [ ] 85% test coverage
- [ ] < 1ms command latency
- [ ] No panics on invalid input
- [ ] Graceful error handling
- [ ] Documentation complete

### Code Quality

- [ ] Clippy passes with no warnings
- [ ] `cargo fmt` compliant
- [ ] No `unsafe` blocks (except FFI)
- [ ] All public APIs documented
- [ ] Architecture review passed

---

## References

### Protocol References

- [RDP Protocol Specification](https://learn.microsoft.com/en-us/windows-server/remote/remote-desktop-services/)
- [DXGI Desktop Duplication API](https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/desktop-dup-api)
- [VNC Protocol](https://github.com/rfbproto/rfbproto)

### Rust Best Practices

- [Effective Rust](https://www.lurklurk.org/effective-rust/)
- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- [Tokio Tutorial](https://tokio.rs/tokio/tutorial)

### Performance

- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [Criterion Benchmarks](https://bheisler.github.io/criterion.rs/book/)

---

## Appendix A: Command Reference

### Protocol Commands (0x0000-0x00FF)

| ID | Command | Description | Payload |
|----|---------|-------------|---------|
| 0x0001 | Ping | Keep-alive ping | Empty |
| 0x0002 | Hello | Connection handshake | HelloPayload |
| 0x0003 | Goodbye | Graceful disconnect | Empty |
| 0x0004 | Heartbeat | Periodic keep-alive | Empty |

### Shell Commands (0x0100-0x01FF)

| ID | Command | Description | Payload |
|----|---------|-------------|---------|
| 0x0101 | ShellExecute | Execute shell command | ShellRequest |
| 0x0102 | ShellCancel | Cancel running command | RequestID |
| 0x0103 | ShellResize | Resize PTY | PtySize |

### File Commands (0x0200-0x02FF)

| ID | Command | Description | Payload |
|----|---------|-------------|---------|
| 0x0201 | FileList | List directory | Path |
| 0x0202 | FileRead | Read file contents | FileRequest |
| 0x0203 | FileWrite | Write file contents | FileChunk |
| 0x0204 | FileDelete | Delete file/directory | Path |
| 0x0205 | FileCopy | Copy file/directory | CopyRequest |
| 0x0206 | FileMove | Move file/directory | MoveRequest |
| 0x0207 | FileMkdir | Create directory | Path |

### System Commands (0x0300-0x03FF)

| ID | Command | Description | Payload |
|----|---------|-------------|---------|
| 0x0301 | SystemInfo | Get system information | Empty |
| 0x0302 | SystemAction | Execute system action | Action |
| 0x0303 | ProcessList | List running processes | Empty |

### Screen Commands (0x0400-0x04FF)

| ID | Command | Description | Payload |
|----|---------|-------------|---------|
| 0x0401 | ScreenStart | Start screen capture | ScreenConfig |
| 0x0402 | ScreenStop | Stop screen capture | Empty |
| 0x0403 | ScreenFrame | Screen data frame | FrameData |
| 0x0404 | InputMouse | Mouse input event | MouseEvent |
| 0x0405 | InputKeyboard | Keyboard input event | KeyEvent |

### Update Commands (0x0500-0x05FF)

| ID | Command | Description | Payload |
|----|---------|-------------|---------|
| 0x0501 | UpdateCheck | Check for updates | Empty |
| 0x0502 | UpdateDownload | Download update | Version |
| 0x0503 | UpdateApply | Apply downloaded update | Empty |

---

## Appendix B: Wire Format Examples

### Minimal Ping Packet (48 bytes + payload)

```
Offset  Size  Field
──────────────────────────────────────
0x00    4     Magic: "TIX1"
0x04    32    Checksum: blake3("")
0x24    4     MessageType: 0x0001 (Ping)
0x28    8     Flags: 0x0
0x30    8     RequestID: 0x1
0x38    8     PayloadLength: 0x0
0x40    8     Version: 0x1
0x48    8     Reserved: 0x0
```

### Shell Execute Packet

```
Offset  Size  Field
──────────────────────────────────────
0x00    4     Magic: "TIX1"
0x04    32    Checksum: blake3(JSON payload)
0x24    4     MessageType: 0x0101 (ShellExecute)
0x28    8     Flags: SHELL_STREAMING
0x30    8     RequestID: 0x12345678
0x38    8     PayloadLength: 45
0x40    8     Version: 0x1
0x48    8     Reserved: 0x0
0x50    45    Payload: {"command":"dir","pty":true}
```

---

*Document Version: 1.0*  
*Last Updated: 2024*  
*Authors: TIX Development Team*
