//! Stream Router - Routes camera streams to WebRTC clients based on layer selection
//!
//! This module handles the server-side logic for:
//! - Tracking which clients want which stream quality
//! - Routing main/sub stream data to appropriate clients
//! - Managing stream subscriptions per client session
//! - Handling layer switching with keyframe alignment
//!
//! Architecture:
//! ```text
//!     Camera
//!       │
//!       ├── Main Stream (1080p/4K) ──┐
//!       │                            │
//!       └── Sub Stream (480p/720p) ──┼──► StreamRouter ──► Client Sessions
//!                                    │         │
//!                                    │         ├── Session A (High layer) → Main
//!                                    │         ├── Session B (Low layer) → Sub
//!                                    │         └── Session C (Auto) → depends
//! ```

use crate::api::{LayerChangeNotification, LayerChangeReason, StreamType, VideoLayer};
use crate::stream_manager::{StreamChunk, StreamSession};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, info, trace, warn};
use uuid::Uuid;

/// Configuration for stream routing
#[derive(Debug, Clone)]
pub struct StreamRouterConfig {
    /// Minimum time between layer switches (prevents oscillation)
    pub min_switch_interval: Duration,
    /// Whether to wait for keyframe when switching streams
    pub keyframe_aligned_switch: bool,
    /// Timeout for waiting for keyframe during switch
    pub keyframe_wait_timeout: Duration,
    /// Default layer for new sessions
    pub default_layer: VideoLayer,
}

impl Default for StreamRouterConfig {
    fn default() -> Self {
        Self {
            min_switch_interval: Duration::from_secs(2),
            keyframe_aligned_switch: true,
            keyframe_wait_timeout: Duration::from_secs(5),
            default_layer: VideoLayer::Auto,
        }
    }
}

/// A client's subscription to a stream
#[derive(Debug)]
pub struct ClientSubscription {
    /// WebRTC session ID
    pub session_id: Uuid,
    /// Camera ID
    pub camera_id: Uuid,
    /// Current active layer
    pub active_layer: VideoLayer,
    /// Current stream type being received
    pub stream_type: StreamType,
    /// Channel to send stream chunks to this client
    pub chunk_sender: mpsc::UnboundedSender<StreamChunk>,
    /// Whether client is waiting for a keyframe (after layer switch)
    pub waiting_for_keyframe: bool,
    /// When the layer was last switched
    pub last_layer_switch: Option<Instant>,
    /// Created timestamp
    pub created_at: Instant,
}

/// Manages stream routing for a single camera
pub struct CameraStreamRouter {
    /// Camera ID
    pub camera_id: Uuid,
    /// Configuration
    config: StreamRouterConfig,
    /// Client subscriptions by session ID
    subscriptions: RwLock<HashMap<Uuid, ClientSubscription>>,
    /// Main stream session (if available)
    main_stream: RwLock<Option<Arc<StreamSession>>>,
    /// Sub stream session (if available)
    sub_stream: RwLock<Option<Arc<StreamSession>>>,
    /// Stats: total chunks routed
    chunks_routed: RwLock<u64>,
}

impl CameraStreamRouter {
    /// Create a new router for a camera
    pub fn new(camera_id: Uuid, config: StreamRouterConfig) -> Self {
        Self {
            camera_id,
            config,
            subscriptions: RwLock::new(HashMap::new()),
            main_stream: RwLock::new(None),
            sub_stream: RwLock::new(None),
            chunks_routed: RwLock::new(0),
        }
    }

    /// Set the main stream session
    pub fn set_main_stream(&self, session: Arc<StreamSession>) {
        *self.main_stream.write() = Some(session);
        info!(camera_id = %self.camera_id, "Main stream connected");
    }

    /// Set the sub stream session
    pub fn set_sub_stream(&self, session: Arc<StreamSession>) {
        *self.sub_stream.write() = Some(session);
        info!(camera_id = %self.camera_id, "Sub stream connected");
    }

    /// Remove main stream
    pub fn clear_main_stream(&self) {
        *self.main_stream.write() = None;
        info!(camera_id = %self.camera_id, "Main stream disconnected");
    }

    /// Remove sub stream
    pub fn clear_sub_stream(&self) {
        *self.sub_stream.write() = None;
        info!(camera_id = %self.camera_id, "Sub stream disconnected");
    }

