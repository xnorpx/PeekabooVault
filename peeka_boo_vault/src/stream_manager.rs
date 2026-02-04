//! Stream Manager - Scrypted-inspired RTSP connection management
//!
//! Maintains a single RTSP connection per camera stream and provides:
//! - ~10 second prebuffer with IDR alignment
//! - Consumer subscription API
//! - Codec detection (H.264/H.265)
//! - Keyframe interval tracking

// Allow dead code for now - these will be used once RTSP client is integrated
#![allow(dead_code)]

use bytes::Bytes;
use parking_lot::RwLock;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

/// Default prebuffer duration
#[allow(unused)]
const DEFAULT_PREBUFFER_DURATION: Duration = Duration::from_secs(10);

/// Maximum prebuffer size in bytes (100MB per stream)
const MAX_PREBUFFER_BYTES: usize = 100 * 1024 * 1024;

/// Chunk of stream data with metadata
#[derive(Debug, Clone)]
pub struct StreamChunk {
    /// Raw data (NAL units or other codec data)
    pub data: Bytes,
    /// Timestamp from RTP
    pub timestamp: u32,
    /// Whether this is a keyframe (IDR for H.264/H.265)
    pub is_keyframe: bool,
    /// Wall clock time when received
    pub received_at: Instant,
    /// Codec-specific data (e.g., SPS/PPS for H.264)
    pub codec_extra: Option<Bytes>,
}

/// Video codec detected from stream
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    H265,
    Unknown,
}

impl std::fmt::Display for VideoCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VideoCodec::H264 => write!(f, "H.264"),
            VideoCodec::H265 => write!(f, "H.265/HEVC"),
            VideoCodec::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Stream session state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Not connected
    Disconnected,
    /// Connecting to camera
    Connecting,
    /// Active and receiving data
    Active,
    /// Reconnecting after error
    Reconnecting,
    /// Stopped (user requested)
    Stopped,
}

/// Stream metadata detected from the connection
#[derive(Debug, Clone)]
pub struct StreamMetadata {
    pub codec: VideoCodec,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// SPS data for H.264
    pub sps: Option<Bytes>,
    /// PPS data for H.264
    pub pps: Option<Bytes>,
    /// VPS data for H.265
    pub vps: Option<Bytes>,
    /// Detected keyframe interval
    pub keyframe_interval: Option<Duration>,
    /// Average bitrate estimate (bytes/sec)
    pub bitrate_estimate: Option<u64>,
}

impl Default for StreamMetadata {
    fn default() -> Self {
        Self {
            codec: VideoCodec::Unknown,
            width: None,
            height: None,
            sps: None,
            pps: None,
            vps: None,
            keyframe_interval: None,
            bitrate_estimate: None,
        }
    }
}

/// Stream session managing a single RTSP connection
pub struct StreamSession {
    /// Camera ID
    pub camera_id: Uuid,
    /// Stream type (main/sub)
    pub stream_type: String,
    /// RTSP URL
    pub rtsp_url: String,
    /// Current state
    state: Arc<RwLock<SessionState>>,
    /// State change notifications
    state_tx: watch::Sender<SessionState>,
    /// Stream metadata
    metadata: Arc<RwLock<StreamMetadata>>,
    /// Prebuffer (ring buffer of recent chunks)
    prebuffer: Arc<RwLock<Prebuffer>>,
    /// Individual subscriber channels (removed when receiver drops)
    subscribers: Arc<RwLock<Vec<mpsc::Sender<StreamChunk>>>>,
    /// Session statistics
    stats: Arc<RwLock<SessionStats>>,
}

/// Prebuffer for instant playback
struct Prebuffer {
    chunks: VecDeque<StreamChunk>,
    total_bytes: usize,
    max_bytes: usize,
    /// Index of first keyframe in buffer (for clean joins)
    first_keyframe_idx: Option<usize>,
}

impl Prebuffer {
    fn new(max_bytes: usize) -> Self {
        Self {
            chunks: VecDeque::new(),
            total_bytes: 0,
            max_bytes,
            first_keyframe_idx: None,
        }
    }

