//! Replay Coordinator for Multi-Camera Synchronized Playback
//!
//! Manages synchronized replay of up to 9 cameras through a SINGLE WebRTC session
//! with multiple video tracks (one per camera, each with high/low simulcast layers).
//!
//! Architecture:
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │  Single RTCPeerConnection                                │
//! │  ┌────────┐ ┌────────┐ ┌────────┐         ┌────────┐   │
//! │  │Track 0 │ │Track 1 │ │Track 2 │   ...   │Track 8 │   │
//! │  │ cam 1  │ │ cam 2  │ │ cam 3  │         │ cam 9  │   │
//! │  │hi + lo │ │hi + lo │ │hi + lo │         │hi + lo │   │
//! │  └────────┘ └────────┘ └────────┘         └────────┘   │
//! │                                                          │
//! │  ┌──────────────────────────────────────────────────┐  │
//! │  │  Single Data Channel - all commands               │  │
//! │  └──────────────────────────────────────────────────┘  │
//! └─────────────────────────────────────────────────────────┘
//!                           │
//!                           ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │  ReplayCoordinator                                       │
//! │  - playhead: single timestamp for ALL cameras           │
//! │  - batch DB query for all cameras at playhead           │
//! │  - distribute frames to respective tracks               │
//! │  - focus mode: upgrade one track to high layer          │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! Key benefits:
//! - Single WebRTC session instead of N sessions
//! - Guaranteed frame-perfect sync across all cameras
//! - Single batch DB query per tick (not N queries)
//! - Efficient bandwidth: all cameras on sub-layer by default
//! - Focus mode: upgrade one camera to high-layer

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::api::{MultiReplayCommand, MultiReplayMessage, MultiReplayState, CameraReplayState, StreamType};

/// Maximum number of cameras in a multi-replay session
pub const MAX_CAMERAS: usize = 9;

/// Simulcast layer for a camera track
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SimulcastLayer {
    /// Low quality (sub-stream) - default for grid view
    #[default]
    Low,
    /// High quality (main-stream) - for focused camera
    High,
}

/// State for a single camera track in the replay session
#[derive(Debug, Clone)]
pub struct CameraTrackState {
    /// Camera ID
    pub camera_id: Uuid,
    /// Track index (0-8)
    pub track_index: usize,
    /// Current simulcast layer being sent
    pub active_layer: SimulcastLayer,
    /// Whether this camera has recording at current playhead
    pub has_recording: bool,
    /// Last frame timestamp sent
    pub last_frame_time: Option<DateTime<Utc>>,
    /// Stream ID for main stream
    pub main_stream_id: Option<i64>,
    /// Stream ID for sub stream  
    pub sub_stream_id: Option<i64>,
}

/// Playback state
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum PlaybackState {
    /// Paused
    #[default]
    Paused,
    /// Playing at specified speed
    Playing { speed: f32 },
    /// Seeking to a timestamp
    Seeking,
    /// Buffering (waiting for data)
    Buffering,
}

/// Configuration for creating a replay coordinator
#[derive(Debug, Clone)]
pub struct ReplayCoordinatorConfig {
    /// Camera IDs to include (order determines track index)
    pub camera_ids: Vec<Uuid>,
    /// Start time of replay range
    pub start_time: DateTime<Utc>,
    /// End time of replay range
    pub end_time: DateTime<Utc>,
    /// Initial playback position
    pub initial_position: DateTime<Utc>,
    /// Whether to auto-play on start
    pub auto_play: bool,
    /// Initial playback speed
    pub initial_speed: f32,
}

/// Frame data for a single camera at a specific timestamp
#[derive(Debug, Clone)]
pub struct CameraFrame {
    /// Camera ID
    pub camera_id: Uuid,
    /// Frame timestamp
    pub timestamp: DateTime<Utc>,
    /// Frame data (H.264/H.265 NAL units)
    pub data: Vec<u8>,
    /// Whether this is a keyframe
    pub is_keyframe: bool,
    /// PTS in 90kHz ticks
    pub pts: i64,
    /// Which layer this frame is for (high or low quality)
    pub layer: SimulcastLayer,
}

