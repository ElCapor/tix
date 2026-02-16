//! Block-level delta detection between consecutive frames.
//!
//! Divides the screen into `block_size × block_size` tiles and compares
//! each tile byte-for-byte against the previous frame. Only tiles that
//! differ are included in the [`DeltaFrame`] output, dramatically
//! reducing bandwidth when the screen is mostly static.

use std::cmp;
use std::time::Instant;

use crate::rdp::types::RawScreenFrame;

// ── Block ────────────────────────────────────────────────────────

/// A rectangular region that has changed since the previous frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    /// Left edge in pixels.
    pub x: u32,
    /// Top edge in pixels.
    pub y: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

// ── DeltaFrame ───────────────────────────────────────────────────

/// Result of the delta detection pass.
///
/// If `full_frame` is `true` the entire screen should be encoded
/// (typically the first frame or after a resolution change).
#[derive(Debug, Clone)]
pub struct DeltaFrame {
    /// Sequential frame counter (set by the caller).
    pub frame_number: u64,
    /// Monotonic timestamp of the captured source frame.
    pub timestamp: Instant,
    /// Screen width in pixels.
    pub width: u32,
    /// Screen height in pixels.
    pub height: u32,
    /// Blocks that differ from the previous frame.
    pub changed_blocks: Vec<Block>,
    /// When `true`, the encoder should transmit the full frame.
    pub full_frame: bool,
}

impl DeltaFrame {
    /// Fraction of the screen area that changed (0.0 – 1.0).
    pub fn change_ratio(&self) -> f64 {
        if self.full_frame {
            return 1.0;
        }
        let total = self.width as f64 * self.height as f64;
        if total == 0.0 {
            return 0.0;
        }
        let changed: f64 = self
            .changed_blocks
            .iter()
            .map(|b| b.width as f64 * b.height as f64)
            .sum();
        (changed / total).min(1.0)
    }
}

// ── DeltaDetector ────────────────────────────────────────────────

/// Stateful detector that remembers the previous frame and emits
/// per-block change information.
///
/// # Block size
///
/// A block size of **64** offers a good trade-off: large enough to
/// amortise the per-block overhead, small enough to skip unchanged
/// regions on a typical desktop.
pub struct DeltaDetector {
    previous_frame: Option<RawScreenFrame>,
    block_size: usize,
}

impl DeltaDetector {
    /// Create a new detector with the given tile size (in pixels).
    pub fn new(block_size: usize) -> Self {
        assert!(block_size > 0, "block_size must be > 0");
        Self {
            previous_frame: None,
            block_size,
        }
    }

    /// Reset the detector, forcing the next frame to be a full frame.
    pub fn reset(&mut self) {
        self.previous_frame = None;
    }

    /// Compare `current` against the stored previous frame.
    ///
    /// The first call (or the call after [`reset`](Self::reset))
    /// always produces a full-frame delta.
    pub fn detect(&mut self, current: &RawScreenFrame) -> DeltaFrame {
        let delta = match &self.previous_frame {
            Some(prev) if prev.width == current.width && prev.height == current.height => {
                self.detect_blocks(current, prev)
            }
            _ => {
                // First frame or resolution change → full frame.
                DeltaFrame {
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
                }
            }
        };

        self.previous_frame = Some(current.clone());
        delta
    }

    // ── Internal ─────────────────────────────────────────────────

    fn detect_blocks(&self, current: &RawScreenFrame, previous: &RawScreenFrame) -> DeltaFrame {
        let w = current.width as usize;
        let h = current.height as usize;
        let bs = self.block_size;

        let blocks_x = (w + bs - 1) / bs;
        let blocks_y = (h + bs - 1) / bs;

        let mut changed = Vec::new();

        for by in 0..blocks_y {
            for bx in 0..blocks_x {
                let start_x = bx * bs;
                let start_y = by * bs;
                let end_x = cmp::min(start_x + bs, w);
                let end_y = cmp::min(start_y + bs, h);

                if Self::block_differs(current, previous, start_x, start_y, end_x, end_y) {
                    changed.push(Block {
                        x: start_x as u32,
                        y: start_y as u32,
                        width: (end_x - start_x) as u32,
                        height: (end_y - start_y) as u32,
                    });
                }
            }
        }

        // If more than 80 % of blocks changed it's cheaper to send a full frame.
        let total_blocks = blocks_x * blocks_y;
        let full_frame = !changed.is_empty()
            && changed.len() as f64 / total_blocks as f64 > 0.80;

        DeltaFrame {
            frame_number: 0,
            timestamp: current.timestamp,
            width: current.width,
            height: current.height,
            changed_blocks: if full_frame {
                vec![Block {
                    x: 0,
                    y: 0,
                    width: current.width,
                    height: current.height,
                }]
            } else {
                changed
            },
            full_frame,
        }
    }

