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
    /// Number of streams currently recording
    pub streams_recording: usize,
    /// Total recordings in database
    pub total_recordings: u64,
}

// ============================================================================
// Recording API Types
// ============================================================================

/// Stream recording configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingConfig {
    /// Whether recording is enabled
    pub enabled: bool,
    /// Which stream to record (main/sub)
    pub stream_type: StreamType,
    /// Retention period in days (0 = infinite)
    pub retention_days: u32,
    /// Segment duration in seconds
    pub segment_duration_secs: u32,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            stream_type: StreamType::Main,
            retention_days: 7,
            segment_duration_secs: 60,
        }
    }
}

/// Stream type (main = high quality, sub = lower quality)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamType {
    #[default]
    Main,
    Sub,
}

impl std::fmt::Display for StreamType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamType::Main => write!(f, "main"),
            StreamType::Sub => write!(f, "sub"),
        }
    }
}

// ============================================================================
// Video Layer / Quality Control
// ============================================================================

/// Video layer selection for adaptive streaming
/// 
/// Layers map to camera streams or transcoded versions:
/// - `High` → Main stream (1080p/4K)
/// - `Medium` → Sub stream (720p/480p) or transcoded
/// - `Low` → Sub stream (480p/360p) or heavily transcoded
/// - `Auto` → Server decides based on bandwidth/UI context
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoLayer {
    /// Highest quality (main stream, ~1080p/4K)
    High,
    /// Medium quality (sub stream, ~720p)
    Medium,
    /// Low quality (sub stream, ~480p or less)
    Low,
    /// Server decides based on bandwidth and UI context
    #[default]
    Auto,
}

impl VideoLayer {
    /// Map layer to stream type
    pub fn to_stream_type(&self) -> StreamType {
        match self {
            VideoLayer::High => StreamType::Main,
            VideoLayer::Medium | VideoLayer::Low => StreamType::Sub,
            VideoLayer::Auto => StreamType::Main, // Default to main for auto
        }
    }
    
    /// Typical bitrate for this layer (kbps)
    pub fn typical_bitrate_kbps(&self) -> u32 {
        match self {
            VideoLayer::High => 4000,   // 4 Mbps
            VideoLayer::Medium => 1500, // 1.5 Mbps
            VideoLayer::Low => 500,     // 500 kbps
            VideoLayer::Auto => 2000,   // 2 Mbps average
        }
    }
}

impl std::fmt::Display for VideoLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VideoLayer::High => write!(f, "high"),
            VideoLayer::Medium => write!(f, "medium"),
            VideoLayer::Low => write!(f, "low"),
            VideoLayer::Auto => write!(f, "auto"),
        }
    }
}

/// Layer selection mode - who decides which layer to send
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayerSelectionMode {
    /// Client explicitly controls layer
    Manual,
    /// Server chooses based on bandwidth
    #[default]
    Adaptive,
    /// UI context based (grid view = low, fullscreen = high)
    UiContext,
}

/// Layer selection preference from client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerPreference {
    /// Preferred layer (when mode is Manual)
    pub preferred_layer: VideoLayer,
    /// Selection mode
    pub mode: LayerSelectionMode,
    /// UI context hint for adaptive mode
    #[serde(default)]
    pub ui_context: Option<UiContext>,
    /// Maximum bitrate client can receive (kbps)
    #[serde(default)]
    pub max_bitrate_kbps: Option<u32>,
}

impl Default for LayerPreference {
    fn default() -> Self {
        Self {
            preferred_layer: VideoLayer::Auto,
            mode: LayerSelectionMode::Adaptive,
            ui_context: None,
            max_bitrate_kbps: None,
        }
    }
}

/// UI context hints for adaptive layer selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UiContext {
    /// Single camera fullscreen view
    Fullscreen,
    /// 2x2 grid (4 cameras)
    Grid2x2,
    /// 3x3 grid (9 cameras)
    Grid3x3,
    /// 4x4 grid (16 cameras)
    Grid4x4,
    /// Picture-in-picture (small overlay)
    PictureInPicture,
    /// Thumbnail preview
    Thumbnail,
    /// Timeline scrubbing (prioritize responsiveness)
    Scrubbing,
}

impl UiContext {
    /// Recommended layer for this UI context
    pub fn recommended_layer(&self) -> VideoLayer {
        match self {
            UiContext::Fullscreen => VideoLayer::High,
            UiContext::Grid2x2 => VideoLayer::Medium,
            UiContext::Grid3x3 | UiContext::Grid4x4 => VideoLayer::Low,
            UiContext::PictureInPicture => VideoLayer::Low,
            UiContext::Thumbnail => VideoLayer::Low,
            UiContext::Scrubbing => VideoLayer::Low,
        }
    }
    
    /// Number of visible streams in this context
    pub fn stream_count(&self) -> u32 {
        match self {
            UiContext::Fullscreen | UiContext::PictureInPicture => 1,
            UiContext::Grid2x2 => 4,
            UiContext::Grid3x3 => 9,
            UiContext::Grid4x4 => 16,
            UiContext::Thumbnail | UiContext::Scrubbing => 1,
        }
    }
}

