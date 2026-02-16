//! Adaptive frame encoder with zstd compression.
//!
//! Encodes [`DeltaFrame`]s into compact [`EncodedFrame`]s suitable for
//! network transmission. Supports both full-frame and delta encoding:
//!
//! - **Full frame**: raw pixel data → zstd compress.
//! - **Delta frame**: per-block header + pixel data → zstd compress.
//!
//! Quality is adjusted dynamically via [`adjust_quality`](AdaptiveEncoder::adjust_quality)
//! based on measured bandwidth reported by the transport layer.

use std::time::Instant;

use crate::error::TixError;
use crate::rdp::delta::{DeltaFrame, Block};
use crate::rdp::types::RawScreenFrame;

// ── EncodedFrame ─────────────────────────────────────────────────

/// A compressed frame ready for network transmission.
#[derive(Debug, Clone)]
pub struct EncodedFrame {
    /// Sequential frame number.
    pub frame_number: u64,
    /// Capture timestamp.
    pub timestamp: Instant,
    /// Screen width in pixels.
    pub width: u32,
    /// Screen height in pixels.
    pub height: u32,
    /// Compressed payload (zstd).
    pub data: Vec<u8>,
    /// Whether this encodes the full screen or only changed blocks.
    pub is_full_frame: bool,
    /// Number of dirty blocks (informational).
    pub block_count: u32,
}

// ── AdaptiveEncoder ──────────────────────────────────────────────

/// Zstd-based frame encoder with adaptive quality control.
///
/// The encoder tracks a target bandwidth and adjusts its compression
/// level (and in the future resolution / quality scaling) to stay
/// within budget while maximising visual fidelity.
pub struct AdaptiveEncoder {
    /// Current zstd compression level (1 = fast / less compression,
    /// 19 = slow / max compression). For 100 MB/s we default to 1.
    compression_level: i32,
    /// Quality slider 0..100 (100 = lossless). Currently mapped
    /// directly to compression level; future work will add lossy
    /// down-scaling.
    quality: u8,
    /// Target bandwidth in bytes/second.
    target_bandwidth: u64,
    /// Most recently measured bandwidth from the transport layer.
    measured_bandwidth: u64,
    /// Number of frames encoded so far.
    frame_count: u64,
}

impl AdaptiveEncoder {
    /// Create an encoder targeting `target_bandwidth` bytes/second.
    ///
    /// For a 100 MB/s direct RJ-45 link pass `100 * 1024 * 1024`.
    pub fn new(target_bandwidth: u64) -> Self {
        Self {
            compression_level: 1, // favour speed
            quality: 90,
            target_bandwidth,
            measured_bandwidth: target_bandwidth,
            frame_count: 0,
        }
    }

    /// Encode a delta frame using pixel data from `source`.
    pub fn encode(
        &mut self,
        delta: &DeltaFrame,
        source: &RawScreenFrame,
    ) -> Result<EncodedFrame, TixError> {
        let raw = if delta.full_frame {
            self.encode_full_frame(source)?
        } else {
            self.encode_delta_blocks(&delta.changed_blocks, source)?
        };

        let compressed = zstd::encode_all(raw.as_slice(), self.compression_level)
            .map_err(|e| TixError::Other(format!("zstd encode failed: {e}")))?;

        self.frame_count += 1;

        Ok(EncodedFrame {
            frame_number: delta.frame_number,
            timestamp: delta.timestamp,
            width: delta.width,
            height: delta.height,
            data: compressed,
            is_full_frame: delta.full_frame,
            block_count: delta.changed_blocks.len() as u32,
        })
    }

    /// Adjust quality based on measured network throughput.
    ///
    /// Called periodically by the service loop after the transport
    /// layer reports actual bandwidth usage.
    pub fn adjust_quality(&mut self, measured_bandwidth: u64) {
        self.measured_bandwidth = measured_bandwidth;

        if measured_bandwidth > self.target_bandwidth {
            // Over budget — increase compression (slower but smaller).
            self.quality = self.quality.saturating_sub(5);
            self.compression_level = (self.compression_level + 1).min(9);
        } else if measured_bandwidth < self.target_bandwidth * 8 / 10 {
            // Under 80 % — decrease compression (faster, larger).
            self.quality = (self.quality + 5).min(100);
            self.compression_level = (self.compression_level - 1).max(1);
        }
    }

    /// Current quality slider value (0..100).
    pub fn quality(&self) -> u8 {
        self.quality
    }