/// Coordinator for multi-camera synchronized replay
/// 
/// This is the core component that manages:
/// - A single playhead for all cameras
/// - Batch frame retrieval from database
/// - Distribution to WebRTC tracks
pub struct ReplayCoordinator {
    /// Session ID
    pub session_id: Uuid,
    /// Configuration
    config: ReplayCoordinatorConfig,
    /// Current playhead position
    playhead: RwLock<DateTime<Utc>>,
    /// Current playback state
    playback_state: RwLock<PlaybackState>,
    /// Per-camera track states
    track_states: RwLock<Vec<CameraTrackState>>,
    /// Index of focused camera (None = grid view, all cameras low quality)
    focused_track: RwLock<Option<usize>>,
    /// Channel to send messages to client
    client_tx: mpsc::UnboundedSender<MultiReplayMessage>,
    /// Shutdown signal
    shutdown_tx: broadcast::Sender<()>,
}

impl ReplayCoordinator {
    /// Create a new replay coordinator
    pub fn new(
        config: ReplayCoordinatorConfig,
        client_tx: mpsc::UnboundedSender<MultiReplayMessage>,
    ) -> Arc<Self> {
        assert!(config.camera_ids.len() <= MAX_CAMERAS, "Max {} cameras", MAX_CAMERAS);
        
        let session_id = Uuid::new_v4();
        let (shutdown_tx, _) = broadcast::channel(1);
        
        // Initialize track states
        let track_states: Vec<CameraTrackState> = config.camera_ids
            .iter()
            .enumerate()
            .map(|(idx, &camera_id)| CameraTrackState {
                camera_id,
                track_index: idx,
                active_layer: SimulcastLayer::Low, // Default to low quality
                has_recording: true, // Will be updated when querying
                last_frame_time: None,
                main_stream_id: None,
                sub_stream_id: None,
            })
            .collect();
        
        info!(
            session_id = %session_id,
            camera_count = config.camera_ids.len(),
            start = %config.start_time,
            end = %config.end_time,
            "Created multi-camera replay coordinator"
        );
        
        Arc::new(Self {
            session_id,
            playhead: RwLock::new(config.initial_position),
            playback_state: RwLock::new(if config.auto_play {
                PlaybackState::Playing { speed: config.initial_speed }
            } else {
                PlaybackState::Paused
            }),
            track_states: RwLock::new(track_states),
            focused_track: RwLock::new(None),
            client_tx,
            shutdown_tx,
            config,
        })
    }
    
    /// Get current playhead position
    pub fn playhead(&self) -> DateTime<Utc> {
        *self.playhead.read()
    }
    
    /// Get current playback state
    pub fn playback_state(&self) -> PlaybackState {
        *self.playback_state.read()
    }
    
    /// Get number of camera tracks
    pub fn track_count(&self) -> usize {
        self.track_states.read().len()
    }
    
    /// Get camera ID for a track index
    pub fn camera_id_for_track(&self, track_index: usize) -> Option<Uuid> {
        self.track_states.read().get(track_index).map(|t| t.camera_id)
    }
    
    /// Get track index for a camera ID
    pub fn track_index_for_camera(&self, camera_id: Uuid) -> Option<usize> {
        self.track_states.read()
            .iter()
            .find(|t| t.camera_id == camera_id)
            .map(|t| t.track_index)
    }
    
    /// Get which layer should be active for each track
    /// Returns: Vec of (track_index, camera_id, layer)
    pub fn get_active_layers(&self) -> Vec<(usize, Uuid, SimulcastLayer)> {
        let tracks = self.track_states.read();
        let focused = *self.focused_track.read();
        
        tracks.iter().map(|track| {
            let layer = if Some(track.track_index) == focused {
                SimulcastLayer::High
            } else {
                SimulcastLayer::Low
            };
            (track.track_index, track.camera_id, layer)
        }).collect()
    }
    
    /// Handle a command from the client
    pub async fn handle_command(
        &self,
        cmd: MultiReplayCommand,
        frame_store: Option<&crate::frame_store::FrameStore>,
    ) {
        match cmd {
            MultiReplayCommand::Play { speed } => {
                self.play(speed);
            }
            MultiReplayCommand::Pause => {
                self.pause();
            }
            MultiReplayCommand::Seek { timestamp } => {
                self.seek(timestamp, frame_store).await;
            }
            MultiReplayCommand::SetSpeed { speed } => {
                self.set_speed(speed);
            }
            MultiReplayCommand::Skip { seconds } => {
                self.skip(seconds, frame_store).await;
            }
            MultiReplayCommand::FocusCamera { camera_id } => {
                self.focus_camera(camera_id);
            }
            MultiReplayCommand::ExitFocus => {
                self.exit_focus();
            }
            MultiReplayCommand::GetState => {
                self.send_state();
            }
            MultiReplayCommand::GetTimeline { start_time, end_time } => {
                // Query timeline with frame store if available
                if let Some(store) = frame_store {
                    self.query_and_send_timeline(store, start_time, end_time).await;
                } else {
                    warn!(
                        session_id = %self.session_id,
                        "Timeline request without frame_store"
                    );
                }
            }
        }
    }
    