/// Server notification about layer change
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerChangeNotification {
    /// New active layer
    pub active_layer: VideoLayer,
    /// Previous layer
    pub previous_layer: Option<VideoLayer>,
    /// Reason for change
    pub reason: LayerChangeReason,
    /// New stream type being used
    pub stream_type: StreamType,
    /// Estimated bitrate (kbps)
    pub estimated_bitrate_kbps: Option<u32>,
    /// Resolution if known
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Why the layer changed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayerChangeReason {
    /// Client requested this layer
    ClientRequest,
    /// Bandwidth constrained
    BandwidthLow,
    /// Bandwidth improved
    BandwidthImproved,
    /// UI context changed
    UiContextChange,
    /// Server policy (e.g., too many viewers)
    ServerPolicy,
    /// Initial connection
    Initial,
}

/// Request to start/stop recording
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingRequest {
    /// Which stream to record
    pub stream_type: StreamType,
}

impl Default for RecordingRequest {
    fn default() -> Self {
        Self {
            stream_type: StreamType::Main,
        }
    }
}

/// Recording status for a camera
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStatus {
    /// Camera ID
    pub camera_id: Uuid,
    /// Whether recording is active
    pub is_recording: bool,
    /// Stream being recorded
    pub stream_type: Option<StreamType>,
    /// When recording started
    pub started_at: Option<DateTime<Utc>>,
    /// Total bytes recorded this session
    pub bytes_recorded: u64,
    /// Frames recorded this session
    pub frames_recorded: u64,
    /// Current recording file (if any)
    pub current_file: Option<String>,
}

/// A recording segment (stored video file)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recording {
    /// Recording ID
    pub id: i64,
    /// Camera ID
    pub camera_id: Uuid,
    /// Stream type
    pub stream_type: StreamType,
    /// Start time
    pub start_time: DateTime<Utc>,
    /// End time
    pub end_time: Option<DateTime<Utc>>,
    /// Duration in seconds
    pub duration_secs: Option<f64>,
    /// File size in bytes
    pub size_bytes: u64,
    /// File path
    pub file_path: String,
    /// Video codec
    pub codec: Option<String>,
    /// Resolution
    pub resolution: Option<String>,
}

/// Query parameters for listing recordings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingsQuery {
    /// Filter by camera ID
    pub camera_id: Option<Uuid>,
    /// Start time filter (inclusive)
    pub start_time: Option<DateTime<Utc>>,
    /// End time filter (inclusive)
    pub end_time: Option<DateTime<Utc>>,
    /// Stream type filter
    pub stream_type: Option<StreamType>,
    /// Maximum number of results
    #[serde(default = "default_limit")]
    pub limit: u32,
    /// Offset for pagination
    #[serde(default)]
    pub offset: u32,
}

fn default_limit() -> u32 {
    100
}

impl Default for RecordingsQuery {
    fn default() -> Self {
        Self {
            camera_id: None,
            start_time: None,
            end_time: None,
            stream_type: None,
            limit: default_limit(),
            offset: 0,
        }
    }
}

/// Response containing list of recordings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingsResponse {
    /// Total number of recordings matching query (before pagination)
    pub total: u64,
    /// Recordings in this page
    pub recordings: Vec<Recording>,
}

// === Clip Export API ===

/// Maximum clip export size in bytes (100MB)
pub const MAX_CLIP_EXPORT_BYTES: u64 = 100 * 1024 * 1024;

/// Maximum clip export duration in seconds (10 minutes)
pub const MAX_CLIP_EXPORT_DURATION_SECS: i64 = 600;

/// Request to export a video clip
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipExportRequest {
    /// Camera ID
    pub camera_id: Uuid,
    /// Start time of the clip
    pub start_time: DateTime<Utc>,
    /// End time of the clip
    pub end_time: DateTime<Utc>,
    /// Stream type (defaults to main)
    #[serde(default)]
    pub stream_type: StreamType,
    /// Optional filename for download (without extension)
    pub filename: Option<String>,
}

/// Response for clip export metadata (before download)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipExportInfo {
    /// Estimated size in bytes
    pub estimated_size_bytes: u64,
    /// Actual duration in seconds
    pub duration_secs: f64,
    /// Number of segments included
    pub segment_count: usize,
    /// Whether the clip exceeds size limits
    pub exceeds_limit: bool,
    /// Error message if any
    pub error: Option<String>,
    /// Suggested filename
    pub suggested_filename: String,
}

/// Stream statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamStats {
    /// Camera ID
    pub camera_id: Uuid,
    /// Stream type
    pub stream_type: StreamType,
    /// Connection state
    pub state: StreamState,
    /// Video codec
    pub codec: Option<String>,
    /// Resolution
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Frames received
    pub frames_received: u64,
    /// Keyframes received
    pub keyframes_received: u64,
    /// Bytes received
    pub bytes_received: u64,
    /// Connection uptime in seconds
    pub connected_secs: Option<f64>,
    /// Number of reconnects
    pub reconnect_count: u32,
}

/// Stream connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamState {
    Disconnected,
    Connecting,
    Active,
    Reconnecting,
    Stopped,
}

// ============================================================================
// Playback API Types (Phase 3)
// ============================================================================

