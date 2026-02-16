//! Slave-side screen capture service.
//!
//! Orchestrates the full capture pipeline:
//!
//! 1. [`DxgiCapturer`] acquires raw frames from the desktop.
//! 2. [`DeltaDetector`] identifies changed blocks.
//! 3. [`AdaptiveEncoder`] compresses the delta.
//! 4. [`ScreenTransport`] sends UDP datagrams to the master.
//!
//! The service runs in a Tokio task and respects a
//! `CancellationToken`-style shutdown via its `running` flag.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::error::TixError;
use crate::rdp::bandwidth::BandwidthEstimator;
use crate::rdp::capture::DxgiCapturer;
use crate::rdp::delta::DeltaDetector;
use crate::rdp::encoder::AdaptiveEncoder;
use crate::rdp::input::InputInjector;
use crate::rdp::transport::ScreenTransport;

// ── ScreenServiceConfig ──────────────────────────────────────────

/// Configuration for [`ScreenService`].
#[derive(Debug, Clone)]
pub struct ScreenServiceConfig {
    /// Target frames per second (1..=60).
    pub target_fps: u8,
    /// Delta detection block size in pixels.
    pub block_size: usize,
    /// Target bandwidth in bytes/second for adaptive quality.
    pub target_bandwidth: u64,
    /// Monitor index (0 = primary).
    pub monitor_index: u32,
    /// DXGI frame acquire timeout in milliseconds.
    pub capture_timeout_ms: u32,
}

impl Default for ScreenServiceConfig {
    fn default() -> Self {
        Self {
            target_fps: 60,
            block_size: 64,
            target_bandwidth: 100 * 1024 * 1024, // 100 MB/s
            monitor_index: 0,
            capture_timeout_ms: 100,
        }
    }
}

// ── ScreenService ────────────────────────────────────────────────

/// Slave-side screen capture service.
///
/// # Lifetime
///
/// Call [`run`](Self::run) to start the capture loop. It runs until
/// [`stop`](Self::stop) is called or an unrecoverable error occurs.
pub struct ScreenService {
    capturer: DxgiCapturer,
    delta: DeltaDetector,
    encoder: AdaptiveEncoder,
    transport: Arc<ScreenTransport>,
    injector: InputInjector,
    bandwidth: BandwidthEstimator,
    running: Arc<AtomicBool>,
    config: ScreenServiceConfig,
}

impl ScreenService {
    /// Create a new service with the given transport and default config.
    pub fn new(transport: ScreenTransport) -> Result<Self, TixError> {
        Self::with_config(transport, ScreenServiceConfig::default())
    }

    /// Create a new service with explicit configuration.
    pub fn with_config(
        transport: ScreenTransport,
        config: ScreenServiceConfig,
    ) -> Result<Self, TixError> {
        let capturer = DxgiCapturer::new(config.monitor_index)?;
        let delta = DeltaDetector::new(config.block_size);
        let encoder = AdaptiveEncoder::new(config.target_bandwidth);
        let injector = InputInjector::new();
        let bandwidth = BandwidthEstimator::new();

        Ok(Self {
            capturer,
            delta,
            encoder,
            transport: Arc::new(transport),
            injector,
            bandwidth,
            running: Arc::new(AtomicBool::new(false)),
            config,
        })
    }

    /// A cloneable handle that can be used to stop the service from
    /// another task.
    pub fn stop_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.running)
    }

    /// Reference to the input injector (for handling incoming input
    /// events from the master).
    pub fn injector(&self) -> &InputInjector {
        &self.injector
    }

    /// Current estimated bandwidth in bytes/second.
    pub fn estimated_bandwidth(&self) -> u64 {
        self.bandwidth.estimate_bps()
    }

    /// Run the capture loop.
    ///
    /// This is intended to be spawned on the Tokio runtime:
    ///
    /// ```no_run
    /// # use tix_core::rdp::service::ScreenService;
    /// # async fn example(mut svc: ScreenService) {
    /// let handle = svc.stop_handle();
    /// tokio::spawn(async move { svc.run().await });
    /// // … later …
    /// handle.store(false, std::sync::atomic::Ordering::SeqCst);
    /// # }
    /// ```
    pub async fn run(&mut self) -> Result<(), TixError> {
        self.running.store(true, Ordering::SeqCst);
        let frame_interval = Duration::from_secs_f64(1.0 / self.config.target_fps as f64);
        let mut frame_number: u64 = 0;
        let mut last_bandwidth_check = Instant::now();

        while self.running.load(Ordering::SeqCst) {
            let loop_start = Instant::now();

            // 1. Capture.
            let raw = match self.capturer.capture_frame(self.config.capture_timeout_ms) {
                Ok(f) => f,
                Err(TixError::Timeout(_)) => {
                    // No new desktop frame within the deadline — skip.
                    tokio::task::yield_now().await;
                    continue;
                }
                Err(e) => return Err(e),
            };

            // 2. Delta detection.
            let mut delta = self.delta.detect(&raw);
            delta.frame_number = frame_number;

            // Skip sending if nothing changed.
            if !delta.full_frame && delta.changed_blocks.is_empty() {
                Self::pace(loop_start, frame_interval).await;
                continue;
            }

            // 3. Encode.
            let encoded = self.encoder.encode(&delta, &raw)?;
            let encoded_size = encoded.data.len() as u64;

            // 4. Send.
            self.transport.send_frame(&encoded).await?;

            // 5. Bandwidth tracking.
            self.bandwidth.record(encoded_size);
            frame_number += 1;

            // Adjust quality every second.
            if last_bandwidth_check.elapsed() > Duration::from_secs(1) {
                let bps = self.bandwidth.estimate_bps();
                self.encoder.adjust_quality(bps);
                last_bandwidth_check = Instant::now();
            }

            // 6. Frame pacing.
            Self::pace(loop_start, frame_interval).await;
        }

        Ok(())
    }

    /// Signal the service to stop.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Whether the service is currently running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Sleep for the remainder of the frame interval.
    async fn pace(loop_start: Instant, interval: Duration) {
        let elapsed = loop_start.elapsed();
        if elapsed < interval {
            tokio::time::sleep(interval - elapsed).await;
        }
    }
}