    /// Start or resume playback
    pub fn play(&self, speed: f32) {
        let speed = speed.clamp(0.1, 16.0);
        *self.playback_state.write() = PlaybackState::Playing { speed };
        
        info!(session_id = %self.session_id, speed, "Playback started");
        
        self.send_position_update();
    }
    
    /// Pause playback
    pub fn pause(&self) {
        *self.playback_state.write() = PlaybackState::Paused;
        
        info!(session_id = %self.session_id, "Playback paused");
        
        self.send_position_update();
    }
    
    /// Set playback speed (without changing play/pause state)
    pub fn set_speed(&self, speed: f32) {
        let speed = speed.clamp(0.1, 16.0);
        let mut state = self.playback_state.write();
        if let PlaybackState::Playing { .. } = *state {
            *state = PlaybackState::Playing { speed };
        }
        
        debug!(session_id = %self.session_id, speed, "Speed changed");
    }
    
    /// Seek to a specific timestamp
    ///
    /// If frame_store is provided, this will find the nearest keyframe for all cameras
    /// and snap the playhead to that position for clean decoder state.
    pub async fn seek(
        &self,
        timestamp: DateTime<Utc>,
        frame_store: Option<&crate::frame_store::FrameStore>,
    ) {
        use crate::frame_store::SeekDirection as FrameSeekDirection;

        // Clamp to valid range
        let requested_timestamp = timestamp
            .max(self.config.start_time)
            .min(self.config.end_time);

        *self.playback_state.write() = PlaybackState::Seeking;

        info!(
            session_id = %self.session_id,
            requested = %requested_timestamp,
            "Seeking to timestamp"
        );

        // Find nearest keyframes for all cameras
        let actual_timestamp = if let Some(store) = frame_store {
            // Clone track states to avoid holding lock across await
            let track_states: Vec<CameraTrackState> = self.track_states.read().clone();
            let mut earliest_keyframe: Option<DateTime<Utc>> = None;

            // For each camera, find the nearest keyframe at or before the requested time
            for track in track_states.iter() {
                // Try both main and sub streams to find a keyframe
                let streams_to_try = [
                    ("main", track.main_stream_id),
                    ("sub", track.sub_stream_id),
                ];

                for (stream_type, _stream_id) in streams_to_try.iter() {
                    match store.find_keyframe(
                        track.camera_id,
                        stream_type,
                        requested_timestamp,
                        FrameSeekDirection::Backward,
                    ).await {
                        Ok(Some(keyframe)) => {
                            let keyframe_time = keyframe.timestamp;

                            debug!(
                                camera_id = %track.camera_id,
                                stream_type = %stream_type,
                                keyframe_time = %keyframe_time,
                                "Found keyframe for camera"
                            );

                            // Track the earliest keyframe across all cameras
                            // We want all cameras to start from a position where they all have keyframes
                            earliest_keyframe = Some(match earliest_keyframe {
                                None => keyframe_time,
                                Some(existing) => existing.min(keyframe_time),
                            });

                            break; // Found a keyframe for this camera, no need to check other streams
                        }
                        Ok(None) => {
                            debug!(
                                camera_id = %track.camera_id,
                                stream_type = %stream_type,
                                "No keyframe found for camera"
                            );
                        }
                        Err(e) => {
                            warn!(
                                camera_id = %track.camera_id,
                                stream_type = %stream_type,
                                error = %e,
                                "Error finding keyframe"
                            );
                        }
                    }
                }
            }

            // Use the earliest keyframe found, or fall back to requested timestamp
            let actual = earliest_keyframe.unwrap_or(requested_timestamp);

            info!(
                session_id = %self.session_id,
                requested = %requested_timestamp,
                actual = %actual,
                "Keyframe seek complete"
            );

            actual
        } else {
            // No frame store provided, just use requested timestamp
            warn!(
                session_id = %self.session_id,
                "Seeking without frame_store - cannot snap to keyframes"
            );
            requested_timestamp
        };

        // Update playhead to the actual keyframe position
        *self.playhead.write() = actual_timestamp;

        // Restore previous playback state
        *self.playback_state.write() = PlaybackState::Paused;

        // Notify client with both requested and actual positions
        let _ = self.client_tx.send(MultiReplayMessage::SeekComplete {
            requested: requested_timestamp,
            actual: actual_timestamp,
        });

        self.send_position_update();
    }
    