/// Seek direction for playback
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SeekDirection {
    /// Find keyframe at or before target time
    Backward,
    /// Find keyframe at or after target time
    Forward,
    /// Find closest keyframe
    Nearest,
}

/// Request to seek to a specific time
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeekRequest {
    /// Target timestamp to seek to
    pub timestamp: DateTime<Utc>,
    /// Seek direction for finding keyframe
    #[serde(default)]
    pub direction: Option<SeekDirection>,
}

/// A video frame for playback
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackFrame {
    /// Frame ID
    pub id: i64,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
    /// Sequence number
    pub sequence_num: i64,
    /// Whether this is a keyframe
    pub is_keyframe: bool,
    /// Frame size in bytes
    pub frame_size: u64,
    /// Video codec
    pub codec: String,
}

/// Request for frames in a time range
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameRangeRequest {
    /// Start time (will seek to previous keyframe if start_from_keyframe is true)
    pub start_time: DateTime<Utc>,
    /// End time
    pub end_time: DateTime<Utc>,
    /// Whether to start from the nearest keyframe before start_time
    #[serde(default = "default_true")]
    pub start_from_keyframe: bool,
}

impl Default for FrameRangeRequest {
    fn default() -> Self {
        Self {
            start_time: Utc::now(),
            end_time: Utc::now(),
            start_from_keyframe: true,
        }
    }
}

/// Response containing frames for playback
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameRangeResponse {
    /// Camera ID
    pub camera_id: Uuid,
    /// Stream type
    pub stream_type: StreamType,
    /// Actual start time (may be earlier due to keyframe seeking)
    pub actual_start_time: DateTime<Utc>,
    /// End time
    pub end_time: DateTime<Utc>,
    /// Total frames in range
    pub frame_count: usize,
    /// Frames metadata (not the actual data)
    pub frames: Vec<PlaybackFrame>,
}

/// Timeline summary for a camera
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineSummary {
    /// Camera ID
    pub camera_id: Uuid,
    /// Stream type
    pub stream_type: StreamType,
    /// Start of available recordings
    pub earliest_time: Option<DateTime<Utc>>,
    /// End of available recordings
    pub latest_time: Option<DateTime<Utc>>,
    /// Total duration in seconds
    pub total_duration_secs: f64,
    /// Recording segments
    pub segments: Vec<TimelineSegment>,
}

/// A segment in the timeline
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineSegment {
    /// Start time
    pub start_time: DateTime<Utc>,
    /// End time
    pub end_time: Option<DateTime<Utc>>,
    /// Duration in seconds
    pub duration_secs: f64,
    /// Number of keyframes (seek points)
    pub keyframe_count: u64,
    /// Whether segment is complete
    pub is_complete: bool,
}

/// Keyframe locations for seeking
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyframeTimeline {
    /// Camera ID
    pub camera_id: Uuid,
    /// Stream type
    pub stream_type: StreamType,
    /// Start of range
    pub start_time: DateTime<Utc>,
    /// End of range
    pub end_time: DateTime<Utc>,
    /// Keyframe timestamps (seek points)
    pub keyframes: Vec<DateTime<Utc>>,
}

// ============================================================================
// WebRTC Signaling Types
// ============================================================================

/// Request to create a WebRTC session
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebRtcOfferRequest {
    /// Camera ID to view
    pub camera_id: Uuid,
    /// Stream type (main or sub)
    #[serde(default)]
    pub stream_type: StreamType,
    /// SDP offer from the browser
    pub sdp: String,
}

/// Response containing SDP answer
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebRtcAnswerResponse {
    /// Whether the request was successful
    pub success: bool,
    /// Error message if failed
    pub error: Option<String>,
    /// Session ID for this WebRTC connection
    pub session_id: Option<Uuid>,
    /// SDP answer to return to browser
    pub sdp: Option<String>,
}

/// Request to add an ICE candidate
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IceCandidateRequest {
    /// Session ID from the offer response
    pub session_id: Uuid,
    /// ICE candidate SDP string
    pub candidate: String,
    /// SDP mid
    pub sdp_mid: Option<String>,
    /// SDP m-line index
    pub sdp_m_line_index: Option<u32>,
}

/// Response to ICE candidate
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IceCandidateResponse {
    pub success: bool,
    pub error: Option<String>,
}

/// WebRTC session info
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebRtcSessionInfo {
    /// Session ID
    pub session_id: Uuid,
    /// Camera ID
    pub camera_id: Uuid,
    /// Stream type
    pub stream_type: String,
    /// Session state
    pub state: String,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
}

/// List of WebRTC sessions
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebRtcSessionsResponse {
    pub sessions: Vec<WebRtcSessionInfo>,
    pub total_count: usize,
}

// ============================================================================
// Storage/Retention API Types (Phase 4)
// ============================================================================

/// Storage statistics response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageStatsResponse {
    /// Hot storage statistics
    pub hot: StorageTierStats,
    /// Cold storage statistics
    pub cold: StorageTierStats,
    /// Per-stream statistics
    pub streams: Vec<StreamStorageStats>,
}