    /// Row-by-row byte comparison for a rectangular tile.
    fn block_differs(
        current: &RawScreenFrame,
        previous: &RawScreenFrame,
        start_x: usize,
        start_y: usize,
        end_x: usize,
        end_y: usize,
    ) -> bool {
        let bpp = current.format.bytes_per_pixel();
        let stride = current.stride as usize;

        for y in start_y..end_y {
            let row_offset = y * stride;
            let left = start_x * bpp;
            let right = end_x * bpp;

            let cur_slice = &current.data[row_offset + left..row_offset + right];
            let prev_slice = &previous.data[row_offset + left..row_offset + right];

            if cur_slice != prev_slice {
                return true;
            }
        }
        false
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn make_frame(w: u32, h: u32, fill: u8) -> RawScreenFrame {
        let stride = w * 4;
        RawScreenFrame {
            width: w,
            height: h,
            stride,
            format: crate::rdp::types::PixelFormat::Bgra8,
            data: vec![fill; (stride * h) as usize],
            timestamp: Instant::now(),
        }
    }

    #[test]
    fn first_frame_is_full() {
        let mut det = DeltaDetector::new(64);
        let frame = make_frame(128, 128, 0);
        let delta = det.detect(&frame);
        assert!(delta.full_frame);
        assert_eq!(delta.changed_blocks.len(), 1);
        assert_eq!(delta.changed_blocks[0].width, 128);
    }

    #[test]
    fn identical_frame_has_no_changes() {
        let mut det = DeltaDetector::new(64);
        let frame = make_frame(128, 128, 0xAA);
        let _ = det.detect(&frame);
        let delta = det.detect(&frame);
        assert!(!delta.full_frame);
        assert!(delta.changed_blocks.is_empty());
    }

    #[test]
    fn single_pixel_change_detects_block() {
        let mut det = DeltaDetector::new(64);
        let frame1 = make_frame(128, 128, 0);
        let _ = det.detect(&frame1);

        let mut frame2 = make_frame(128, 128, 0);
        // Change one pixel in block (0,0).
        frame2.data[0] = 0xFF;
        let delta = det.detect(&frame2);

        assert!(!delta.full_frame);
        assert_eq!(delta.changed_blocks.len(), 1);
        assert_eq!(delta.changed_blocks[0].x, 0);
        assert_eq!(delta.changed_blocks[0].y, 0);
    }

    #[test]
    fn full_change_collapses_to_full_frame() {
        let mut det = DeltaDetector::new(64);
        let frame1 = make_frame(128, 128, 0);
        let _ = det.detect(&frame1);

        let frame2 = make_frame(128, 128, 0xFF);
        let delta = det.detect(&frame2);

        // All blocks differ → should be promoted to full_frame.
        assert!(delta.full_frame);
    }

    #[test]
    fn change_ratio_calculation() {
        let delta = DeltaFrame {
            frame_number: 0,
            timestamp: Instant::now(),
            width: 100,
            height: 100,
            changed_blocks: vec![Block {
                x: 0,
                y: 0,
                width: 50,
                height: 50,
            }],
            full_frame: false,
        };
        let ratio = delta.change_ratio();
        assert!((ratio - 0.25).abs() < 1e-6);
    }

    #[test]
    fn reset_forces_full_frame() {
        let mut det = DeltaDetector::new(64);
        let frame = make_frame(64, 64, 0);
        let _ = det.detect(&frame);
        det.reset();
        let delta = det.detect(&frame);
        assert!(delta.full_frame);
    }
}