    /// Number of frames encoded so far.
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    // ── Internal encoding helpers ────────────────────────────────

    /// Full frame: emit all rows packed tightly (no padding).
    fn encode_full_frame(&self, source: &RawScreenFrame) -> Result<Vec<u8>, TixError> {
        let bpp = source.format.bytes_per_pixel();
        let row_len = source.width as usize * bpp;
        let mut out = Vec::with_capacity(row_len * source.height as usize);

        for y in 0..source.height {
            let row_start = y as usize * source.stride as usize;
            out.extend_from_slice(&source.data[row_start..row_start + row_len]);
        }

        Ok(out)
    }

    /// Delta: emit a sequence of `[block_header | block_pixels]`.
    ///
    /// Block header (16 bytes, little-endian):
    /// ```text
    /// x:      u32
    /// y:      u32
    /// width:  u32
    /// height: u32
    /// ```
    fn encode_delta_blocks(
        &self,
        blocks: &[Block],
        source: &RawScreenFrame,
    ) -> Result<Vec<u8>, TixError> {
        let bpp = source.format.bytes_per_pixel();
        let mut out = Vec::new();

        // Leading u32: number of blocks.
        out.extend_from_slice(&(blocks.len() as u32).to_le_bytes());

        for block in blocks {
            // Block header.
            out.extend_from_slice(&block.x.to_le_bytes());
            out.extend_from_slice(&block.y.to_le_bytes());
            out.extend_from_slice(&block.width.to_le_bytes());
            out.extend_from_slice(&block.height.to_le_bytes());

            // Pixel data for this block.
            let start_x_bytes = block.x as usize * bpp;
            let row_bytes = block.width as usize * bpp;

            for row in 0..block.height {
                let y = (block.y + row) as usize;
                let offset = y * source.stride as usize + start_x_bytes;
                out.extend_from_slice(&source.data[offset..offset + row_bytes]);
            }
        }

        Ok(out)
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdp::delta::{Block, DeltaFrame};
    use crate::rdp::types::{PixelFormat, RawScreenFrame};
    use std::time::Instant;

    fn test_frame(w: u32, h: u32) -> RawScreenFrame {
        let stride = w * 4;
        RawScreenFrame {
            width: w,
            height: h,
            stride,
            format: PixelFormat::Bgra8,
            data: vec![0xAB; (stride * h) as usize],
            timestamp: Instant::now(),
        }
    }

    fn full_delta(w: u32, h: u32) -> DeltaFrame {
        DeltaFrame {
            frame_number: 1,
            timestamp: Instant::now(),
            width: w,
            height: h,
            changed_blocks: vec![Block {
                x: 0,
                y: 0,
                width: w,
                height: h,
            }],
            full_frame: true,
        }
    }

    fn partial_delta(w: u32, h: u32) -> DeltaFrame {
        DeltaFrame {
            frame_number: 2,
            timestamp: Instant::now(),
            width: w,
            height: h,
            changed_blocks: vec![Block {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            }],
            full_frame: false,
        }
    }

    #[test]
    fn encode_full_frame_compresses() {
        let mut enc = AdaptiveEncoder::new(100 * 1024 * 1024);
        let frame = test_frame(128, 128);
        let delta = full_delta(128, 128);
        let encoded = enc.encode(&delta, &frame).unwrap();

        assert!(encoded.is_full_frame);
        // Compressed should be smaller (repetitive data).
        assert!(encoded.data.len() < frame.data.len());
        assert_eq!(enc.frame_count(), 1);
    }

    #[test]
    fn encode_delta_frame() {
        let mut enc = AdaptiveEncoder::new(100 * 1024 * 1024);
        let frame = test_frame(128, 128);
        let delta = partial_delta(128, 128);
        let encoded = enc.encode(&delta, &frame).unwrap();

        assert!(!encoded.is_full_frame);
        assert_eq!(encoded.block_count, 1);
    }

    #[test]
    fn quality_decreases_when_over_budget() {
        let mut enc = AdaptiveEncoder::new(1_000_000);
        let initial = enc.quality();
        enc.adjust_quality(2_000_000); // 2× over budget
        assert!(enc.quality() < initial);
    }

    #[test]
    fn quality_increases_when_under_budget() {
        let mut enc = AdaptiveEncoder::new(10_000_000);
        enc.quality = 50; // manually set lower
        enc.adjust_quality(1_000_000); // 10 % of budget
        assert!(enc.quality() > 50);
    }
}