/// Storage usage information for settings UI
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsageResponse {
    /// Hot storage path
    pub hot_path: String,
    /// Hot storage bytes used
    pub hot_used_bytes: u64,
    /// Hot storage quota in bytes
    pub hot_quota_bytes: u64,
    /// Hot storage usage percentage
    pub hot_usage_percent: f64,
    /// Cold storage path
    pub cold_path: String,
    /// Cold storage bytes used
    pub cold_used_bytes: u64,
    /// Cold storage quota in bytes
    pub cold_quota_bytes: u64,
    /// Cold storage usage percentage
    pub cold_usage_percent: f64,
}

/// Statistics for a storage tier (hot or cold)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageTierStats {
    /// Total bytes used
    pub bytes_used: u64,
    /// Configured quota in bytes
    pub quota_bytes: u64,
    /// Usage percentage (0.0-100.0)
    pub usage_percent: f64,
    /// Number of recording segments
    pub recording_count: u64,
    /// Oldest recording timestamp
    pub oldest_recording: Option<DateTime<Utc>>,
    /// Newest recording timestamp
    pub newest_recording: Option<DateTime<Utc>>,
}

/// Per-stream storage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamStorageStats {
    /// Stream ID
    pub stream_id: i64,
    /// Camera ID (if available)
    pub camera_id: Option<Uuid>,
    /// Stream type
    pub stream_type: String,
    /// Total bytes for this stream
    pub total_bytes: u64,
    /// Total duration in seconds
    pub total_duration_secs: f64,
    /// Recording count
    pub recording_count: u64,
    /// Oldest recording
    pub oldest_time: Option<DateTime<Utc>>,
    /// Newest recording
    pub newest_time: Option<DateTime<Utc>>,
}

/// Timeline query request
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineQuery {
    /// Camera ID
    pub camera_id: Uuid,
    /// Start time
    pub start_time: DateTime<Utc>,
    /// End time
    pub end_time: DateTime<Utc>,
    /// Granularity in seconds (for aggregation)
    #[serde(default = "default_granularity")]
    pub granularity_secs: u32,
}

fn default_granularity() -> u32 {
    60 // 1 minute buckets by default
}

impl Default for TimelineQuery {
    fn default() -> Self {
        Self {
            camera_id: Uuid::nil(),
            start_time: Utc::now(),
            end_time: Utc::now(),
            granularity_secs: default_granularity(),
        }
    }
}

/// Timeline response with hot/cold information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineResponse {
    /// Camera ID
    pub camera_id: Uuid,
    /// Query start time
    pub start_time: DateTime<Utc>,
    /// Query end time
    pub end_time: DateTime<Utc>,
    /// Timeline segments
    pub segments: Vec<TimelineSegmentInfo>,
    /// Total duration with recordings (seconds)
    pub total_duration_secs: f64,
    /// Total gaps (seconds)
    pub total_gap_secs: f64,
}

/// Timeline segment with hot/cold information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineSegmentInfo {
    /// Segment start time
    pub start_time: DateTime<Utc>,
    /// Segment end time
    pub end_time: DateTime<Utc>,
    /// Duration in seconds
    pub duration_secs: f64,
    /// Size in bytes
    pub size_bytes: u64,
    /// Whether this segment is in cold storage
    pub is_cold: bool,
}

/// Retention configuration response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionConfigResponse {
    /// Hot storage quota in bytes
    pub hot_quota_bytes: u64,
    /// Cold storage quota in bytes
    pub cold_quota_bytes: u64,
    /// Max age in hot storage (seconds)
    pub hot_max_age_secs: u64,
    /// Max age in cold storage (seconds, 0 = unlimited)
    pub cold_max_age_secs: u64,
    /// Migration interval (seconds)
    pub migration_interval_secs: u64,
    /// Migration threshold (0.0-1.0)
    pub migration_threshold: f64,
}

/// Migration status response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationStatusResponse {
    /// Whether migration job is running
    pub is_running: bool,
    /// Last migration time
    pub last_run: Option<DateTime<Utc>>,
    /// Recordings migrated in last run
    pub last_migrated_count: u64,
    /// Bytes migrated in last run
    pub last_migrated_bytes: u64,
    /// Recordings deleted in last run
    pub last_deleted_count: u64,
    /// Next scheduled run
    pub next_run: Option<DateTime<Utc>>,
}

// ============================================================================
// Health Monitoring API Types (Phase 5)
// ============================================================================

/// Health state for a camera
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthState {
    /// Camera is online and responding
    Online,
    /// Camera is offline (not responding)
    Offline,
    /// Camera is degraded (responding with issues)
    Degraded,
    /// Health state unknown
    Unknown,
}

/// Camera health information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraHealthResponse {
    /// Camera ID
    pub camera_id: Uuid,
    /// Current health state
    pub state: HealthState,
    /// Last successful health check
    pub last_success: Option<DateTime<Utc>>,
    /// Last failed health check
    pub last_failure: Option<DateTime<Utc>>,
    /// Consecutive successful checks
    pub consecutive_successes: u32,
    /// Consecutive failed checks
    pub consecutive_failures: u32,
    /// Average response latency (ms)
    pub avg_latency_ms: Option<f64>,
}

