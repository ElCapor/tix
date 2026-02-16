//! Master-side screen frame consumer.
//!
//! Receives encoded frames from the [`ScreenTransport`], decodes them
//! via [`FrameDecoder`], and provides the latest frame buffer to the
//! display layer.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::watch;

use crate::error::TixError;
use crate::rdp::decoder::FrameDecoder;
use crate::rdp::transport::ScreenTransport;
use crate::rdp::types::PixelFormat;

// ── FrameStats ───────────────────────────────────────────────────

/// Per-frame statistics exposed to the UI.
#[derive(Debug, Clone, Default)]
pub struct FrameStats {
    /// Current smoothed frames per second.
    pub fps: f64,
    /// Total frames received since start.
    pub total_frames: u64,
    /// Total bytes received (compressed, from the network).
    pub total_bytes: u64,
    /// Last frame width.
    pub width: u32,
    /// Last frame height.
    pub height: u32,
}

// ── ScreenClient ─────────────────────────────────────────────────

/// Master-side consumer that receives and decodes screen frames.
///
/// The decoded frame buffer is published via a `tokio::sync::watch`
/// channel so the display renderer can read the latest frame without
/// blocking the receive loop.
pub struct ScreenClient {
    transport: Arc<ScreenTransport>,
    decoder: FrameDecoder,
    running: Arc<AtomicBool>,
    pixel_format: PixelFormat,
    /// Sender half of the frame-buffer watch channel.
    frame_tx: watch::Sender<Vec<u8>>,
    /// Receiver half — clone this to get frames in the renderer.
    frame_rx: watch::Receiver<Vec<u8>>,
    /// Stats channel.
    stats_tx: watch::Sender<FrameStats>,
    stats_rx: watch::Receiver<FrameStats>,
}

impl ScreenClient {
    /// Create a new client wrapping the given transport.
    ///
    /// `pixel_format` describes the expected pixel layout (typically
    /// [`PixelFormat::Bgra8`] from DXGI capture).
    pub fn new(transport: ScreenTransport, pixel_format: PixelFormat) -> Self {
        let (frame_tx, frame_rx) = watch::channel(Vec::new());
        let (stats_tx, stats_rx) = watch::channel(FrameStats::default());
        Self {
            transport: Arc::new(transport),
            decoder: FrameDecoder::new(),
            running: Arc::new(AtomicBool::new(false)),
            pixel_format,
            frame_tx,
            frame_rx,
            stats_tx,
            stats_rx,
        }
    }

    /// Obtain a `watch::Receiver` that yields the latest decoded
    /// frame buffer whenever a new frame arrives.
    pub fn frame_receiver(&self) -> watch::Receiver<Vec<u8>> {
        self.frame_rx.clone()
    }

    /// Obtain a `watch::Receiver` for frame statistics.
    pub fn stats_receiver(&self) -> watch::Receiver<FrameStats> {
        self.stats_rx.clone()
    }

    /// A cloneable stop handle.
    pub fn stop_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.running)
    }

    /// Run the receive loop.
    ///
    /// Blocks the calling task until [`stop`](Self::stop) is invoked or
    /// the transport encounters an unrecoverable error.
    pub async fn run(&mut self) -> Result<(), TixError> {
        self.running.store(true, Ordering::SeqCst);

        let bpp = self.pixel_format.bytes_per_pixel();
        let mut fps_samples: Vec<Duration> = Vec::with_capacity(120);
        let mut last_frame_time = Instant::now();
        let mut total_frames: u64 = 0;
        let mut total_bytes: u64 = 0;

        while self.running.load(Ordering::SeqCst) {
            let encoded = match self.transport.receive_frame().await {
                Ok(f) => f,
                Err(TixError::Timeout(_)) => continue,
                Err(e) => return Err(e),
            };

            total_bytes += encoded.data.len() as u64;
            total_frames += 1;

            // Decode.
            let decoded = self.decoder.decode(&encoded)?;
            let _ = self.decoder.apply(&decoded, bpp);

            // Publish.
            let buf = self.decoder.frame_buffer().to_vec();
            let _ = self.frame_tx.send(buf);

            // FPS tracking.
            let now = Instant::now();
            fps_samples.push(now.duration_since(last_frame_time));
            last_frame_time = now;
            if fps_samples.len() > 60 {
                fps_samples.remove(0);
            }
            let avg_secs: f64 =
                fps_samples.iter().map(|d| d.as_secs_f64()).sum::<f64>() / fps_samples.len() as f64;
            let fps = if avg_secs > 0.0 { 1.0 / avg_secs } else { 0.0 };

            let _ = self.stats_tx.send(FrameStats {
                fps,
                total_frames,
                total_bytes,
                width: decoded.width,
                height: decoded.height,
            });
        }

        Ok(())
    }

    /// Signal the client to stop.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Whether the receive loop is currently running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}
