//! GUI client configuration.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Top-level configuration for the GUI client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GuiConfig {
    /// Network settings.
    pub network: NetworkConfig,
    /// Display settings.
    pub display: DisplayConfig,
    /// Performance tuning.
    pub performance: PerformanceConfig,
    /// Input forwarding settings.
    pub input: InputConfig,
    /// Logging.
    pub logging: LoggingConfig,
}

/// Network settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    /// Slave control address (IP:port for TCP handshake).
    pub slave_address: String,
    /// Connection timeout in milliseconds.
    pub timeout_ms: u64,
}

/// Display settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    /// Initial window width.
    pub width: u32,
    /// Initial window height.
    pub height: u32,
    /// Start in fullscreen mode.
    pub fullscreen: bool,
    /// Enable vsync (cap rendering to monitor refresh rate).
    pub vsync: bool,
}

/// Performance settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PerformanceConfig {
    /// Max buffered frames before dropping.
    pub buffer_size: u32,
    /// Quality hint: "low", "medium", "high".
    pub quality: String,
}

/// Input forwarding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InputConfig {
    /// Forward mouse events.
    pub capture_mouse: bool,
    /// Forward keyboard events.
    pub capture_keyboard: bool,
}

/// Logging.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    /// Log level.
    pub level: String,
    /// Optional log file.
    pub file: String,
}

// ── Defaults ─────────────────────────────────────────────────────

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            network: NetworkConfig::default(),
            display: DisplayConfig::default(),
            performance: PerformanceConfig::default(),
            input: InputConfig::default(),
            logging: LoggingConfig::default(),
        }
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            slave_address: "127.0.0.1:7332".into(),
            timeout_ms: 5000,
        }
    }
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fullscreen: false,
            vsync: true,
        }
    }
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            buffer_size: 3,
            quality: "high".into(),
        }
    }
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            capture_mouse: true,
            capture_keyboard: true,
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

impl GuiConfig {
    /// Load from a TOML file, falling back to defaults.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => toml::from_str(&contents).unwrap_or_else(|e| {
                tracing::warn!("invalid config {}: {e}; using defaults", path.display());
                Self::default()
            }),
            Err(_) => {
                tracing::info!("no config at {}; using defaults", path.display());
                Self::default()
            }
        }
    }

    /// Write default config to a file.
    pub fn write_default(path: &Path) -> std::io::Result<()> {
        let cfg = Self::default();
        let text = toml::to_string_pretty(&cfg)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(path, text)
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_serializes() {
        let cfg = GuiConfig::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        assert!(text.contains("slave_address"));
        assert!(text.contains("width"));
    }

    #[test]
    fn roundtrip_config() {
        let cfg = GuiConfig::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let parsed: GuiConfig = toml::from_str(&text).unwrap();
        assert_eq!(parsed.display.width, 1920);
        assert_eq!(parsed.network.slave_address, "192.168.1.100:7332");
    }
}
