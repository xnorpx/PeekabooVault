//! API DTOs for PeekabooVault
//!
//! Request and response types for the HTTP API.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

/// A discovered ONVIF device from WS-Discovery
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredDevice {
    /// Unique identifier for this discovery result
    pub id: Uuid,
    /// WS-Discovery address reference (URN)
    pub address: String,
    /// Device name from scopes (if available)
    pub name: Option<String>,
    /// Hardware identifier from scopes (if available)
    pub hardware: Option<String>,
    /// Device types (e.g., "NetworkVideoTransmitter")
    pub types: Vec<String>,
    /// ONVIF service URLs
    pub urls: Vec<Url>,
    /// When this device was discovered
    pub discovered_at: DateTime<Utc>,
    /// Whether this device has been onboarded
    pub is_onboarded: bool,
}

/// A camera that has been onboarded into the system
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Camera {
    /// Unique identifier for this camera
    pub id: Uuid,
    /// Human-readable name for this camera
    pub name: String,
    /// ONVIF device address reference (links to DiscoveredDevice)
    pub onvif_address: Option<String>,
    /// Primary ONVIF service URL
    pub onvif_url: Option<Url>,
    /// Main stream RTSP URI
    pub main_stream_uri: Option<String>,
    /// Sub stream RTSP URI
    pub sub_stream_uri: Option<String>,
    /// All available stream profiles from ONVIF
    pub stream_profiles: Vec<StreamProfile>,
    /// Whether credentials have been configured
    pub has_credentials: bool,
    /// Connection status
    pub status: CameraStatus,
    /// Last successful probe time
    pub last_seen: Option<DateTime<Utc>>,
    /// Device manufacturer (from ONVIF GetDeviceInformation)
    pub manufacturer: Option<String>,
    /// Device model (from ONVIF GetDeviceInformation)
    pub model: Option<String>,
    /// Firmware version
    pub firmware_version: Option<String>,
    /// Serial number
    pub serial_number: Option<String>,
    /// Hardware ID
    pub hardware_id: Option<String>,
}

/// Stream profile information from ONVIF
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamProfile {
    /// Profile token (unique identifier for this profile)
    pub token: String,
    /// Profile name (e.g., "mainStream", "subStream")
    pub name: String,
    /// RTSP stream URI
    pub stream_uri: String,
    /// Video codec (H264, JPEG, MPEG4, H265)
    pub encoding: Option<String>,
    /// Video width in pixels
    pub width: Option<u32>,
    /// Video height in pixels
    pub height: Option<u32>,
    /// Maximum frame rate in fps
    pub frame_rate: Option<f32>,
    /// Maximum bitrate in kbps
    pub bitrate_kbps: Option<i32>,
    /// Quality setting (0-100)
    pub quality: Option<f64>,
    /// GOP length (I-frame interval)
    pub gop_length: Option<i32>,
    /// H.264/H.265 profile (Baseline, Main, High)
    pub codec_profile: Option<String>,
    /// Encoding interval (1 = every frame)
    pub encoding_interval: Option<i32>,
    /// Whether frame rate is guaranteed
    pub guaranteed_frame_rate: Option<bool>,
}

/// Camera connection status
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CameraStatus {
    /// Camera is online and responding
    Online,
    /// Camera is not responding
    Offline,
    /// Camera needs credentials
    Unauthorized,
    /// Camera status unknown (not yet probed)
    #[default]
    Unknown,
    /// Currently probing the camera
    Probing,
}

/// Request to start a discovery scan
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverRequest {
    /// Duration to scan in seconds (default: 5)
    #[serde(default = "default_duration")]
    pub duration_secs: u64,
}

fn default_duration() -> u64 {
    5
}

impl Default for DiscoverRequest {
    fn default() -> Self {
        Self { duration_secs: 5 }
    }
}

/// Response from a discovery scan
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverResponse {
    /// Whether the scan was successful
    pub success: bool,
    /// Error message if scan failed
    pub error: Option<String>,
    /// Discovered devices
    pub devices: Vec<DiscoveredDevice>,
    /// How long the scan took
    pub scan_duration_ms: u64,
}

/// Request to add/onboard a camera
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddCameraRequest {
    /// Human-readable name for the camera
    pub name: String,
    /// Host (IP address or hostname) - we'll auto-detect the ONVIF URL
    pub host: String,
    /// Optional username for ONVIF authentication
    pub username: Option<String>,
    /// Optional password for ONVIF authentication
    pub password: Option<String>,
}

/// Request to set camera credentials
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetCredentialsRequest {
    /// Username for ONVIF/RTSP authentication
    pub username: String,
    /// Password for ONVIF/RTSP authentication
    pub password: String,
}

/// Response from setting credentials
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetCredentialsResponse {
    pub success: bool,
    pub error: Option<String>,
    /// Updated camera info after credential validation
    pub camera: Option<Camera>,
}

/// Request to probe a camera (fetch device info + stream URIs)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeRequest {
    /// Whether to refresh stream URIs
    #[serde(default = "default_true")]
    pub refresh_streams: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ProbeRequest {
    fn default() -> Self {
        Self {
            refresh_streams: true,
        }
    }
}

/// Response from probing a camera
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResponse {
    pub success: bool,
    pub error: Option<String>,
    /// Updated camera info
    pub camera: Option<Camera>,
    /// Stream profiles found
    pub profiles: Vec<StreamProfile>,
}

/// Generic API response wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiResponse<T> {
    pub success: bool,
    pub error: Option<String>,
    pub data: Option<T>,
}

impl<T> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            error: None,
            data: Some(data),
        }
    }

    pub fn err(error: impl Into<String>) -> Self {
        Self {
            success: false,
            error: Some(error.into()),
            data: None,
        }
    }
}

/// Server status information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    /// Server version
    pub version: String,
    /// Server uptime in seconds
    pub uptime_secs: u64,
    /// Number of configured cameras
    pub camera_count: usize,
    /// Number of cameras currently online
    pub cameras_online: usize,
    /// Whether a discovery scan is in progress
    pub discovery_in_progress: bool,
}