    /// Skip forward or backward by seconds
    pub async fn skip(
        &self,
        seconds: i32,
        frame_store: Option<&crate::frame_store::FrameStore>,
    ) {
        let current = self.playhead();
        let new_time = if seconds >= 0 {
            current + chrono::Duration::seconds(seconds as i64)
        } else {
            current - chrono::Duration::seconds((-seconds) as i64)
        };

        self.seek(new_time, frame_store).await;
    }

    /// Query and send timeline with keyframe positions for all cameras
    async fn query_and_send_timeline(
        &self,
        frame_store: &crate::frame_store::FrameStore,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) {
        use crate::api::KeyframeTimeline;

        // Clone track states to avoid holding lock across await
        let track_states: Vec<CameraTrackState> = self.track_states.read().clone();
        let mut timelines = Vec::new();

        info!(
            session_id = %self.session_id,
            start = %start_time,
            end = %end_time,
            camera_count = track_states.len(),
            "Querying timeline for all cameras"
        );

        // Query timeline for each camera
        for track in track_states.iter() {
            // Try both main and sub streams, prefer main
            let streams_to_try = [
                ("main", track.main_stream_id),
                ("sub", track.sub_stream_id),
            ];

            for (stream_type, _stream_id) in streams_to_try.iter() {
                match frame_store.get_timeline(
                    track.camera_id,
                    stream_type,
                    start_time,
                    end_time,
                ).await {
                    Ok(keyframes) if !keyframes.is_empty() => {
                        debug!(
                            camera_id = %track.camera_id,
                            stream_type = %stream_type,
                            keyframe_count = keyframes.len(),
                            "Got timeline for camera"
                        );

                        timelines.push(KeyframeTimeline {
                            camera_id: track.camera_id,
                            stream_type: match *stream_type {
                                "main" => crate::api::StreamType::Main,
                                "sub" => crate::api::StreamType::Sub,
                                _ => crate::api::StreamType::Main,
                            },
                            start_time,
                            end_time,
                            keyframes,
                        });

                        break; // Found keyframes for this camera, move to next
                    }
                    Ok(_) => {
                        debug!(
                            camera_id = %track.camera_id,
                            stream_type = %stream_type,
                            "No keyframes found for camera in range"
                        );
                    }
                    Err(e) => {
                        warn!(
                            camera_id = %track.camera_id,
                            stream_type = %stream_type,
                            error = %e,
                            "Error querying timeline"
                        );
                    }
                }
            }
        }

        info!(
            session_id = %self.session_id,
            timeline_count = timelines.len(),
            "Timeline query complete"
        );

        // Send timeline to client
        let _ = self.client_tx.send(MultiReplayMessage::KeyframeTimelineData { timelines });
    }

    /// Focus on a specific camera (upgrade to high quality)
    pub fn focus_camera(&self, camera_id: Uuid) {
        let track_index = self.track_index_for_camera(camera_id);
        
        if let Some(idx) = track_index {
            let _previous_focused = *self.focused_track.read();
            *self.focused_track.write() = Some(idx);
            
            // Update track states
            {
                let mut tracks = self.track_states.write();
                for track in tracks.iter_mut() {
                    track.active_layer = if track.track_index == idx {
                        SimulcastLayer::High
                    } else {
                        SimulcastLayer::Low
                    };
                }
            }
            
            info!(
                session_id = %self.session_id,
                camera_id = %camera_id,
                track_index = idx,
                "Camera focused - upgrading to high quality"
            );
            
            // Notify client
            let _ = self.client_tx.send(MultiReplayMessage::FocusChanged {
                camera_id: Some(camera_id),
                stream_type: StreamType::Main,
            });
            
        } else {
            warn!(session_id = %self.session_id, %camera_id, "Focus requested for unknown camera");
        }
    }
    
