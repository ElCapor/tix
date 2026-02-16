//! Frame decoder / decompressor.
//!
//! Takes [`EncodedFrame`]s received from the network and reconstructs
//! pixel data that can be rendered on the master display.

use crate::error::TixError;
use crate::rdp::encoder::EncodedFrame;

// ── DecodedFrame ─────────────────────────────────────────────────

/// A decompressed frame ready for rendering.
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    /// Screen width in pixels.
    pub width: u32,
    /// Screen height in pixels.
    pub height: u32,
    /// Whether this represents the full screen or a partial update.
    pub is_full_frame: bool,
    /// Decompressed data.
    ///
    /// - **Full frame**: tightly-packed BGRA rows (`width * 4 * height` bytes).
    /// - **Delta frame**: block-encoded payload (see [`AdaptiveEncoder`]).
    pub data: Vec<u8>,
    /// Number of dirty blocks (0 for full frames).
    pub block_count: u32,
}

// ── DecodedBlock ─────────────────────────────────────────────────

/// A single dirty block extracted from a delta frame.
#[derive(Debug, Clone)]
pub struct DecodedBlock {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Pixel data: `width * height * bpp` bytes (tightly packed rows).
    pub data: Vec<u8>,
}

// ── FrameDecoder ─────────────────────────────────────────────────

/// Stateless decoder that decompresses zstd-encoded frames.
pub struct FrameDecoder {
    /// Persistent frame buffer (full screen, updated incrementally).
    frame_buffer: Vec<u8>,
    /// Dimensions of the current frame buffer.
    buf_width: u32,
    buf_height: u32,
}

impl FrameDecoder {
    /// Create a new decoder.
    pub fn new() -> Self {
        Self {
            frame_buffer: Vec::new(),
            buf_width: 0,
            buf_height: 0,
        }
    }

    /// Decompress an encoded frame and return the decoded payload.
    pub fn decode(&mut self, encoded: &EncodedFrame) -> Result<DecodedFrame, TixError> {
        let decompressed = zstd::decode_all(encoded.data.as_slice())
            .map_err(|e| TixError::Other(format!("zstd decode failed: {e}")))?;

        Ok(DecodedFrame {
            width: encoded.width,
            height: encoded.height,
            is_full_frame: encoded.is_full_frame,
            data: decompressed,
            block_count: encoded.block_count,
        })
    }

    /// Apply a decoded frame to the internal frame buffer and return
    /// a reference to the complete, up-to-date screen image.
    ///
    /// For full frames, the buffer is replaced entirely.
    /// For delta frames, only the dirty blocks are patched in.
    pub fn apply(&mut self, frame: &DecodedFrame, bpp: usize) -> Result<&[u8], TixError> {
        let fb_size = frame.width as usize * frame.height as usize * bpp;

        // Resize / reinitialise if dimensions changed.
        if frame.width != self.buf_width || frame.height != self.buf_height {
            self.frame_buffer = vec![0u8; fb_size];
            self.buf_width = frame.width;
            self.buf_height = frame.height;
        }

        if frame.is_full_frame {
            self.apply_full_frame(&frame.data, bpp)?;
        } else {
            self.apply_delta_frame(&frame.data, bpp)?;
        }

        Ok(&self.frame_buffer)
    }

    /// Current frame buffer contents (may be empty before first decode).
    pub fn frame_buffer(&self) -> &[u8] {
        &self.frame_buffer
    }

    // ── Internal ─────────────────────────────────────────────────

    fn apply_full_frame(&mut self, data: &[u8], bpp: usize) -> Result<(), TixError> {
        let expected = self.buf_width as usize * self.buf_height as usize * bpp;
        if data.len() < expected {
            return Err(TixError::Other(format!(
                "full frame too short: {} < {}",
                data.len(),
                expected
            )));
        }
        self.frame_buffer[..expected].copy_from_slice(&data[..expected]);
        Ok(())
    }

    fn apply_delta_frame(&mut self, data: &[u8], bpp: usize) -> Result<(), TixError> {
        if data.len() < 4 {
            return Err(TixError::Other("delta frame too short for block count".into()));
        }

        let block_count = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
        let mut offset = 4;
        let row_stride = self.buf_width as usize * bpp;

        for _ in 0..block_count {
            if offset + 16 > data.len() {
                return Err(TixError::Other("delta frame truncated (block header)".into()));
            }

            let x = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
            let y = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap());
            let w = u32::from_le_bytes(data[offset + 8..offset + 12].try_into().unwrap());
            let h = u32::from_le_bytes(data[offset + 12..offset + 16].try_into().unwrap());
            offset += 16;

            let block_row_bytes = w as usize * bpp;

            for row in 0..h as usize {
                let src_start = offset;
                let src_end = src_start + block_row_bytes;
                if src_end > data.len() {
                    return Err(TixError::Other("delta frame truncated (block data)".into()));
                }

                let dst_y = (y as usize + row) * row_stride;
                let dst_x = x as usize * bpp;
                let dst_start = dst_y + dst_x;

                self.frame_buffer[dst_start..dst_start + block_row_bytes]
                    .copy_from_slice(&data[src_start..src_end]);

                offset += block_row_bytes;
            }
        }

