//! Configuration for the RDP slave service.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Top-level configuration loaded from a TOML file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SlaveConfig {
    /// Network settings.
    pub network: NetworkConfig,
    /// Screen capture settings.
    pub screen: ScreenConfig,
    /// Performance tuning.
    pub performance: PerformanceConfig,
    /// Logging settings.
    pub logging: LoggingConfig,
}

/// Network configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    /// UDP port to bind for screen data.
    pub listen_port: u16,
    /// TCP port to listen for control connections.
    pub control_port: u16,
    /// Maximum concurrent master connections (1 for direct RJ-45).
    pub max_connections: u32,
}

/// Screen capture configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenConfig {
    /// Capture quality preset: "low", "medium", "high".
    pub capture_quality: String,
    /// Target frames per second.
    pub fps: u8,
    /// Enable delta detection (send only changed blocks).
    pub delta_detection: bool,
    /// Block size for delta detection (pixels).
    pub block_size: usize,
    /// Monitor index to capture (0 = primary).
    pub monitor_index: u32,
    /// DXGI acquire timeout in milliseconds.
    pub capture_timeout_ms: u32,
}

/// Performance tuning.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PerformanceConfig {
    /// Target bandwidth in megabytes per second.
    pub target_bandwidth_mbps: u64,
    /// Enable adaptive quality adjustment.
    pub adaptive_quality: bool,
}

/// Logging settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    /// Log level: "trace", "debug", "info", "warn", "error".
    pub level: String,
    /// Optional log file path. If empty, logs to stderr.
    pub file: String,
}

// ── Defaults ─────────────────────────────────────────────────────

impl Default for SlaveConfig {
    fn default() -> Self {
        Self {
            network: NetworkConfig::default(),
            screen: ScreenConfig::default(),
            performance: PerformanceConfig::default(),
            logging: LoggingConfig::default(),
        }
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            listen_port: 7331,
            control_port: 7332,
            max_connections: 1,
        }
    }
}

impl Default for ScreenConfig {
    fn default() -> Self {
        Self {
            capture_quality: "high".into(),
            fps: 60,
            delta_detection: true,
            block_size: 64,
            monitor_index: 0,
            capture_timeout_ms: 100,
        }
    }
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            target_bandwidth_mbps: 100,
            adaptive_quality: true,
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".into(),
            file: String::new(),
        }
    }
}

// ── Loading ──────────────────────────────────────────────────────

impl SlaveConfig {
    /// Load configuration from a TOML file, falling back to defaults.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => toml::from_str(&contents).unwrap_or_else(|e| {
                tracing::warn!("invalid config {}: {e}; using defaults", path.display());
                Self::default()
            }),
            Err(_) => {
                tracing::info!(
                    "no config at {}; using defaults",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Write the default configuration to a file (for bootstrapping).
    pub fn write_default(path: &Path) -> std::io::Result<()> {
        let cfg = Self::default();
        let text = toml::to_string_pretty(&cfg)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(path, text)
    }

    /// Convert capture settings into a `ScreenServiceConfig`.
    pub fn to_service_config(&self) -> tix_core::rdp::service::ScreenServiceConfig {
        tix_core::rdp::service::ScreenServiceConfig {
            target_fps: self.screen.fps.clamp(1, 60),
            block_size: self.screen.block_size.max(8),
            target_bandwidth: self.performance.target_bandwidth_mbps * 1024 * 1024,
            monitor_index: self.screen.monitor_index,
            capture_timeout_ms: self.screen.capture_timeout_ms,
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_serializes() {
        let cfg = SlaveConfig::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        assert!(text.contains("listen_port"));
        assert!(text.contains("fps"));
    }

    #[test]
    fn roundtrip_config() {
        let cfg = SlaveConfig::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let parsed: SlaveConfig = toml::from_str(&text).unwrap();
        assert_eq!(parsed.network.listen_port, 7331);
        assert_eq!(parsed.screen.fps, 60);
    }

    #[test]
    fn to_service_config_clamps() {
        let mut cfg = SlaveConfig::default();
        cfg.screen.fps = 120; // beyond max
        let svc = cfg.to_service_config();
        assert_eq!(svc.target_fps, 60);
    }
}
