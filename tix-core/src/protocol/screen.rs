//! Remote desktop protocol (TixRP) — screen capture, input injection.
//!
//! # Wire Protocol
//!
//! ## Screen Start
//! ```text
//! Master ──[ScreenStart]─────────────────────► Slave
//!   Payload: ScreenStartRequest (bincode)
//!
//! Slave  ──[ScreenStart]─────────────────────► Master   (ack)
//!   Payload: ScreenConfig (bincode)
//! ```
//!
//! ## Screen Frames (continuous)
//! ```text
//! Slave  ──[ScreenFrame + STREAMING]─────────► Master   (repeated)
//!   Payload: ScreenFrame (bincode)
//! ```
//!
//! ## Screen Stop
//! ```text
//! Master ──[ScreenStop]──────────────────────► Slave
//!   Payload: empty
//! ```
//!
//! ## Input Injection
//! ```text
//! Master ──[InputMouse]──────────────────────► Slave
//!   Payload: MouseEvent (bincode)
//!
//! Master ──[InputKeyboard]───────────────────► Slave
//!   Payload: KeyEvent (bincode)
//! ```

use serde::{Deserialize, Serialize};

use crate::error::TixError;
use crate::flags::ProtocolFlags;
use crate::message::Command;
use crate::packet::Packet;

// ── Screen Start ──────────────────────────────────────────────────

/// Request to start screen capture on the slave.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScreenStartRequest {
    /// Desired quality (0-100, where 100 = lossless).
    pub quality: u8,

    /// Target frames per second (1-60).
    pub fps: u8,

    /// Optional capture region (if None, capture full screen).
    pub region: Option<CaptureRegion>,

    /// Preferred image format for frames.
    pub format: ImageFormat,

    /// Whether to include cursor position in frames.
    pub include_cursor: bool,

    /// Monitor index to capture (0 = primary).
    pub monitor: u8,
}

impl Default for ScreenStartRequest {
    fn default() -> Self {
        Self {
            quality: 75,
            fps: 30,
            region: None,
            format: ImageFormat::Jpeg,
            include_cursor: true,
            monitor: 0,
        }
    }
}

impl ScreenStartRequest {
    /// Create a new screen start request with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set quality level.
    pub fn with_quality(mut self, quality: u8) -> Self {
        self.quality = quality.min(100);
        self
    }

    /// Set target FPS.
    pub fn with_fps(mut self, fps: u8) -> Self {
        self.fps = fps.clamp(1, 60);
        self
    }

    /// Set capture region.
    pub fn with_region(mut self, region: CaptureRegion) -> Self {
        self.region = Some(region);
        self
    }

    /// Set image format.
    pub fn with_format(mut self, format: ImageFormat) -> Self {
        self.format = format;
        self
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
        Packet::new_command(request_id, Command::ScreenStart, payload)
    }
}

// ── Screen Config ─────────────────────────────────────────────────

/// Acknowledged screen configuration sent back to master after ScreenStart.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScreenConfig {
    /// Actual screen width in pixels.
    pub width: u32,

    /// Actual screen height in pixels.
    pub height: u32,

    /// Negotiated quality.
    pub quality: u8,

    /// Negotiated FPS.
    pub fps: u8,

    /// Image format that will be used.
    pub format: ImageFormat,

    /// Monitor name/description.
    pub monitor_name: String,
}

impl ScreenConfig {
    /// Serialize to bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>, TixError> {
        bincode::serialize(self).map_err(|e| TixError::Encoding(e.to_string()))
    }

    /// Deserialize from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TixError> {
        bincode::deserialize(bytes).map_err(|e| TixError::Encoding(e.to_string()))
    }

    /// Build a response `Packet`.
    pub fn into_packet(self, request_id: u64) -> Result<Packet, TixError> {
        let payload = self.to_bytes()?;
        Packet::new_response(request_id, Command::ScreenStart, payload)
    }
}