    fn push(&mut self, chunk: StreamChunk) {
        let chunk_size = chunk.data.len();
        
        // Evict old data if over limit
        while self.total_bytes + chunk_size > self.max_bytes && !self.chunks.is_empty() {
            if let Some(old) = self.chunks.pop_front() {
                self.total_bytes -= old.data.len();
            }
            // Recalculate first keyframe index
            self.first_keyframe_idx = self.chunks.iter().position(|c| c.is_keyframe);
        }
        
        // Track keyframe position
        if chunk.is_keyframe && self.first_keyframe_idx.is_none() {
            self.first_keyframe_idx = Some(self.chunks.len());
        }
        
        self.total_bytes += chunk_size;
        self.chunks.push_back(chunk);
    }

    /// Get all chunks from the first keyframe onwards
    fn get_from_keyframe(&self) -> Vec<StreamChunk> {
        match self.first_keyframe_idx {
            Some(idx) => self.chunks.iter().skip(idx).cloned().collect(),
            None => Vec::new(),
        }
    }

    fn clear(&mut self) {
        self.chunks.clear();
        self.total_bytes = 0;
        self.first_keyframe_idx = None;
    }
}

/// Session statistics
#[derive(Debug, Clone, Default)]
pub struct SessionStats {
    /// Total bytes received
    pub bytes_received: u64,
    /// Total frames received
    pub frames_received: u64,
    /// Keyframes received
    pub keyframes_received: u64,
    /// Last keyframe time
    pub last_keyframe_at: Option<Instant>,
    /// Connection start time
    pub connected_at: Option<Instant>,
    /// Number of reconnections
    pub reconnect_count: u32,
}

impl StreamSession {
    /// Create a new stream session
    pub fn new(camera_id: Uuid, stream_type: String, rtsp_url: String) -> Self {
        let (state_tx, _) = watch::channel(SessionState::Disconnected);

        Self {
            camera_id,
            stream_type,
            rtsp_url,
            state: Arc::new(RwLock::new(SessionState::Disconnected)),
            state_tx,
            metadata: Arc::new(RwLock::new(StreamMetadata::default())),
            prebuffer: Arc::new(RwLock::new(Prebuffer::new(MAX_PREBUFFER_BYTES))),
            subscribers: Arc::new(RwLock::new(Vec::new())),
            stats: Arc::new(RwLock::new(SessionStats::default())),
        }
    }

    /// Get current state
    pub fn state(&self) -> SessionState {
        *self.state.read()
    }

    /// Subscribe to state changes
    pub fn subscribe_state(&self) -> watch::Receiver<SessionState> {
        self.state_tx.subscribe()
    }

    /// Get stream metadata
    pub fn metadata(&self) -> StreamMetadata {
        self.metadata.read().clone()
    }

    /// Get current statistics
    pub fn stats(&self) -> SessionStats {
        self.stats.read().clone()
    }

    /// Subscribe to live stream data
    /// Returns prebuffered data first, then a receiver for live data
    /// Channel capacity of 256 frames provides ~8 seconds buffer at 30fps
    pub fn subscribe(&self) -> (Vec<StreamChunk>, mpsc::Receiver<StreamChunk>) {
        let prebuffered = self.prebuffer.read().get_from_keyframe();
        let (tx, rx) = mpsc::channel(256);
        self.subscribers.write().push(tx);
        (prebuffered, rx)
    }

    /// Get current subscriber count
    pub fn subscriber_count(&self) -> usize {
        self.subscribers.read().len()
    }

    /// Update session state
    fn set_state(&self, new_state: SessionState) {
        *self.state.write() = new_state;
        let _ = self.state_tx.send(new_state);
    }

    /// Process received chunk
    /// 
    /// Data flow (minimizing copies):
    /// 1. Clone to WebRTC subscribers (Bytes clone = Arc increment, O(1))
    /// 2. [Future: clone to other targets like HLS, recording, etc.]
    /// 3. Finally hand ownership to prebuffer/storage (no clone)
    pub(crate) fn on_chunk(&self, chunk: StreamChunk) {
        // Update stats first (just reads chunk.data.len())
        {
            let mut stats = self.stats.write();
            stats.bytes_received += chunk.data.len() as u64;
            stats.frames_received += 1;
            if chunk.is_keyframe {
                stats.keyframes_received += 1;
                stats.last_keyframe_at = Some(chunk.received_at);
            }
        }

        // === Phase 1: WebRTC subscribers ===
        // Clone to all WebRTC viewers (Bytes clone = Arc increment, O(1))
        // Remove subscribers with closed/full channels
        {
            let mut subs = self.subscribers.write();
            subs.retain(|tx| tx.try_send(chunk.clone()).is_ok());
        }

        // === Phase 2: Optional targets (future) ===
        // HLS muxer, secondary recording, analytics, etc.
        // Each would get chunk.clone()

        // === Phase 3: Storage (takes ownership, releases Bytes) ===
        // Prebuffer is the final destination - no clone needed
        self.prebuffer.write().push(chunk);
    }