/// Health check history entry
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheckEntry {
    /// Check timestamp
    pub timestamp: DateTime<Utc>,
    /// Whether check was successful
    pub success: bool,
    /// Response latency in ms
    pub latency_ms: Option<u32>,
    /// Error message if failed
    pub error: Option<String>,
}

/// Health history response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthHistoryResponse {
    /// Camera ID
    pub camera_id: Uuid,
    /// Health check history
    pub checks: Vec<HealthCheckEntry>,
    /// Total checks in range
    pub total_count: u64,
}

/// All cameras health summary
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AllHealthResponse {
    /// Per-camera health info
    pub cameras: Vec<CameraHealthResponse>,
    /// Total cameras online
    pub online_count: usize,
    /// Total cameras offline
    pub offline_count: usize,
    /// Total cameras degraded
    pub degraded_count: usize,
}

// ============================================================================
// Events/Timeline API Types (Phase 5)
// ============================================================================

/// Normalized event type
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Motion,
    MotionEnd,
    Audio,
    AudioEnd,
    DigitalInput,
    Tamper,
    LineCrossing,
    ObjectDetected,
    FaceDetected,
    VehicleDetected,
    PersonDetected,
    Unknown,
}

/// An event in the timeline
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEvent {
    /// Event ID
    pub id: Uuid,
    /// Camera ID
    pub camera_id: Uuid,
    /// Event type
    pub event_type: EventType,
    /// Event timestamp
    pub timestamp: DateTime<Utc>,
    /// Event end timestamp (for duration events)
    pub end_timestamp: Option<DateTime<Utc>>,
    /// Confidence score (0.0-1.0)
    pub confidence: Option<f64>,
    /// Bounding box [x, y, width, height] normalized 0-1
    pub bounding_box: Option<[f64; 4]>,
    /// Thumbnail URL (if available)
    pub thumbnail_url: Option<String>,
}

/// Events query parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsQuery {
    /// Filter by camera ID
    pub camera_id: Option<Uuid>,
    /// Start time filter
    pub start_time: Option<DateTime<Utc>>,
    /// End time filter
    pub end_time: Option<DateTime<Utc>>,
    /// Filter by event types
    pub event_types: Option<Vec<EventType>>,
    /// Maximum number of results
    #[serde(default = "default_events_limit")]
    pub limit: u32,
    /// Offset for pagination
    #[serde(default)]
    pub offset: u32,
}

fn default_events_limit() -> u32 {
    100
}

impl Default for EventsQuery {
    fn default() -> Self {
        Self {
            camera_id: None,
            start_time: None,
            end_time: None,
            event_types: None,
            limit: default_events_limit(),
            offset: 0,
        }
    }
}

/// Events query response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsResponse {
    /// Total events matching query
    pub total: u64,
    /// Events in this page
    pub events: Vec<TimelineEvent>,
}

/// Combined timeline view (recordings + events)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CombinedTimelineResponse {
    /// Camera ID
    pub camera_id: Uuid,
    /// Start time of view
    pub start_time: DateTime<Utc>,
    /// End time of view
    pub end_time: DateTime<Utc>,
    /// Recording segments
    pub recordings: Vec<TimelineSegmentInfo>,
    /// Events in range
    pub events: Vec<TimelineEvent>,
    /// Health state changes in range
    pub health_changes: Vec<HealthStateChange>,
}

/// Health state change event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthStateChange {
    /// Timestamp of change
    pub timestamp: DateTime<Utc>,
    /// Previous state
    pub previous_state: HealthState,
    /// New state
    pub new_state: HealthState,
}

// ============================================================================
// WebRTC Data Channel Protocol
// ============================================================================
//
// All communication after WebRTC connection uses data channel messages.
// Messages are JSON-encoded with a "type" discriminator.
//
// Flow:
//   1. Client connects via WebRTC signaling (HTTP POST /api/webrtc/offer)
//   2. Data channel "control" opens automatically
//   3. All further communication uses the data channel
//
// Message Types:
//   Client → Server: Commands (seek, play, pause, getTimeline, etc.)
//   Server → Client: Responses, position updates, timeline data, events

