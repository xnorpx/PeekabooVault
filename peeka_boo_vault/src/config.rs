//! Configuration management for PeekabooVault
//!
//! TOML-based configuration following Frigate's config-as-truth philosophy.

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
    /// Path to the SQLite database
    pub db_path: PathBuf,
    /// Path to hot recording storage
    pub hot_storage_path: PathBuf,
    /// Path to cold recording storage
    pub cold_storage_path: Option<PathBuf>,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            db_path: PathBuf::from("./data/peekaboovault.db"),
            hot_storage_path: PathBuf::from("./data/recordings/hot"),
            cold_storage_path: None,
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
