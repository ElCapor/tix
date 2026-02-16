//! Shared connection state machine used by both master and slave.
//!
//! Provides a `ConnectionPhase` enum that models the full lifecycle
//! of a TIX peer connection, with validated transitions that return
//! `Result` instead of panicking.

use std::time::Instant;

use crate::error::TixError;

// ── ConnectionPhase ──────────────────────────────────────────────

/// The current phase of a TIX peer connection.
///
/// ```text
///  Disconnected ──► Connecting ──► Handshaking ──► Connected
///       ▲                │               │              │
///       │                ▼               ▼              ▼
///       └──────── Disconnecting ◄────────┴──────────────┘
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConnectionPhase {
    /// No active connection. Initial / terminal state.
    #[default]
    Disconnected,

    /// TCP connection initiated but not yet established.
    Connecting,

    /// TCP link is up; performing protocol handshake (Hello exchange).
    Handshaking,

    /// Handshake complete; ready for commands and responses.
    Connected {
        /// When the connection entered the `Connected` state.
        since: Instant,
    },

    /// Graceful shutdown in progress (Goodbye sent/received).
    Disconnecting,
}

impl std::fmt::Display for ConnectionPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => write!(f, "Disconnected"),
            Self::Connecting => write!(f, "Connecting"),
            Self::Handshaking => write!(f, "Handshaking"),
            Self::Connected { .. } => write!(f, "Connected"),
            Self::Disconnecting => write!(f, "Disconnecting"),
        }
    }
}

impl ConnectionPhase {
    /// Returns `true` when the connection is fully established and
    /// ready for protocol traffic.
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected { .. })
    }

    /// Returns `true` when the connection is in a terminal or idle state.
    pub fn is_disconnected(&self) -> bool {
        matches!(self, Self::Disconnected)
    }

    /// How long the connection has been in the `Connected` state.
    ///
    /// Returns `None` for any other phase.
    pub fn connected_duration(&self) -> Option<std::time::Duration> {
        match self {
            Self::Connected { since } => Some(since.elapsed()),
            _ => None,
        }
    }

    // ── Transitions ──────────────────────────────────────────────

    /// Transition to `Connecting`.
    ///
    /// Valid from: `Disconnected`.
    pub fn begin_connect(&mut self) -> Result<(), TixError> {
        match self {
            Self::Disconnected => {
                *self = Self::Connecting;
                Ok(())
            }
            _other => Err(TixError::ProtocolViolation(
                "cannot connect: not in Disconnected state",
            )),
        }
    }

    /// Transition to `Handshaking`.
    ///
    /// Valid from: `Connecting`.
    pub fn begin_handshake(&mut self) -> Result<(), TixError> {
        match self {
            Self::Connecting => {
                *self = Self::Handshaking;
                Ok(())
            }
            _ => Err(TixError::ProtocolViolation(
                "cannot handshake: not in Connecting state",
            )),
        }
    }

    /// Transition to `Connected`.
    ///
    /// Valid from: `Handshaking`.
    pub fn complete_handshake(&mut self) -> Result<(), TixError> {
        match self {
            Self::Handshaking => {
                *self = Self::Connected {
                    since: Instant::now(),
                };
                Ok(())
            }
            _ => Err(TixError::ProtocolViolation(
                "cannot complete handshake: not in Handshaking state",
            )),
        }
    }

    /// Transition to `Disconnecting`.
    ///
    /// Valid from: `Handshaking`, `Connected`.
    pub fn begin_disconnect(&mut self) -> Result<(), TixError> {
        match self {
            Self::Handshaking | Self::Connected { .. } => {
                *self = Self::Disconnecting;
                Ok(())
            }
            _ => Err(TixError::ProtocolViolation(
                "cannot disconnect: not in Handshaking or Connected state",
            )),
        }
    }

    /// Transition to `Disconnected`.
    ///
    /// Valid from: `Disconnecting`, `Connecting` (timeout/failure),
    /// `Handshaking` (failure).
    pub fn finish_disconnect(&mut self) -> Result<(), TixError> {
        match self {
            Self::Disconnecting | Self::Connecting | Self::Handshaking => {
                *self = Self::Disconnected;
                Ok(())
            }
            _ => Err(TixError::ProtocolViolation(
                "cannot finish disconnect: not in a disconnectable state",
            )),
        }
    }

    /// Force-reset to `Disconnected` regardless of current state.
    ///
    /// Use this for unrecoverable errors (e.g. I/O failure mid-stream).
    pub fn force_disconnect(&mut self) {
        *self = Self::Disconnected;
    }
}

