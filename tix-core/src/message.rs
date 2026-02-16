//! Protocol message types and command definitions.
//!
//! Uses proper enums with `TryFrom` — no panics on unknown values.

use crate::error::TixError;
use std::fmt;

// ── MessageType ──────────────────────────────────────────────────

/// Distinguishes commands (master → slave) from responses (slave → master).
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageType {
    /// A request sent from master to slave.
    Command = 0x1,
    /// A reply sent from slave to master.
    Response = 0x2,
}

impl TryFrom<u32> for MessageType {
    type Error = TixError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0x1 => Ok(MessageType::Command),
            0x2 => Ok(MessageType::Response),
            _ => Err(TixError::UnknownVariant {
                type_name: "MessageType",
                value: value as u64,
            }),
        }
    }
}

impl fmt::Display for MessageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MessageType::Command => write!(f, "Command"),
            MessageType::Response => write!(f, "Response"),
        }
    }
}

// ── Command ──────────────────────────────────────────────────────

/// All commands understood by the TIX protocol.
///
/// Organized by category:
/// - `0x0001..0x00FF` — Protocol-level (handshake, heartbeat)
/// - `0x0100..0x01FF` — Shell commands
/// - `0x0200..0x02FF` — File commands
/// - `0x0300..0x03FF` — System commands
/// - `0x0400..0x04FF` — Screen capture / remote desktop (TixRP)
/// - `0x0500..0x05FF` — Update commands
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    // ── Protocol (0x00xx) ────────────────────────────────────────
    /// Keep-alive ping.
    Ping = 0x0001,
    /// Connection handshake.
    Hello = 0x0002,
    /// Graceful disconnect.
    Goodbye = 0x0003,
    /// Periodic heartbeat.
    Heartbeat = 0x0004,

    // ── Shell (0x01xx) ───────────────────────────────────────────
    /// Execute a shell command.
    ShellExecute = 0x0101,
    /// Cancel a running shell command.
    ShellCancel = 0x0102,
    /// Resize the PTY.
    ShellResize = 0x0103,

    // ── File (0x02xx) ────────────────────────────────────────────
    /// List directory contents.
    ListDir = 0x0201,
    /// Read / download a file.
    FileRead = 0x0202,
    /// Write / upload a file.
    FileWrite = 0x0203,
    /// List available drives (Windows).
    ListDrives = 0x0204,
    /// Copy files on the remote.
    Copy = 0x0205,
    /// Upload file (local → remote).
    Upload = 0x0206,
    /// Download file (remote → local).
    Download = 0x0207,

    // ── System (0x03xx) ──────────────────────────────────────────
    /// Query system information (OS, CPU, RAM, etc.).
    SystemInfo = 0x0301,
    /// Perform a system action (shutdown, reboot, sleep).
    SystemAction = 0x0302,
    /// List running processes.
    ProcessList = 0x0303,

    // ── Screen / Remote Desktop (0x04xx) ─────────────────────────
    /// Start screen capture session.
    ScreenStart = 0x0401,
    /// Stop screen capture session.
    ScreenStop = 0x0402,
    /// A screen frame (data from slave → master).
    ScreenFrame = 0x0403,
    /// Mouse input event (master → slave).
    InputMouse = 0x0404,
    /// Keyboard input event (master → slave).
    InputKeyboard = 0x0405,

    // ── Update (0x05xx) ──────────────────────────────────────────
    /// Check for updates.
    UpdateCheck = 0x0501,
    /// Push an update payload.
    UpdatePush = 0x0502,
    /// Apply the staged update.
    UpdateApply = 0x0503,
}

impl TryFrom<u64> for Command {
    type Error = TixError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0x0001 => Ok(Command::Ping),
            0x0002 => Ok(Command::Hello),
            0x0003 => Ok(Command::Goodbye),
            0x0004 => Ok(Command::Heartbeat),

            0x0101 => Ok(Command::ShellExecute),
            0x0102 => Ok(Command::ShellCancel),
            0x0103 => Ok(Command::ShellResize),

            0x0201 => Ok(Command::ListDir),
            0x0202 => Ok(Command::FileRead),
            0x0203 => Ok(Command::FileWrite),
            0x0204 => Ok(Command::ListDrives),
            0x0205 => Ok(Command::Copy),
            0x0206 => Ok(Command::Upload),
            0x0207 => Ok(Command::Download),

            0x0301 => Ok(Command::SystemInfo),
            0x0302 => Ok(Command::SystemAction),
            0x0303 => Ok(Command::ProcessList),

            0x0401 => Ok(Command::ScreenStart),
            0x0402 => Ok(Command::ScreenStop),
            0x0403 => Ok(Command::ScreenFrame),
            0x0404 => Ok(Command::InputMouse),
            0x0405 => Ok(Command::InputKeyboard),

            0x0501 => Ok(Command::UpdateCheck),
            0x0502 => Ok(Command::UpdatePush),
            0x0503 => Ok(Command::UpdateApply),

            _ => Err(TixError::UnknownVariant {
                type_name: "Command",
                value,
            }),
        }
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl Command {
    /// Returns `true` if this command expects a response from the peer.
    pub fn expects_response(&self) -> bool {
        !matches!(self, Command::Heartbeat | Command::Goodbye)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_type_roundtrip() {
        assert_eq!(
            MessageType::try_from(MessageType::Command as u32).unwrap(),
            MessageType::Command
        );
        assert_eq!(
            MessageType::try_from(MessageType::Response as u32).unwrap(),
            MessageType::Response
        );
    }

    #[test]
    fn message_type_invalid() {
        assert!(MessageType::try_from(0xFF).is_err());
    }

    #[test]
    fn command_roundtrip() {
        let cmds = [
            Command::Ping,
            Command::Hello,
            Command::Goodbye,
            Command::Heartbeat,
            Command::ShellExecute,
            Command::ShellCancel,
            Command::ShellResize,
            Command::ListDir,
            Command::FileRead,
            Command::FileWrite,
            Command::ListDrives,
            Command::Copy,
            Command::Upload,
            Command::Download,
            Command::SystemInfo,
            Command::SystemAction,
            Command::ProcessList,
            Command::ScreenStart,
            Command::ScreenStop,
            Command::ScreenFrame,
            Command::InputMouse,
            Command::InputKeyboard,
            Command::UpdateCheck,
            Command::UpdatePush,
            Command::UpdateApply,
        ];
        for cmd in cmds {
            assert_eq!(Command::try_from(cmd as u64).unwrap(), cmd);
        }
    }

    #[test]
    fn command_invalid() {
        assert!(Command::try_from(0xDEAD).is_err());
    }

    #[test]
    fn heartbeat_does_not_expect_response() {
        assert!(!Command::Heartbeat.expects_response());
        assert!(Command::Ping.expects_response());
    }
}
