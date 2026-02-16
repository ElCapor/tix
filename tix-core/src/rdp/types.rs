//! Shared types for the RDP capture/display pipeline.
//!
//! These are **internal** frame representations used between pipeline stages.
//! They are distinct from [`crate::protocol::screen::ScreenFrame`], which is
//! the serialisable *wire* type carried inside TIX packets.

use std::time::Instant;

// ── PixelFormat ──────────────────────────────────────────────────

/// Pixel layout for raw captured frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// 4 bytes per pixel: Blue, Green, Red, Alpha (DXGI default).
    Bgra8,
    /// 4 bytes per pixel: Red, Green, Blue, Alpha.
    Rgba8,
    /// 3 bytes per pixel: Red, Green, Blue.
    Rgb8,
}

impl PixelFormat {
    /// Bytes consumed by a single pixel in this format.
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            PixelFormat::Bgra8 | PixelFormat::Rgba8 => 4,
            PixelFormat::Rgb8 => 3,
        }
    }
}

// ── RawScreenFrame ───────────────────────────────────────────────

/// A raw, uncompressed screen capture obtained from the OS.
///
/// The `data` buffer holds `height` rows of `stride` bytes each.
/// `stride` may be larger than `width * bytes_per_pixel` due to
/// GPU row-alignment requirements (e.g. DXGI may pad rows to 256-byte
/// boundaries).
#[derive(Debug, Clone)]
pub struct RawScreenFrame {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Row pitch in **bytes** (may exceed `width * bpp`).
    pub stride: u32,
    /// Pixel layout.
    pub format: PixelFormat,
    /// Raw pixel data — `stride * height` bytes.
    pub data: Vec<u8>,
    /// Monotonic capture timestamp.
    pub timestamp: Instant,
}

impl RawScreenFrame {
    /// Total byte size the raw bitmap occupies.
    pub fn byte_len(&self) -> usize {
        self.stride as usize * self.height as usize
    }

    /// Returns a row slice (including possible padding bytes).
    pub fn row(&self, y: u32) -> &[u8] {
        let start = y as usize * self.stride as usize;
        let end = start + self.stride as usize;
        &self.data[start..end]
    }

    /// Returns the pixel bytes at `(x, y)`.
    ///
    /// # Panics
    ///
    /// Panics if `(x, y)` is out of bounds.
    pub fn pixel(&self, x: u32, y: u32) -> &[u8] {
        let bpp = self.format.bytes_per_pixel();
        let offset = y as usize * self.stride as usize + x as usize * bpp;
        &self.data[offset..offset + bpp]
    }
}