    /// Exit focus mode (all cameras back to low quality)
    pub fn exit_focus(&self) {
        let _previous_focused = self.focused_track.write().take();
        
        // Reset all tracks to low quality
        {
            let mut tracks = self.track_states.write();
            for track in tracks.iter_mut() {
                track.active_layer = SimulcastLayer::Low;
            }
        }
        
        info!(session_id = %self.session_id, "Focus mode exited - all cameras low quality");
        
        // Notify client
        let _ = self.client_tx.send(MultiReplayMessage::FocusChanged {
            camera_id: None,
            stream_type: StreamType::Sub,
        });
    }
    
    /// Advance playhead by elapsed time (called from playback loop)
    pub fn advance_playhead(&self, elapsed_ms: u64) -> Option<DateTime<Utc>> {
        let state = *self.playback_state.read();
        
        if let PlaybackState::Playing { speed } = state {
            let advance_ms = (elapsed_ms as f32 * speed) as i64;
            let mut playhead = self.playhead.write();
            
            let new_time = *playhead + chrono::Duration::milliseconds(advance_ms);
            
            if new_time >= self.config.end_time {
                // Reached end
                *playhead = self.config.end_time;
                drop(playhead);
                
                *self.playback_state.write() = PlaybackState::Paused;
                let _ = self.client_tx.send(MultiReplayMessage::EndOfRecordings);
                
                return None;
            }
            
            *playhead = new_time;
            Some(new_time)
        } else {
            None
        }
    }
    
    /// Get current state for all cameras
    pub fn get_state(&self) -> MultiReplayState {
        let tracks = self.track_states.read();
        let playhead = *self.playhead.read();
        let playback = *self.playback_state.read();
        let focused = *self.focused_track.read();
        
        let (is_playing, speed) = match playback {
            PlaybackState::Playing { speed } => (true, speed),
            _ => (false, 1.0),
        };
        
        let camera_states: Vec<CameraReplayState> = tracks.iter().map(|track| {
            CameraReplayState {
                camera_id: track.camera_id,
                is_active: track.has_recording,
                stream_type: match track.active_layer {
                    SimulcastLayer::High => StreamType::Main,
                    SimulcastLayer::Low => StreamType::Sub,
                },
                buffered: None, // TODO: Track buffered ranges
                in_gap: !track.has_recording,
            }
        }).collect();
        
        MultiReplayState {
            session_id: self.session_id,
            current_time: playhead,
            is_playing,
            speed,
            focused_camera: focused.and_then(|idx| tracks.get(idx).map(|t| t.camera_id)),
            camera_states,
        }
    }
    
    /// Send current state to client
    fn send_state(&self) {
        let state = self.get_state();
        let _ = self.client_tx.send(MultiReplayMessage::State(state));
    }
    
    /// Send position update to client
    fn send_position_update(&self) {
        let playhead = *self.playhead.read();
        let playback = *self.playback_state.read();
        
        let (is_playing, speed) = match playback {
            PlaybackState::Playing { speed } => (true, speed),
            _ => (false, 1.0),
        };
        
        let _ = self.client_tx.send(MultiReplayMessage::Position {
            timestamp: playhead,
            is_playing,
            speed,
        });
    }
    
    /// Get camera IDs that need frames at current playhead
    /// Returns: Vec of (track_index, camera_id, layer)
    pub fn get_frame_requests(&self) -> Vec<(usize, Uuid, SimulcastLayer)> {
        self.get_active_layers()
    }
    
    /// Update track state after receiving frames
    pub fn update_track_frame_time(&self, track_index: usize, timestamp: DateTime<Utc>) {
        let mut tracks = self.track_states.write();
        if let Some(track) = tracks.get_mut(track_index) {
            track.last_frame_time = Some(timestamp);
            track.has_recording = true;
        }
    }
    
    /// Mark a track as being in a gap (no recording)
    pub fn mark_track_gap(&self, track_index: usize, in_gap: bool) {
        let mut tracks = self.track_states.write();
        if let Some(track) = tracks.get_mut(track_index) {
            let was_in_gap = !track.has_recording;
            let now_in_gap = in_gap;

            // Only update and notify if state changed
            if was_in_gap != now_in_gap {
                track.has_recording = !in_gap;

                // Notify client of state change
                let _ = self.client_tx.send(MultiReplayMessage::GapStatus {
                    camera_id: track.camera_id,
                    in_gap,
                    next_recording: None, // TODO: Find next recording
                });

                if in_gap {
                    debug!(
                        session_id = %self.session_id,
                        camera_id = %track.camera_id,
                        track_index = track_index,
                        "Track entered gap (no recording)"
                    );
                } else {
                    debug!(
                        session_id = %self.session_id,
                        camera_id = %track.camera_id,
                        track_index = track_index,
                        "Track exited gap (recording resumed)"
                    );
                }
            }
        }
    }
    