    /// Subscribe a client session to receive stream data
    pub fn subscribe(
        &self,
        session_id: Uuid,
        initial_layer: VideoLayer,
    ) -> mpsc::UnboundedReceiver<StreamChunk> {
        let (tx, rx) = mpsc::unbounded_channel();
        
        let stream_type = initial_layer.to_stream_type();
        
        let subscription = ClientSubscription {
            session_id,
            camera_id: self.camera_id,
            active_layer: initial_layer,
            stream_type,
            chunk_sender: tx,
            waiting_for_keyframe: true, // Always wait for first keyframe
            last_layer_switch: None,
            created_at: Instant::now(),
        };

        self.subscriptions.write().insert(session_id, subscription);
        info!(
            camera_id = %self.camera_id,
            session_id = %session_id,
            layer = %initial_layer,
            "Client subscribed to stream"
        );

        rx
    }

    /// Unsubscribe a client session
    pub fn unsubscribe(&self, session_id: Uuid) {
        if self.subscriptions.write().remove(&session_id).is_some() {
            info!(
                camera_id = %self.camera_id,
                session_id = %session_id,
                "Client unsubscribed from stream"
            );
        }
    }

    /// Switch a client's layer
    pub fn switch_layer(
        &self,
        session_id: Uuid,
        new_layer: VideoLayer,
    ) -> Result<LayerChangeNotification, StreamRouterError> {
        let mut subs = self.subscriptions.write();
        let sub = subs
            .get_mut(&session_id)
            .ok_or(StreamRouterError::SessionNotFound)?;

        // Check minimum switch interval
        if let Some(last_switch) = sub.last_layer_switch {
            if last_switch.elapsed() < self.config.min_switch_interval {
                return Err(StreamRouterError::SwitchTooFast);
            }
        }

        let previous_layer = sub.active_layer;
        let previous_stream = sub.stream_type;
        let new_stream_type = new_layer.to_stream_type();

        // Update subscription
        sub.active_layer = new_layer;
        sub.stream_type = new_stream_type;
        sub.last_layer_switch = Some(Instant::now());
        
        // If stream type changed, wait for keyframe
        if new_stream_type != previous_stream && self.config.keyframe_aligned_switch {
            sub.waiting_for_keyframe = true;
        }

        info!(
            camera_id = %self.camera_id,
            session_id = %session_id,
            previous_layer = %previous_layer,
            new_layer = %new_layer,
            "Layer switched"
        );

        Ok(LayerChangeNotification {
            active_layer: new_layer,
            previous_layer: Some(previous_layer),
            reason: LayerChangeReason::ClientRequest,
            stream_type: new_stream_type,
            estimated_bitrate_kbps: Some(new_layer.typical_bitrate_kbps()),
            width: None,
            height: None,
        })
    }

    /// Route a stream chunk to appropriate subscribers
    pub fn route_chunk(&self, stream_type: StreamType, chunk: StreamChunk) {
        let mut subs = self.subscriptions.write();
        let mut routed = 0u32;

        for sub in subs.values_mut() {
            // Check if this client wants this stream type
            if sub.stream_type != stream_type {
                continue;
            }

            // If waiting for keyframe, only send if this is a keyframe
            if sub.waiting_for_keyframe {
                if chunk.is_keyframe {
                    sub.waiting_for_keyframe = false;
                    debug!(
                        camera_id = %self.camera_id,
                        session_id = %sub.session_id,
                        "Got keyframe, starting stream"
                    );
                } else {
                    trace!(
                        camera_id = %self.camera_id,
                        session_id = %sub.session_id,
                        "Waiting for keyframe, skipping chunk"
                    );
                    continue;
                }
            }

            // Send to client
            if sub.chunk_sender.send(chunk.clone()).is_err() {
                // Client disconnected, will be cleaned up later
                warn!(
                    camera_id = %self.camera_id,
                    session_id = %sub.session_id,
                    "Failed to send chunk to client"
                );
            } else {
                routed += 1;
            }
        }

        if routed > 0 {
            *self.chunks_routed.write() += routed as u64;
        }
    }

    /// Get list of active layer for each stream type
    pub fn get_required_streams(&self) -> (bool, bool) {
        let subs = self.subscriptions.read();
        let mut need_main = false;
        let mut need_sub = false;

        for sub in subs.values() {
            match sub.stream_type {
                StreamType::Main => need_main = true,
                StreamType::Sub => need_sub = true,
            }
        }

        (need_main, need_sub)
    }

    /// Get subscription count
    pub fn subscription_count(&self) -> usize {
        self.subscriptions.read().len()
    }

    /// Get stats
    pub fn stats(&self) -> StreamRouterStats {
        let subs = self.subscriptions.read();
        let (need_main, need_sub) = self.get_required_streams();

        StreamRouterStats {
            camera_id: self.camera_id,
            subscription_count: subs.len(),
            main_stream_active: self.main_stream.read().is_some(),
            sub_stream_active: self.sub_stream.read().is_some(),
            clients_on_main: subs.values().filter(|s| s.stream_type == StreamType::Main).count(),
            clients_on_sub: subs.values().filter(|s| s.stream_type == StreamType::Sub).count(),
            need_main_stream: need_main,
            need_sub_stream: need_sub,
            chunks_routed: *self.chunks_routed.read(),
        }
    }

