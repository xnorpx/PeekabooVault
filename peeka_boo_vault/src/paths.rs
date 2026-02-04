//! Platform-specific paths for PeekabooVault
//!
//! Handles configuration and data storage paths across:
//! - Windows: %APPDATA%\PeekabooVault or %PROGRAMDATA%\PeekabooVault
//! - macOS: ~/Library/Application Support/PeekabooVault
//! - Linux: ~/.config/peekaboovault (XDG_CONFIG_HOME)
//! - Docker: /config and /recordings (mount points)
//!
//! Environment variables for Docker/custom deployments:
//! - PEEKABOOVAULT_CONFIG_DIR: Override config directory
//! - PEEKABOOVAULT_DATA_DIR: Override data directory
//! - PEEKABOOVAULT_RECORDINGS_DIR: Override recordings directory

use std::path::PathBuf;

/// Application name for path construction
const APP_NAME: &str = "PeekabooVault";
#[cfg(target_os = "linux")]
const APP_NAME_LOWER: &str = "peekaboovault";

/// Get the configuration directory path
/// 
/// Priority:
/// 1. PEEKABOOVAULT_CONFIG_DIR environment variable
/// 2. Docker: /config (if exists)
/// 3. Platform-specific user directory
pub fn config_dir() -> PathBuf {
    // Check environment variable first
    if let Ok(dir) = std::env::var("PEEKABOOVAULT_CONFIG_DIR") {
        return PathBuf::from(dir);
    }

    // Check for Docker mount point
    let docker_config = PathBuf::from("/config");
    if docker_config.exists() {
        return docker_config;
    }

    // Platform-specific paths
    platform_config_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Get the data directory path (databases, cache)
/// 
/// Priority:
/// 1. PEEKABOOVAULT_DATA_DIR environment variable
/// 2. Docker: /data (if exists)
/// 3. Platform-specific data directory
pub fn data_dir() -> PathBuf {
    // Check environment variable first
    if let Ok(dir) = std::env::var("PEEKABOOVAULT_DATA_DIR") {
        return PathBuf::from(dir);
    }

    // Check for Docker mount point
    let docker_data = PathBuf::from("/data");
    if docker_data.exists() {
        return docker_data;
    }

    // Platform-specific paths
    platform_data_dir().unwrap_or_else(|| PathBuf::from("./data"))
}

/// Get the recordings directory path
/// 
/// Priority:
/// 1. PEEKABOOVAULT_RECORDINGS_DIR environment variable
/// 2. Docker: /recordings (if exists)
/// 3. {data_dir}/recordings
pub fn recordings_dir() -> PathBuf {
    // Check environment variable first
    if let Ok(dir) = std::env::var("PEEKABOOVAULT_RECORDINGS_DIR") {
        return PathBuf::from(dir);
    }

    // Check for Docker mount point
    let docker_recordings = PathBuf::from("/recordings");
    if docker_recordings.exists() {
        return docker_recordings;
    }

    // Default to data_dir/recordings
    data_dir().join("recordings")
}

/// Get the default configuration file path
pub fn default_config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Get the hot database path
pub fn hot_db_path() -> PathBuf {
    data_dir().join("hot.db")
}

/// Get the cold database path
pub fn cold_db_path() -> PathBuf {
    data_dir().join("cold.db")
}

/// Get the hot recordings storage path
pub fn hot_recordings_path() -> PathBuf {
    recordings_dir().join("hot")
}

/// Get the cold recordings storage path
pub fn cold_recordings_path() -> PathBuf {
    recordings_dir().join("cold")
}

/// Get the log directory path
pub fn log_dir() -> PathBuf {
    // Check environment variable
    if let Ok(dir) = std::env::var("PEEKABOOVAULT_LOG_DIR") {
        return PathBuf::from(dir);
    }

    // Docker: /logs if exists
    let docker_logs = PathBuf::from("/logs");
    if docker_logs.exists() {
        return docker_logs;
    }

    // Platform-specific
    #[cfg(target_os = "windows")]
    {
        data_dir().join("logs")
    }
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir()
            .map(|h| h.join("Library/Logs").join(APP_NAME))
            .unwrap_or_else(|| data_dir().join("logs"))
    }
    #[cfg(target_os = "linux")]
    {
        // /var/log for system service, otherwise data_dir/logs
        let var_log = PathBuf::from("/var/log").join(APP_NAME_LOWER);
        if var_log.parent().map(|p| p.exists()).unwrap_or(false) {
            var_log
        } else {
            data_dir().join("logs")
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        data_dir().join("logs")
    }
}

/// Platform-specific configuration directory
fn platform_config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        // Windows: %APPDATA%\PeekabooVault (e.g., C:\Users\<user>\AppData\Roaming\PeekabooVault)
        dirs::config_dir().map(|p| p.join(APP_NAME))
    }

    #[cfg(target_os = "macos")]
    {
        // macOS: ~/Library/Application Support/PeekabooVault
        dirs::config_dir().map(|p| p.join(APP_NAME))
    }

    #[cfg(target_os = "linux")]
    {
        // Linux: ~/.config/peekaboovault (XDG_CONFIG_HOME)
        dirs::config_dir().map(|p| p.join(APP_NAME_LOWER))
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        // Fallback for other platforms
        dirs::config_dir().map(|p| p.join(APP_NAME_LOWER))
    }
}