/// Message from client to server over data channel
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DataChannelCommand {
    // === Playback Control ===
    /// Start or resume playback
    #[serde(rename = "play")]
    Play {
        /// Playback speed (1.0 = normal, 2.0 = 2x, 0.5 = half speed)
        #[serde(default = "default_speed")]
        speed: f32,
    },

    /// Pause playback
    #[serde(rename = "pause")]
    Pause,

    /// Seek to a specific timestamp
    #[serde(rename = "seek")]
    Seek {
        /// Target timestamp (ISO 8601)
        timestamp: DateTime<Utc>,
        /// Direction for finding keyframe
        #[serde(default)]
        direction: Option<SeekDirection>,
    },

    /// Set playback speed without changing play/pause state
    #[serde(rename = "setSpeed")]
    SetSpeed {
        speed: f32,
    },

    /// Skip forward/backward by seconds
    #[serde(rename = "skip")]
    Skip {
        /// Seconds to skip (negative = backward)
        seconds: i32,
    },

    // === Timeline Queries ===
    /// Request timeline data for a time range
    #[serde(rename = "getTimeline")]
    GetTimeline {
        /// Start of range
        start_time: DateTime<Utc>,
        /// End of range
        end_time: DateTime<Utc>,
    },

    /// Request keyframe positions for seeking
    #[serde(rename = "getKeyframes")]
    GetKeyframes {
        /// Start of range
        start_time: DateTime<Utc>,
        /// End of range
        end_time: DateTime<Utc>,
    },

    /// Request events in a time range
    #[serde(rename = "getEvents")]
    GetEvents {
        /// Start of range
        start_time: DateTime<Utc>,
        /// End of range
        end_time: DateTime<Utc>,
        /// Filter by event types
        #[serde(default)]
        event_types: Option<Vec<String>>,
    },

    // === Mode Control ===
    /// Switch between live and replay mode
    #[serde(rename = "setMode")]
    SetMode {
        mode: PlaybackMode,
    },

    /// Switch stream (main/sub)
    #[serde(rename = "setStream")]
    SetStream {
        stream_type: StreamType,
    },

    // === Layer Control (Simulcast/Quality) ===
    /// Set video layer preference
    #[serde(rename = "setLayer")]
    SetLayer {
        /// Desired layer (high/medium/low/auto)
        layer: VideoLayer,
    },

    /// Set layer selection mode and preferences
    #[serde(rename = "setLayerPreference")]
    SetLayerPreference {
        /// Full preference settings
        #[serde(flatten)]
        preference: LayerPreference,
    },

    /// Notify server of UI context change (for adaptive mode)
    #[serde(rename = "setUiContext")]
    SetUiContext {
        /// Current UI context
        context: UiContext,
    },

    // === Focus Control (Multi-Camera) ===
    /// Request focus (upgrade to HD quality) for this camera
    #[serde(rename = "requestFocus")]
    RequestFocus,

    /// Release focus (downgrade back to low quality)
    #[serde(rename = "releaseFocus")]
    ReleaseFocus,

    /// Request current bandwidth budget info
    #[serde(rename = "getBandwidthBudget")]
    GetBandwidthBudget,

    // === Status ===
    /// Request current playback status
    #[serde(rename = "getStatus")]
    GetStatus,

    /// Ping for keepalive
    #[serde(rename = "ping")]
    Ping {
        /// Client timestamp for RTT calculation
        client_time: DateTime<Utc>,
    },
}

fn default_speed() -> f32 {
    1.0
}

/// Playback mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackMode {
    /// Live stream from camera
    Live,
    /// Replay from recordings
    Replay,
}