// ── Screen Stop ───────────────────────────────────────────────────

/// Request to stop screen capture. Payload is empty, but we define a
/// type for consistency and future extensibility.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ScreenStopRequest;

impl ScreenStopRequest {
    /// Build a command `Packet`.
    pub fn into_packet(self, request_id: u64) -> Result<Packet, TixError> {
        Packet::new_command(request_id, Command::ScreenStop, Vec::new())
    }
}

// ── Screen Frame ──────────────────────────────────────────────────

/// A single captured screen frame from slave to master.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScreenFrame {
    /// Sequential frame number (0-based).
    pub frame_number: u64,

    /// Capture timestamp in microseconds since capture start.
    pub timestamp_us: u64,

    /// Frame width in pixels.
    pub width: u32,

    /// Frame height in pixels.
    pub height: u32,

    /// Image encoding format.
    pub format: ImageFormat,

    /// Encoded image data.
    pub data: Vec<u8>,

    /// Optional cursor information.
    pub cursor: Option<CursorInfo>,

    /// Whether this is a full frame or a delta from the previous.
    pub is_delta: bool,
}

impl ScreenFrame {
    /// Serialize to bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>, TixError> {
        bincode::serialize(self).map_err(|e| TixError::Encoding(e.to_string()))
    }

    /// Deserialize from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TixError> {
        bincode::deserialize(bytes).map_err(|e| TixError::Encoding(e.to_string()))
    }

    /// Build a streaming response `Packet`.
    pub fn into_packet(self, request_id: u64) -> Result<Packet, TixError> {
        let payload = self.to_bytes()?;
        Packet::new_response_with_flags(
            request_id,
            Command::ScreenFrame,
            payload,
            ProtocolFlags::STREAMING,
        )
    }
}

// ── Capture Region ────────────────────────────────────────────────

/// A rectangular region of the screen to capture.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct CaptureRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl CaptureRegion {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Full-screen region placeholder.
    pub fn full_screen(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }
}

// ── Image Format ──────────────────────────────────────────────────

/// Supported image encoding formats for screen frames.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub enum ImageFormat {
    /// JPEG — lossy, good compression, fast.
    #[default]
    Jpeg,
    /// PNG — lossless, slower.
    Png,
    /// Raw BGRA pixels — no compression, lowest latency.
    RawBgra,
    /// Raw RGB pixels.
    RawRgb,
}

impl std::fmt::Display for ImageFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageFormat::Jpeg => write!(f, "jpeg"),
            ImageFormat::Png => write!(f, "png"),
            ImageFormat::RawBgra => write!(f, "raw_bgra"),
            ImageFormat::RawRgb => write!(f, "raw_rgb"),
        }
    }
}

// ── Cursor Info ───────────────────────────────────────────────────

/// Cursor position and visibility information.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct CursorInfo {
    /// Cursor X position in screen coordinates.
    pub x: i32,
    /// Cursor Y position in screen coordinates.
    pub y: i32,
    /// Whether the cursor is visible.
    pub visible: bool,
}

impl CursorInfo {
    pub fn new(x: i32, y: i32, visible: bool) -> Self {
        Self { x, y, visible }
    }
}

// ── Mouse Input ───────────────────────────────────────────────────

/// Mouse input event injected from master to slave.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct MouseEvent {
    /// X position in screen coordinates.
    pub x: i32,
    /// Y position in screen coordinates.
    pub y: i32,
    /// Type of mouse event.
    pub kind: MouseEventKind,
    /// Which button (if applicable).
    pub button: MouseButton,
    /// Scroll delta (for scroll events).
    pub scroll_delta: i16,
}

impl MouseEvent {
    /// Create a mouse move event.
    pub fn move_to(x: i32, y: i32) -> Self {
        Self {
            x,
            y,
            kind: MouseEventKind::Move,
            button: MouseButton::None,
            scroll_delta: 0,
        }
    }