        Ok(())
    }

    /// Parse a delta payload into individual [`DecodedBlock`]s.
    ///
    /// Useful when the renderer wants to blit blocks individually
    /// rather than patching into a frame buffer.
    pub fn extract_blocks(data: &[u8], bpp: usize) -> Result<Vec<DecodedBlock>, TixError> {
        if data.len() < 4 {
            return Err(TixError::Other("delta too short".into()));
        }

        let count = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
        let mut offset = 4;
        let mut blocks = Vec::with_capacity(count);

        for _ in 0..count {
            if offset + 16 > data.len() {
                return Err(TixError::Other("truncated block header".into()));
            }

            let x = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
            let y = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap());
            let w = u32::from_le_bytes(data[offset + 8..offset + 12].try_into().unwrap());
            let h = u32::from_le_bytes(data[offset + 12..offset + 16].try_into().unwrap());
            offset += 16;

            let block_bytes = w as usize * h as usize * bpp;
            if offset + block_bytes > data.len() {
                return Err(TixError::Other("truncated block data".into()));
            }

            blocks.push(DecodedBlock {
                x,
                y,
                width: w,
                height: h,
                data: data[offset..offset + block_bytes].to_vec(),
            });
            offset += block_bytes;
        }

        Ok(blocks)
    }
}

impl Default for FrameDecoder {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdp::delta::{Block, DeltaFrame};
    use crate::rdp::encoder::AdaptiveEncoder;
    use crate::rdp::types::{PixelFormat, RawScreenFrame};
    use std::time::Instant;

    fn test_frame(w: u32, h: u32, fill: u8) -> RawScreenFrame {
        let stride = w * 4;
        RawScreenFrame {
            width: w,
            height: h,
            stride,
            format: PixelFormat::Bgra8,
            data: vec![fill; (stride * h) as usize],
            timestamp: Instant::now(),
        }
    }

    #[test]
    fn full_frame_roundtrip() {
        let source = test_frame(64, 64, 0xCD);
        let delta = DeltaFrame {
            frame_number: 0,
            timestamp: Instant::now(),
            width: 64,
            height: 64,
            changed_blocks: vec![Block { x: 0, y: 0, width: 64, height: 64 }],
            full_frame: true,
        };

        let mut enc = AdaptiveEncoder::new(100_000_000);
        let encoded = enc.encode(&delta, &source).unwrap();

        let mut dec = FrameDecoder::new();
        let decoded = dec.decode(&encoded).unwrap();
        assert!(decoded.is_full_frame);

        let bpp = 4;
        let buf = dec.apply(&decoded, bpp).unwrap();
        // Every pixel should be 0xCD.
        assert_eq!(buf.len(), 64 * 64 * 4);
        assert!(buf.iter().all(|&b| b == 0xCD));
    }

    #[test]
    fn delta_frame_roundtrip() {
        let source = test_frame(128, 128, 0x42);
        let delta = DeltaFrame {
            frame_number: 1,
            timestamp: Instant::now(),
            width: 128,
            height: 128,
            changed_blocks: vec![Block { x: 0, y: 0, width: 32, height: 32 }],
            full_frame: false,
        };

        let mut enc = AdaptiveEncoder::new(100_000_000);
        let encoded = enc.encode(&delta, &source).unwrap();

        let mut dec = FrameDecoder::new();
        let decoded = dec.decode(&encoded).unwrap();
        assert!(!decoded.is_full_frame);

        let bpp = 4;
        let buf = dec.apply(&decoded, bpp).unwrap();
        // First 32×32 block should be 0x42, rest should be 0.
        let row_stride = 128 * 4;
        for y in 0..32 {
            for x in 0..32 {
                let off = y * row_stride + x * 4;
                assert_eq!(buf[off], 0x42, "pixel ({x},{y}) mismatch");
            }
        }
    }

    #[test]
    fn extract_blocks_works() {
        let source = test_frame(128, 128, 0xFF);
        let delta = DeltaFrame {
            frame_number: 0,
            timestamp: Instant::now(),
            width: 128,
            height: 128,
            changed_blocks: vec![
                Block { x: 0, y: 0, width: 16, height: 16 },
                Block { x: 64, y: 64, width: 16, height: 16 },
            ],
            full_frame: false,
        };

        let mut enc = AdaptiveEncoder::new(100_000_000);
        let encoded = enc.encode(&delta, &source).unwrap();

        let mut dec = FrameDecoder::new();
        let decoded = dec.decode(&encoded).unwrap();

        let blocks = FrameDecoder::extract_blocks(&decoded.data, 4).unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].width, 16);
        assert_eq!(blocks[1].x, 64);
    }
}