    /// Update stream metadata
    pub(crate) fn set_metadata(&self, metadata: StreamMetadata) {
        *self.metadata.write() = metadata;
    }

    /// Mark session as connected
    pub(crate) fn on_connected(&self) {
        self.set_state(SessionState::Active);
        self.stats.write().connected_at = Some(Instant::now());
    }

    /// Mark session as disconnected with reconnect
    pub(crate) fn on_disconnected(&self, will_reconnect: bool) {
        if will_reconnect {
            self.set_state(SessionState::Reconnecting);
            self.stats.write().reconnect_count += 1;
        } else {
            self.set_state(SessionState::Disconnected);
        }
        self.prebuffer.write().clear();
    }
}

/// Stream Manager - manages all camera stream sessions
pub struct StreamManager {
    sessions: RwLock<HashMap<(Uuid, String), Arc<StreamSession>>>,
}

impl StreamManager {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }

    /// Get or create a stream session
    pub fn get_or_create_session(
        &self,
        camera_id: Uuid,
        stream_type: &str,
        rtsp_url: &str,
    ) -> Arc<StreamSession> {
        let key = (camera_id, stream_type.to_string());
        
        // Check if exists
        if let Some(session) = self.sessions.read().get(&key) {
            return session.clone();
        }

        // Create new session
        let session = Arc::new(StreamSession::new(
            camera_id,
            stream_type.to_string(),
            rtsp_url.to_string(),
        ));
        
        self.sessions.write().insert(key, session.clone());
        session
    }

    /// Get an existing session
    pub fn get_session(&self, camera_id: Uuid, stream_type: &str) -> Option<Arc<StreamSession>> {
        let key = (camera_id, stream_type.to_string());
        self.sessions.read().get(&key).cloned()
    }

    /// Remove a session
    pub fn remove_session(&self, camera_id: Uuid, stream_type: &str) {
        let key = (camera_id, stream_type.to_string());
        self.sessions.write().remove(&key);
    }

    /// List all active sessions
    pub fn list_sessions(&self) -> Vec<(Uuid, String, SessionState)> {
        self.sessions
            .read()
            .iter()
            .map(|((id, stype), session)| (*id, stype.clone(), session.state()))
            .collect()
    }
}

impl Default for StreamManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prebuffer() {
        let mut prebuffer = Prebuffer::new(1024);
        
        // Add non-keyframe
        prebuffer.push(StreamChunk {
            data: Bytes::from(vec![0u8; 100]),
            timestamp: 0,
            is_keyframe: false,
            received_at: Instant::now(),
            codec_extra: None,
        });
        
        // Should be empty since no keyframe
        assert!(prebuffer.get_from_keyframe().is_empty());
        
        // Add keyframe
        prebuffer.push(StreamChunk {
            data: Bytes::from(vec![1u8; 100]),
            timestamp: 1,
            is_keyframe: true,
            received_at: Instant::now(),
            codec_extra: None,
        });
        
        // Should have one chunk
        let chunks = prebuffer.get_from_keyframe();
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].is_keyframe);
    }

    #[test]
    fn test_stream_manager() {
        let manager = StreamManager::new();
        let camera_id = Uuid::new_v4();
        
        let session1 = manager.get_or_create_session(camera_id, "main", "rtsp://test");
        let session2 = manager.get_or_create_session(camera_id, "main", "rtsp://test");
        
        // Should be same session
        assert!(Arc::ptr_eq(&session1, &session2));
        
        // Different stream type should be different session
        let session3 = manager.get_or_create_session(camera_id, "sub", "rtsp://test2");
        assert!(!Arc::ptr_eq(&session1, &session3));
    }
}