    /// Create a mouse button press.
    pub fn press(x: i32, y: i32, button: MouseButton) -> Self {
        Self {
            x,
            y,
            kind: MouseEventKind::Press,
            button,
            scroll_delta: 0,
        }
    }

    /// Create a mouse button release.
    pub fn release(x: i32, y: i32, button: MouseButton) -> Self {
        Self {
            x,
            y,
            kind: MouseEventKind::Release,
            button,
            scroll_delta: 0,
        }
    }

    /// Create a scroll event.
    pub fn scroll(x: i32, y: i32, delta: i16) -> Self {
        Self {
            x,
            y,
            kind: MouseEventKind::Scroll,
            button: MouseButton::None,
            scroll_delta: delta,
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
        Packet::new_command(request_id, Command::InputMouse, payload)
    }
}

/// Kind of mouse event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MouseEventKind {
    Move,
    Press,
    Release,
    Scroll,
    DoubleClick,
}

/// Mouse button identifier.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MouseButton {
    None,
    Left,
    Right,
    Middle,
    X1,
    X2,
}

// ── Keyboard Input ────────────────────────────────────────────────

/// Keyboard input event injected from master to slave.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct KeyEvent {
    /// Virtual key code (platform-specific).
    pub virtual_key: u16,

    /// Hardware scan code.
    pub scan_code: u16,

    /// Whether this is a press or release.
    pub action: KeyAction,

    /// Modifier flags (Shift, Ctrl, Alt, etc.).
    pub modifiers: u8,
}

/// Key action type.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum KeyAction {
    Press,
    Release,
}

/// Modifier key flags.
pub mod key_modifiers {
    pub const NONE: u8 = 0x00;
    pub const SHIFT: u8 = 0x01;
    pub const CTRL: u8 = 0x02;
    pub const ALT: u8 = 0x04;
    pub const META: u8 = 0x08; // Windows key / Super
}

impl KeyEvent {
    /// Create a key press event.
    pub fn press(virtual_key: u16, scan_code: u16, modifiers: u8) -> Self {
        Self {
            virtual_key,
            scan_code,
            action: KeyAction::Press,
            modifiers,
        }
    }

    /// Create a key release event.
    pub fn release(virtual_key: u16, scan_code: u16, modifiers: u8) -> Self {
        Self {
            virtual_key,
            scan_code,
            action: KeyAction::Release,
            modifiers,
        }
    }

