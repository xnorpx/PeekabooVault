//! Configuration management for PeekabooVault
//!
//! TOML-based configuration following Frigate's config-as-truth philosophy.

use crate::paths;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;

/// Main configuration for PeekabooVault
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Server configuration
    pub server: ServerConfig,
    /// Discovery configuration
    pub discovery: DiscoveryConfig,
    /// Storage configuration
    pub storage: StorageConfig,
    /// WebRTC configuration
    pub webrtc: WebRtcConfig,
}

/// Server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Address to bind the HTTP server
    pub bind_address: SocketAddr,
    /// Path to static UI assets
    pub static_dir: Option<PathBuf>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "0.0.0.0:8080".parse().unwrap(),
            static_dir: Some(PathBuf::from("./ui/build")),
        }
    }
}

/// Discovery configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscoveryConfig {
    /// Default discovery scan duration in seconds
    pub default_duration_secs: u64,
    /// Automatically scan on startup
    pub auto_discover: bool,
    /// Discovery scan interval in seconds (0 = disabled)
    pub scan_interval_secs: u64,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            default_duration_secs: 5,
            auto_discover: true,
            scan_interval_secs: 300, // 5 minutes
        }
    }
}

/// Storage configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// Path to the hot SQLite database (recent recordings)
    pub hot_db_path: PathBuf,
    /// Path to the cold SQLite database (archived recordings)
    pub cold_db_path: PathBuf,
    /// Path to hot recording storage
    pub hot_storage_path: PathBuf,
    /// Path to cold recording storage
    pub cold_storage_path: PathBuf,
    /// Retention configuration
    pub retention: RetentionConfig,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            hot_db_path: paths::hot_db_path(),
            cold_db_path: paths::cold_db_path(),
            hot_storage_path: paths::hot_recordings_path(),
            cold_storage_path: paths::cold_recordings_path(),
            retention: RetentionConfig::default(),
        }
    }
}

/// Retention policy configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RetentionConfig {
    /// Hot storage quota in bytes (default: 50GB)
    pub hot_quota_bytes: u64,
    /// Cold storage quota in bytes (default: 500GB)
    pub cold_quota_bytes: u64,
    /// Maximum age in hot storage (seconds, 0 = unlimited)
    pub hot_max_age_secs: u64,
    /// Maximum age in cold storage (seconds, 0 = unlimited)
    pub cold_max_age_secs: u64,
    /// Hot-to-cold migration interval in seconds
    pub migration_interval_secs: u64,
    /// Minimum hot storage usage before migration starts (0.0-1.0)
    pub migration_threshold: f64,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            hot_quota_bytes: 50 * 1024 * 1024 * 1024,  // 50 GB
            cold_quota_bytes: 500 * 1024 * 1024 * 1024, // 500 GB
            hot_max_age_secs: 7 * 24 * 60 * 60,        // 7 days
            cold_max_age_secs: 0,                       // unlimited
            migration_interval_secs: 300,               // 5 minutes
            migration_threshold: 0.8,                   // 80% full
        }
    }
}

/// WebRTC configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebRtcConfig {
    /// Address to bind the UDP socket for WebRTC
    pub bind_address: SocketAddr,
    /// Enable WebRTC server
    pub enabled: bool,
    /// Max sessions per camera
    pub max_sessions_per_camera: usize,
    /// Session timeout in seconds
    pub session_timeout_secs: u64,
}

impl Default for WebRtcConfig {
    fn default() -> Self {
        Self {
            bind_address: "0.0.0.0:10000".parse().unwrap(),
            enabled: true,
            max_sessions_per_camera: 10,
            session_timeout_secs: 60,
        }
    }
}

impl Config {
    /// Load configuration from a TOML file
    pub fn load(path: &PathBuf) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }

    /// Load configuration from file or use defaults
    pub fn load_or_default(path: Option<&PathBuf>) -> Self {
        match path {
            Some(p) if p.exists() => Self::load(p).unwrap_or_else(|e| {
                tracing::warn!("Failed to load config from {:?}: {}, using defaults", p, e);
                Self::default()
            }),
            _ => Self::default(),
        }
    }

    /// Save configuration to a TOML file
    pub fn save(&self, path: &PathBuf) -> anyhow::Result<()> {
        let content = toml::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        Ok(())
    }
}