/// Message from server to client over data channel
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DataChannelMessage {
    // === Playback State ===
    /// Current playback position (sent periodically during playback)
    #[serde(rename = "position")]
    Position {
        /// Current timestamp being played
        timestamp: DateTime<Utc>,
        /// PTS in 90kHz ticks (for precise sync)
        pts: i64,
        /// Current playback mode
        mode: PlaybackMode,
        /// Whether currently playing
        is_playing: bool,
        /// Current playback speed
        speed: f32,
    },

    /// Playback state changed
    #[serde(rename = "stateChange")]
    StateChange {
        /// Previous state
        previous: PlaybackStateInfo,
        /// Current state
        current: PlaybackStateInfo,
    },

    /// Seek completed
    #[serde(rename = "seekComplete")]
    SeekComplete {
        /// Requested timestamp
        requested_timestamp: DateTime<Utc>,
        /// Actual timestamp (may differ due to keyframe alignment)
        actual_timestamp: DateTime<Utc>,
    },

    /// End of available recordings reached
    #[serde(rename = "endOfStream")]
    EndOfStream,

    /// Gap in recordings (no data available)
    #[serde(rename = "gap")]
    Gap {
        /// Start of gap
        gap_start: DateTime<Utc>,
        /// End of gap (where recordings resume)
        gap_end: Option<DateTime<Utc>>,
    },

    // === Timeline Data ===
    /// Timeline response
    #[serde(rename = "timeline")]
    Timeline {
        /// Start of range
        start_time: DateTime<Utc>,
        /// End of range
        end_time: DateTime<Utc>,
        /// Recording segments
        segments: Vec<TimelineSegmentInfo>,
        /// Earliest available recording
        earliest_available: Option<DateTime<Utc>>,
        /// Latest available recording  
        latest_available: Option<DateTime<Utc>>,
    },

    /// Keyframe positions response
    #[serde(rename = "keyframes")]
    Keyframes {
        /// Start of range
        start_time: DateTime<Utc>,
        /// End of range
        end_time: DateTime<Utc>,
        /// Keyframe timestamps
        keyframes: Vec<DateTime<Utc>>,
    },

    /// Events response
    #[serde(rename = "events")]
    Events {
        /// Start of range
        start_time: DateTime<Utc>,
        /// End of range
        end_time: DateTime<Utc>,
        /// Events
        events: Vec<TimelineEvent>,
    },

    // === Buffering ===
    /// Buffering state changed
    #[serde(rename = "buffering")]
    Buffering {
        /// Whether currently buffering
        is_buffering: bool,
        /// Buffered ranges (start, end pairs)
        buffered_ranges: Vec<(DateTime<Utc>, DateTime<Utc>)>,
    },

    // === Status ===
    /// Current status response
    #[serde(rename = "status")]
    Status(PlaybackStatus),

    /// Pong response
    #[serde(rename = "pong")]
    Pong {
        /// Client's original timestamp
        client_time: DateTime<Utc>,
        /// Server timestamp
        server_time: DateTime<Utc>,
    },

    // === Stream/Mode Changes ===
    /// Stream type changed (main/sub)
    #[serde(rename = "streamChanged")]
    StreamChanged {
        /// New stream type
        stream_type: StreamType,
        /// Previous stream type
        previous_stream_type: Option<StreamType>,
    },

    /// Mode changed (live/replay)
    #[serde(rename = "modeChanged")]
    ModeChanged {
        /// New mode
        mode: PlaybackMode,
        /// Previous mode
        previous_mode: Option<PlaybackMode>,
    },

    // === Layer Changes ===
    /// Video layer changed (server notification)
    #[serde(rename = "layerChanged")]
    LayerChanged(LayerChangeNotification),

    /// Current layer info
    #[serde(rename = "layerInfo")]
    LayerInfo {
        /// Current active layer
        active_layer: VideoLayer,
        /// Current selection mode
        mode: LayerSelectionMode,
        /// Available layers for this stream
        available_layers: Vec<VideoLayer>,
        /// Current stream type
        stream_type: StreamType,
        /// Estimated bitrate (kbps)
        estimated_bitrate_kbps: Option<u32>,
    },

    /// Bandwidth budget update (for multi-camera scenarios)
    #[serde(rename = "bandwidthBudget")]
    BandwidthBudget {
        /// Total estimated available bandwidth (kbps)
        total_kbps: u32,
        /// Currently used bandwidth (kbps)
        used_kbps: u32,
        /// Available for new streams (kbps)
        available_kbps: u32,
        /// Can afford a high-quality stream upgrade?
        can_upgrade: bool,
        /// Recommended action ("upgrade", "downgrade", "maintain")
        recommendation: String,
    },

    // === Focus Control ===
    /// Focus request granted
    #[serde(rename = "focusGranted")]
    FocusGranted {
        /// Camera that now has focus
        camera_id: Uuid,
        /// New quality layer
        layer: VideoLayer,
        /// Bitrate for focused stream (kbps)
        bitrate_kbps: u32,
    },

    /// Focus request denied
    #[serde(rename = "focusDenied")]
    FocusDenied {
        /// Camera that was denied focus
        camera_id: Uuid,
        /// Reason for denial
        reason: String,
        /// Available bandwidth (kbps)
        available_kbps: u32,
        /// Required bandwidth for upgrade (kbps)
        required_kbps: u32,
    },

    /// Focus was revoked (by bandwidth constraint or another camera)
    #[serde(rename = "focusRevoked")]
    FocusRevoked {
        /// Camera that lost focus
        camera_id: Uuid,
        /// Reason for revocation
        reason: String,
        /// New quality layer
        new_layer: VideoLayer,
    },

    // === Errors ===
    /// Error response
    #[serde(rename = "error")]
    Error {
        /// Error code
        code: String,
        /// Human-readable message
        message: String,
        /// Original command that caused the error (if applicable)
        command: Option<String>,
    },
}

/// Playback state info for state change notifications
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackStateInfo {
    pub mode: PlaybackMode,
    pub is_playing: bool,
    pub speed: f32,
    pub timestamp: Option<DateTime<Utc>>,
}

/// Complete playback status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackStatus {
    /// Camera ID
    pub camera_id: Uuid,
    /// Stream type
    pub stream_type: StreamType,
    /// Current mode
    pub mode: PlaybackMode,
    /// Whether playing
    pub is_playing: bool,
    /// Playback speed
    pub speed: f32,
    /// Current position (if in replay mode)
    pub current_timestamp: Option<DateTime<Utc>>,
    /// Video codec
    pub codec: Option<String>,
    /// Resolution
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Connection state
    pub connection_state: String,
    /// Buffered ranges
    pub buffered_ranges: Vec<(DateTime<Utc>, DateTime<Utc>)>,
    /// Current video layer
    pub active_layer: VideoLayer,
    /// Layer selection mode
    pub layer_mode: LayerSelectionMode,
    /// Available layers
    pub available_layers: Vec<VideoLayer>,
}
// ============================================================================
// Multi-Camera Replay Types
// ============================================================================
//
// Multi-camera replay allows synchronized playback across up to 9 cameras.
// Two viewing modes:
//   1. Grid Mode: All cameras show sub-streams for bandwidth efficiency
//   2. Focus Mode: Selected camera shows main/HD stream, others show sub-streams

/// Request to create a multi-camera replay session
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiReplayRequest {
    /// Camera IDs to include in replay (max 9)
    pub camera_ids: Vec<Uuid>,
    /// Start time for replay
    pub start_time: DateTime<Utc>,
    /// End time for replay (optional, defaults to now)
    pub end_time: Option<DateTime<Utc>>,
    /// Initial playback speed
    #[serde(default = "default_speed")]
    pub speed: f32,
    /// Whether to auto-play on start
    #[serde(default)]
    pub auto_play: bool,
}