    /// Check if a modifier is set.
    pub fn has_modifier(&self, modifier: u8) -> bool {
        self.modifiers & modifier != 0
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
        Packet::new_command(request_id, Command::InputKeyboard, payload)
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_start_request_roundtrip() {
        let req = ScreenStartRequest::new()
            .with_quality(90)
            .with_fps(60)
            .with_format(ImageFormat::Png);

        let bytes = req.to_bytes().unwrap();
        let decoded = ScreenStartRequest::from_bytes(&bytes).unwrap();
        assert_eq!(req, decoded);
        assert_eq!(decoded.quality, 90);
        assert_eq!(decoded.fps, 60);
        assert_eq!(decoded.format, ImageFormat::Png);
    }

    #[test]
    fn screen_start_with_region() {
        let req = ScreenStartRequest::new().with_region(CaptureRegion::new(100, 200, 800, 600));

        let bytes = req.to_bytes().unwrap();
        let decoded = ScreenStartRequest::from_bytes(&bytes).unwrap();
        let region = decoded.region.unwrap();
        assert_eq!(region.x, 100);
        assert_eq!(region.y, 200);
        assert_eq!(region.width, 800);
        assert_eq!(region.height, 600);
    }

    #[test]
    fn screen_config_roundtrip() {
        let config = ScreenConfig {
            width: 1920,
            height: 1080,
            quality: 75,
            fps: 30,
            format: ImageFormat::Jpeg,
            monitor_name: "Primary".to_string(),
        };

        let bytes = config.to_bytes().unwrap();
        let decoded = ScreenConfig::from_bytes(&bytes).unwrap();
        assert_eq!(config, decoded);
    }

    #[test]
    fn screen_frame_roundtrip() {
        let frame = ScreenFrame {
            frame_number: 42,
            timestamp_us: 1_000_000,
            width: 1920,
            height: 1080,
            format: ImageFormat::Jpeg,
            data: vec![0xFF; 100],
            cursor: Some(CursorInfo::new(960, 540, true)),
            is_delta: false,
        };

        let bytes = frame.to_bytes().unwrap();
        let decoded = ScreenFrame::from_bytes(&bytes).unwrap();
        assert_eq!(frame, decoded);
        assert_eq!(decoded.cursor.unwrap().x, 960);
    }

    #[test]
    fn mouse_event_roundtrip() {
        let events = vec![
            MouseEvent::move_to(100, 200),
            MouseEvent::press(100, 200, MouseButton::Left),
            MouseEvent::release(100, 200, MouseButton::Left),
            MouseEvent::scroll(100, 200, -120),
        ];

        for event in events {
            let bytes = event.to_bytes().unwrap();
            let decoded = MouseEvent::from_bytes(&bytes).unwrap();
            assert_eq!(event, decoded);
        }
    }

    #[test]
    fn key_event_roundtrip() {
        let event = KeyEvent::press(0x41, 0x1E, key_modifiers::SHIFT | key_modifiers::CTRL);
        let bytes = event.to_bytes().unwrap();
        let decoded = KeyEvent::from_bytes(&bytes).unwrap();

        assert_eq!(event, decoded);
        assert!(decoded.has_modifier(key_modifiers::SHIFT));
        assert!(decoded.has_modifier(key_modifiers::CTRL));
        assert!(!decoded.has_modifier(key_modifiers::ALT));
    }

    #[test]
    fn key_event_release() {
        let event = KeyEvent::release(0x41, 0x1E, key_modifiers::NONE);
        assert_eq!(event.action, KeyAction::Release);
        assert!(!event.has_modifier(key_modifiers::SHIFT));
    }

    #[test]
    fn screen_start_into_packet() {
        let req = ScreenStartRequest::new().with_quality(50);
        let packet = req.into_packet(7).unwrap();

        assert_eq!(packet.command().unwrap(), Command::ScreenStart);
        assert_eq!(packet.request_id(), 7);

        let decoded = ScreenStartRequest::from_bytes(packet.payload()).unwrap();
        assert_eq!(decoded.quality, 50);
    }

    #[test]
    fn mouse_event_into_packet() {
        let event = MouseEvent::press(500, 300, MouseButton::Left);
        let packet = event.into_packet(10).unwrap();
        assert_eq!(packet.command().unwrap(), Command::InputMouse);
    }

    #[test]
    fn key_event_into_packet() {
        let event = KeyEvent::press(0x0D, 0x1C, key_modifiers::NONE); // Enter key
        let packet = event.into_packet(11).unwrap();
        assert_eq!(packet.command().unwrap(), Command::InputKeyboard);
    }

    #[test]
    fn fps_clamped() {
        let req = ScreenStartRequest::new().with_fps(200);
        assert_eq!(req.fps, 60);

        let req = ScreenStartRequest::new().with_fps(0);
        assert_eq!(req.fps, 1);
    }

    #[test]
    fn quality_clamped() {
        let req = ScreenStartRequest::new().with_quality(255);
        assert_eq!(req.quality, 100);
    }

    #[test]
    fn image_format_display() {
        assert_eq!(ImageFormat::Jpeg.to_string(), "jpeg");
        assert_eq!(ImageFormat::Png.to_string(), "png");
        assert_eq!(ImageFormat::RawBgra.to_string(), "raw_bgra");
    }
}