/// Platform-specific data directory
fn platform_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        // Windows: %LOCALAPPDATA%\PeekabooVault (e.g., C:\Users\<user>\AppData\Local\PeekabooVault)
        dirs::data_local_dir().map(|p| p.join(APP_NAME))
    }

    #[cfg(target_os = "macos")]
    {
        // macOS: ~/Library/Application Support/PeekabooVault
        dirs::data_dir().map(|p| p.join(APP_NAME))
    }

    #[cfg(target_os = "linux")]
    {
        // Linux: ~/.local/share/peekaboovault (XDG_DATA_HOME)
        dirs::data_dir().map(|p| p.join(APP_NAME_LOWER))
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        dirs::data_dir().map(|p| p.join(APP_NAME_LOWER))
    }
}

/// Ensure all required directories exist
pub fn ensure_directories() -> std::io::Result<()> {
    std::fs::create_dir_all(config_dir())?;
    std::fs::create_dir_all(data_dir())?;
    std::fs::create_dir_all(hot_recordings_path())?;
    std::fs::create_dir_all(cold_recordings_path())?;
    Ok(())
}

/// Path information for display/debugging
#[derive(Debug, Clone)]
pub struct PathInfo {
    pub config_dir: PathBuf,
    pub config_file: PathBuf,
    pub data_dir: PathBuf,
    pub recordings_dir: PathBuf,
    pub hot_db: PathBuf,
    pub cold_db: PathBuf,
    pub log_dir: PathBuf,
}

impl PathInfo {
    /// Get all current paths
    pub fn current() -> Self {
        Self {
            config_dir: config_dir(),
            config_file: default_config_path(),
            data_dir: data_dir(),
            recordings_dir: recordings_dir(),
            hot_db: hot_db_path(),
            cold_db: cold_db_path(),
            log_dir: log_dir(),
        }
    }
}

impl std::fmt::Display for PathInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "PeekabooVault Paths:")?;
        writeln!(f, "  Config:      {}", self.config_dir.display())?;
        writeln!(f, "  Config File: {}", self.config_file.display())?;
        writeln!(f, "  Data:        {}", self.data_dir.display())?;
        writeln!(f, "  Recordings:  {}", self.recordings_dir.display())?;
        writeln!(f, "  Hot DB:      {}", self.hot_db.display())?;
        writeln!(f, "  Cold DB:     {}", self.cold_db.display())?;
        writeln!(f, "  Logs:        {}", self.log_dir.display())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_info() {
        let info = PathInfo::current();
        println!("{}", info);
        
        // Paths should not be empty
        assert!(!info.config_dir.as_os_str().is_empty());
        assert!(!info.data_dir.as_os_str().is_empty());
    }

    #[test]
    fn test_env_override() {
        // Save original value
        let original = std::env::var("PEEKABOOVAULT_CONFIG_DIR").ok();
        
        // Set custom path
        // SAFETY: This test is single-threaded and we restore the original value
        unsafe {
            std::env::set_var("PEEKABOOVAULT_CONFIG_DIR", "/custom/config");
        }
        assert_eq!(config_dir(), PathBuf::from("/custom/config"));
        
        // Restore original
        // SAFETY: This test is single-threaded
        unsafe {
            match original {
                Some(val) => std::env::set_var("PEEKABOOVAULT_CONFIG_DIR", val),
                None => std::env::remove_var("PEEKABOOVAULT_CONFIG_DIR"),
            }
        }
    }
}