// ── Capabilities ─────────────────────────────────────────────────

/// Capabilities advertised by a peer during the Hello handshake.
///
/// Both master and slave exchange these so each side knows what the
/// remote end supports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerCapabilities {
    /// Supports streaming shell output.
    pub shell_streaming: bool,

    /// Supports delta-sync file transfers.
    pub file_delta_sync: bool,

    /// Supports screen capture / remote desktop.
    pub screen_capture: bool,

    /// Supports zstd payload compression.
    pub compression: bool,

    /// Maximum payload size the peer will accept.
    pub max_payload_size: u64,
}

impl Default for PeerCapabilities {
    fn default() -> Self {
        Self {
            shell_streaming: true,
            file_delta_sync: true,
            screen_capture: true,
            compression: true,
            max_payload_size: crate::packet::MAX_PAYLOAD_SIZE as u64,
        }
    }
}

impl PeerCapabilities {
    /// Negotiate capabilities by taking the intersection of both peers.
    pub fn negotiate(&self, remote: &Self) -> Self {
        Self {
            shell_streaming: self.shell_streaming && remote.shell_streaming,
            file_delta_sync: self.file_delta_sync && remote.file_delta_sync,
            screen_capture: self.screen_capture && remote.screen_capture,
            compression: self.compression && remote.compression,
            max_payload_size: self.max_payload_size.min(remote.max_payload_size),
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_lifecycle() {
        let mut phase = ConnectionPhase::Disconnected;

        phase.begin_connect().unwrap();
        assert_eq!(phase, ConnectionPhase::Connecting);

        phase.begin_handshake().unwrap();
        assert_eq!(phase, ConnectionPhase::Handshaking);

        phase.complete_handshake().unwrap();
        assert!(phase.is_connected());
        assert!(phase.connected_duration().is_some());

        phase.begin_disconnect().unwrap();
        assert_eq!(phase, ConnectionPhase::Disconnecting);

        phase.finish_disconnect().unwrap();
        assert!(phase.is_disconnected());
    }

    #[test]
    fn invalid_transition_connect_when_connected() {
        let mut phase = ConnectionPhase::Connected {
            since: Instant::now(),
        };
        assert!(phase.begin_connect().is_err());
    }

    #[test]
    fn invalid_transition_handshake_from_disconnected() {
        let mut phase = ConnectionPhase::Disconnected;
        assert!(phase.begin_handshake().is_err());
    }

    #[test]
    fn invalid_transition_complete_handshake_from_connecting() {
        let mut phase = ConnectionPhase::Connecting;
        assert!(phase.complete_handshake().is_err());
    }

    #[test]
    fn disconnect_from_handshaking() {
        let mut phase = ConnectionPhase::Handshaking;
        phase.begin_disconnect().unwrap();
        assert_eq!(phase, ConnectionPhase::Disconnecting);
        phase.finish_disconnect().unwrap();
        assert!(phase.is_disconnected());
    }

    #[test]
    fn force_disconnect_from_any_state() {
        let mut phase = ConnectionPhase::Connected {
            since: Instant::now(),
        };
        phase.force_disconnect();
        assert!(phase.is_disconnected());
    }

    #[test]
    fn finish_disconnect_from_connecting_on_failure() {
        let mut phase = ConnectionPhase::Connecting;
        phase.finish_disconnect().unwrap();
        assert!(phase.is_disconnected());
    }

    #[test]
    fn display_format() {
        assert_eq!(ConnectionPhase::Disconnected.to_string(), "Disconnected");
        assert_eq!(ConnectionPhase::Connecting.to_string(), "Connecting");
        assert_eq!(ConnectionPhase::Handshaking.to_string(), "Handshaking");
        assert_eq!(
            ConnectionPhase::Connected {
                since: Instant::now()
            }
            .to_string(),
            "Connected"
        );
        assert_eq!(ConnectionPhase::Disconnecting.to_string(), "Disconnecting");
    }

    #[test]
    fn capabilities_negotiate() {
        let local = PeerCapabilities {
            screen_capture: true,
            compression: false,
            ..Default::default()
        };
        let remote = PeerCapabilities {
            screen_capture: false,
            compression: true,
            max_payload_size: 1024,
            ..Default::default()
        };
        let negotiated = local.negotiate(&remote);
        assert!(!negotiated.screen_capture);
        assert!(!negotiated.compression);
        assert_eq!(negotiated.max_payload_size, 1024);
        assert!(negotiated.shell_streaming);
    }

    #[test]
    fn default_phase_is_disconnected() {
        let phase = ConnectionPhase::default();
        assert!(phase.is_disconnected());
    }
}