    /// Clean up disconnected clients
    pub fn cleanup_disconnected(&self) {
        let mut subs = self.subscriptions.write();
        subs.retain(|session_id, sub| {
            if sub.chunk_sender.is_closed() {
                info!(
                    camera_id = %self.camera_id,
                    session_id = %session_id,
                    "Removing disconnected client"
                );
                false
            } else {
                true
            }
        });
    }
}

/// Statistics for a stream router
#[derive(Debug, Clone)]
pub struct StreamRouterStats {
    pub camera_id: Uuid,
    pub subscription_count: usize,
    pub main_stream_active: bool,
    pub sub_stream_active: bool,
    pub clients_on_main: usize,
    pub clients_on_sub: usize,
    pub need_main_stream: bool,
    pub need_sub_stream: bool,
    pub chunks_routed: u64,
}

/// Global stream router manager
pub struct StreamRouterManager {
    /// Configuration
    config: StreamRouterConfig,
    /// Routers by camera ID
    routers: RwLock<HashMap<Uuid, Arc<CameraStreamRouter>>>,
}

impl StreamRouterManager {
    /// Create a new manager
    pub fn new(config: StreamRouterConfig) -> Self {
        Self {
            config,
            routers: RwLock::new(HashMap::new()),
        }
    }

    /// Get or create a router for a camera
    pub fn get_or_create(&self, camera_id: Uuid) -> Arc<CameraStreamRouter> {
        let routers = self.routers.read();
        if let Some(router) = routers.get(&camera_id) {
            return Arc::clone(router);
        }
        drop(routers);

        // Create new router
        let router = Arc::new(CameraStreamRouter::new(camera_id, self.config.clone()));
        self.routers.write().insert(camera_id, Arc::clone(&router));
        
        info!(camera_id = %camera_id, "Created stream router");
        router
    }

    /// Get router for a camera (if exists)
    pub fn get(&self, camera_id: Uuid) -> Option<Arc<CameraStreamRouter>> {
        self.routers.read().get(&camera_id).cloned()
    }

    /// Remove router for a camera
    pub fn remove(&self, camera_id: Uuid) {
        if self.routers.write().remove(&camera_id).is_some() {
            info!(camera_id = %camera_id, "Removed stream router");
        }
    }

    /// Get all stats
    pub fn all_stats(&self) -> Vec<StreamRouterStats> {
        self.routers
            .read()
            .values()
            .map(|r| r.stats())
            .collect()
    }

    /// Cleanup all disconnected clients
    pub fn cleanup_all(&self) {
        for router in self.routers.read().values() {
            router.cleanup_disconnected();
        }
    }
}

/// Stream router errors
#[derive(Debug, thiserror::Error)]
pub enum StreamRouterError {
    #[error("Session not found")]
    SessionNotFound,

    #[error("Layer switch too fast, please wait")]
    SwitchTooFast,

    #[error("Stream not available")]
    StreamNotAvailable,

    #[error("Internal error: {0}")]
    Internal(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_router_creation() {
        let camera_id = Uuid::new_v4();
        let config = StreamRouterConfig::default();
        let router = CameraStreamRouter::new(camera_id, config);

        assert_eq!(router.camera_id, camera_id);
        assert_eq!(router.subscription_count(), 0);
    }

    #[test]
    fn test_subscription() {
        let camera_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let config = StreamRouterConfig::default();
        let router = CameraStreamRouter::new(camera_id, config);

        let _rx = router.subscribe(session_id, VideoLayer::High);
        
        assert_eq!(router.subscription_count(), 1);
        
        let (need_main, need_sub) = router.get_required_streams();
        assert!(need_main);
        assert!(!need_sub);
    }

    #[test]
    fn test_layer_switch() {
        let camera_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let mut config = StreamRouterConfig::default();
        config.min_switch_interval = Duration::from_millis(0); // Disable for test
        
        let router = CameraStreamRouter::new(camera_id, config);
        let _rx = router.subscribe(session_id, VideoLayer::High);

        // Switch to low
        let notification = router.switch_layer(session_id, VideoLayer::Low).unwrap();
        assert_eq!(notification.active_layer, VideoLayer::Low);
        assert_eq!(notification.previous_layer, Some(VideoLayer::High));

        let (need_main, need_sub) = router.get_required_streams();
        assert!(!need_main);
        assert!(need_sub);
    }
}