    /// Get shutdown receiver
    pub fn shutdown_rx(&self) -> broadcast::Receiver<()> {
        self.shutdown_tx.subscribe()
    }
    
    /// Shutdown the coordinator
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
        info!(session_id = %self.session_id, "Replay coordinator shutdown");
    }
}

impl Drop for ReplayCoordinator {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    
    fn create_test_coordinator() -> (Arc<ReplayCoordinator>, mpsc::UnboundedReceiver<MultiReplayMessage>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let now = Utc::now();
        
        let config = ReplayCoordinatorConfig {
            camera_ids: vec![Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()],
            start_time: now - Duration::hours(1),
            end_time: now,
            initial_position: now - Duration::minutes(30),
            auto_play: false,
            initial_speed: 1.0,
        };
        
        let coordinator = ReplayCoordinator::new(config, tx);
        (coordinator, rx)
    }
    
    #[test]
    fn test_coordinator_creation() {
        let (coordinator, _rx) = create_test_coordinator();
        
        assert_eq!(coordinator.track_count(), 3);
        assert!(matches!(coordinator.playback_state(), PlaybackState::Paused));
    }
    
    #[test]
    fn test_play_pause() {
        let (coordinator, mut rx) = create_test_coordinator();
        
        coordinator.play(1.0);
        assert!(matches!(coordinator.playback_state(), PlaybackState::Playing { speed: 1.0 }));
        
        // Should have sent position update
        let msg = rx.try_recv().unwrap();
        assert!(matches!(msg, MultiReplayMessage::Position { is_playing: true, .. }));
        
        coordinator.pause();
        assert!(matches!(coordinator.playback_state(), PlaybackState::Paused));
    }
    
    #[test]
    fn test_focus_camera() {
        let (coordinator, mut rx) = create_test_coordinator();
        
        let camera_id = coordinator.camera_id_for_track(1).unwrap();
        coordinator.focus_camera(camera_id);
        
        // Check focused track
        let layers = coordinator.get_active_layers();
        assert_eq!(layers[0].2, SimulcastLayer::Low);
        assert_eq!(layers[1].2, SimulcastLayer::High); // Focused
        assert_eq!(layers[2].2, SimulcastLayer::Low);
        
        // Should have sent focus changed message
        let msg = rx.try_recv().unwrap();
        assert!(matches!(msg, MultiReplayMessage::FocusChanged { camera_id: Some(_), .. }));
    }
    
    #[test]
    fn test_exit_focus() {
        let (coordinator, mut rx) = create_test_coordinator();
        
        // Focus then exit
        let camera_id = coordinator.camera_id_for_track(1).unwrap();
        coordinator.focus_camera(camera_id);
        let _ = rx.try_recv(); // Consume focus message
        
        coordinator.exit_focus();
        
        // All should be low quality
        let layers = coordinator.get_active_layers();
        assert!(layers.iter().all(|(_, _, layer)| *layer == SimulcastLayer::Low));
        
        // Should have sent exit focus message
        let msg = rx.try_recv().unwrap();
        assert!(matches!(msg, MultiReplayMessage::FocusChanged { camera_id: None, .. }));
    }
    
    #[test]
    fn test_advance_playhead() {
        let (coordinator, _rx) = create_test_coordinator();
        
        coordinator.play(2.0); // 2x speed
        
        let before = coordinator.playhead();
        let result = coordinator.advance_playhead(100); // 100ms
        
        assert!(result.is_some());
        let after = coordinator.playhead();
        
        // Should have advanced by 200ms (100ms * 2x speed)
        let diff = after - before;
        assert_eq!(diff.num_milliseconds(), 200);
    }
    
    #[test]
    fn test_get_state() {
        let (coordinator, _rx) = create_test_coordinator();
        
        coordinator.play(1.5);
        
        let state = coordinator.get_state();
        
        assert_eq!(state.session_id, coordinator.session_id);
        assert!(state.is_playing);
        assert_eq!(state.speed, 1.5);
        assert_eq!(state.camera_states.len(), 3);
        assert!(state.focused_camera.is_none());
    }
}