/// Response for multi-camera replay session creation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiReplayResponse {
    /// Replay session ID
    pub session_id: Uuid,
    /// Camera info for each included camera
    pub cameras: Vec<ReplayCameraInfo>,
    /// Effective start time (aligned to available recordings)
    pub start_time: DateTime<Utc>,
    /// Effective end time
    pub end_time: DateTime<Utc>,
    /// Combined timeline showing when recordings are available
    pub timeline: MultiCameraTimeline,
}

/// Info about a camera in a multi-replay session
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayCameraInfo {
    /// Camera ID
    pub camera_id: Uuid,
    /// Camera name
    pub name: String,
    /// Slot index in grid (0-8)
    pub slot: u8,
    /// Whether this camera has recordings in the time range
    pub has_recordings: bool,
    /// Recording coverage for this camera
    pub segments: Vec<TimelineSegmentInfo>,
}

/// Combined timeline for multiple cameras
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiCameraTimeline {
    /// Start of timeline
    pub start_time: DateTime<Utc>,
    /// End of timeline
    pub end_time: DateTime<Utc>,
    /// Time spans where at least one camera has recording
    pub any_recording: Vec<(DateTime<Utc>, DateTime<Utc>)>,
    /// Time spans where ALL cameras have recording
    pub all_recording: Vec<(DateTime<Utc>, DateTime<Utc>)>,
    /// Gaps where no camera has recording
    pub gaps: Vec<(DateTime<Utc>, DateTime<Utc>)>,
}

/// Multi-replay session state
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiReplayState {
    /// Session ID
    pub session_id: Uuid,
    /// Current playback timestamp
    pub current_time: DateTime<Utc>,
    /// Whether playing
    pub is_playing: bool,
    /// Playback speed
    pub speed: f32,
    /// Focused camera (None = grid view)
    pub focused_camera: Option<Uuid>,
    /// Per-camera playback states
    pub camera_states: Vec<CameraReplayState>,
}

/// Replay state for a single camera in multi-replay
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraReplayState {
    /// Camera ID
    pub camera_id: Uuid,
    /// Whether currently streaming frames
    pub is_active: bool,
    /// Stream type currently being used
    pub stream_type: StreamType,
    /// Current buffered range
    pub buffered: Option<(DateTime<Utc>, DateTime<Utc>)>,
    /// Whether in a gap (no recording available)
    pub in_gap: bool,
}

/// Commands specific to multi-camera replay data channel
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MultiReplayCommand {
    /// Start or resume playback
    #[serde(rename = "play")]
    Play {
        #[serde(default = "default_speed")]
        speed: f32,
    },

    /// Pause playback
    #[serde(rename = "pause")]
    Pause,

    /// Seek to timestamp (all cameras sync to this time)
    #[serde(rename = "seek")]
    Seek {
        timestamp: DateTime<Utc>,
    },

    /// Set playback speed
    #[serde(rename = "setSpeed")]
    SetSpeed {
        speed: f32,
    },

    /// Skip forward/backward
    #[serde(rename = "skip")]
    Skip {
        /// Seconds to skip (negative = backward)
        seconds: i32,
    },

    /// Focus on a specific camera (switches to main stream)
    #[serde(rename = "focusCamera")]
    FocusCamera {
        camera_id: Uuid,
    },

    /// Exit focus mode (back to grid view with sub streams)
    #[serde(rename = "exitFocus")]
    ExitFocus,

    /// Request current state
    #[serde(rename = "getState")]
    GetState,

    /// Request timeline for all cameras
    #[serde(rename = "getTimeline")]
    GetTimeline {
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    },
}

/// Messages from server for multi-camera replay
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MultiReplayMessage {
    /// Current position update (sent periodically)
    #[serde(rename = "position")]
    Position {
        /// Current playback timestamp
        timestamp: DateTime<Utc>,
        /// Whether playing
        is_playing: bool,
        /// Current speed
        speed: f32,
    },

    /// Full state update
    #[serde(rename = "state")]
    State(MultiReplayState),

    /// Focus changed
    #[serde(rename = "focusChanged")]
    FocusChanged {
        /// Newly focused camera (None = grid view)
        camera_id: Option<Uuid>,
        /// Stream type for focused camera
        stream_type: StreamType,
    },

    /// Camera entered/exited gap
    #[serde(rename = "gapStatus")]
    GapStatus {
        camera_id: Uuid,
        in_gap: bool,
        /// Next recording timestamp (if in gap)
        next_recording: Option<DateTime<Utc>>,
    },

    /// Timeline data
    #[serde(rename = "timeline")]
    Timeline(MultiCameraTimeline),

    /// Seek completed
    #[serde(rename = "seekComplete")]
    SeekComplete {
        requested: DateTime<Utc>,
        actual: DateTime<Utc>,
    },

    /// Keyframe positions for seeking
    #[serde(rename = "keyframeTimeline")]
    KeyframeTimelineData {
        /// Keyframe timelines for each camera
        timelines: Vec<KeyframeTimeline>,
    },

    /// End of available recordings
    #[serde(rename = "endOfRecordings")]
    EndOfRecordings,

    /// Error
    #[serde(rename = "error")]
    Error {
        code: String,
        message: String,
    },
}