//! Bandwidth estimator for adaptive quality control.
//!
//! Tracks bytes sent over a rolling window and derives the current
//! throughput in bytes/second. The encoder uses this to decide
//! whether to increase or decrease quality / compression.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Rolling-window bandwidth estimator.
///
/// Records `(timestamp, bytes)` samples and computes the average
/// throughput over the most recent `window` duration.
pub struct BandwidthEstimator {
    /// Samples: `(when, bytes)`.
    samples: VecDeque<(Instant, u64)>,
    /// Window duration.
    window: Duration,
    /// Running total of bytes in the window.
    total_bytes: u64,
    /// Smoothed RTT in microseconds (optional, for latency tracking).
    smoothed_rtt_us: u64,
}

impl BandwidthEstimator {
    /// Create an estimator with a 1-second rolling window.
    pub fn new() -> Self {
        Self::with_window(Duration::from_secs(1))
    }

    /// Create an estimator with a custom window duration.
    pub fn with_window(window: Duration) -> Self {
        Self {
            samples: VecDeque::with_capacity(256),
            window,
            total_bytes: 0,
            smoothed_rtt_us: 0,
        }
    }

    /// Record that `bytes` were transmitted at the current instant.
    pub fn record(&mut self, bytes: u64) {
        self.record_at(Instant::now(), bytes);
    }

    /// Record with an explicit timestamp (useful for testing).
    pub fn record_at(&mut self, when: Instant, bytes: u64) {
        self.samples.push_back((when, bytes));
        self.total_bytes += bytes;
        self.evict(when);
    }

    /// Update the smoothed RTT (exponential moving average, α = 0.125).
    pub fn record_rtt(&mut self, rtt: Duration) {
        let rtt_us = rtt.as_micros() as u64;
        if self.smoothed_rtt_us == 0 {
            self.smoothed_rtt_us = rtt_us;
        } else {
            // EWMA: srtt = 7/8 * srtt + 1/8 * sample
            self.smoothed_rtt_us = self.smoothed_rtt_us * 7 / 8 + rtt_us / 8;
        }
    }

    /// Estimated throughput in bytes/second over the rolling window.
    pub fn estimate_bps(&self) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        let elapsed = match (self.samples.front(), self.samples.back()) {
            (Some((first, _)), Some((last, _))) => {
                let d = last.duration_since(*first);
                if d.is_zero() {
                    Duration::from_millis(1)
                } else {
                    d
                }
            }
            _ => return 0,
        };
        let secs = elapsed.as_secs_f64();
        (self.total_bytes as f64 / secs) as u64
    }

    /// Smoothed round-trip time, or `Duration::ZERO` if not yet measured.
    pub fn latency(&self) -> Duration {
        Duration::from_micros(self.smoothed_rtt_us)
    }

    /// Number of samples currently in the window.
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    // ── Internal ─────────────────────────────────────────────────

    fn evict(&mut self, now: Instant) {
        while let Some(&(ts, bytes)) = self.samples.front() {
            if now.duration_since(ts) > self.window {
                self.samples.pop_front();
                self.total_bytes = self.total_bytes.saturating_sub(bytes);
            } else {
                break;
            }
        }
    }
}

impl Default for BandwidthEstimator {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_estimator_returns_zero() {
        let est = BandwidthEstimator::new();
        assert_eq!(est.estimate_bps(), 0);
    }

    #[test]
    fn single_sample() {
        let mut est = BandwidthEstimator::new();
        est.record(1024);
        // With a single point the interval is zero → we get some value.
        let bps = est.estimate_bps();
        assert!(bps >= 1024);
    }

    #[test]
    fn two_samples_one_second_apart() {
        let mut est = BandwidthEstimator::with_window(Duration::from_secs(5));
        let t0 = Instant::now();
        est.record_at(t0, 1_000_000);
        est.record_at(t0 + Duration::from_secs(1), 1_000_000);
        let bps = est.estimate_bps();
        // 2 MB over 1 second ≈ 2 MB/s.
        assert!(bps >= 1_900_000 && bps <= 2_100_000, "bps = {bps}");
    }

    #[test]
    fn evicts_old_samples() {
        let mut est = BandwidthEstimator::with_window(Duration::from_millis(500));
        let t0 = Instant::now();
        est.record_at(t0, 1000);
        // Fast-forward beyond the window.
        est.record_at(t0 + Duration::from_secs(1), 500);
        // Old sample should be evicted.
        assert_eq!(est.sample_count(), 1);
    }

    #[test]
    fn smoothed_rtt() {
        let mut est = BandwidthEstimator::new();
        est.record_rtt(Duration::from_millis(10));
        assert_eq!(est.latency(), Duration::from_millis(10));

        est.record_rtt(Duration::from_millis(2));
        // EWMA: (10000 * 7/8 + 2000 / 8) = 8750 + 250 = 9000 µs = 9 ms
        assert!(est.latency().as_micros() > 8000 && est.latency().as_micros() < 10000);
    }
}
