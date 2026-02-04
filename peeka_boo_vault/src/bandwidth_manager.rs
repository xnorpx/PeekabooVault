//! Bandwidth Budget Manager for Multi-Camera Streaming
//!
//! Coordinates bandwidth allocation across multiple camera streams to ensure
//! optimal quality distribution based on available network capacity.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                    BandwidthBudgetManager                           │
//! │  ┌───────────────────────────────────────────────────────────────┐  │
//! │  │  Total Budget: 8000 kbps (from BWE)                           │  │
//! │  │  Reserved for base: N × 400 kbps (sub streams)                │  │
//! │  │  Available for focus: 8000 - (N × 400) kbps                   │  │
//! │  └───────────────────────────────────────────────────────────────┘  │
//! │                                                                     │
//! │  Allocations:                                                       │
//! │  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐                   │
//! │  │ cam1    │ │ cam2    │ │ cam3    │ │ cam4    │                   │
//! │  │ LOW     │ │ HIGH    │ │ LOW     │ │ LOW     │                   │
//! │  │ 400kbps │ │ 4000kbps│ │ 400kbps │ │ 400kbps │                   │
//! │  └─────────┘ └─────────┘ └─────────┘ └─────────┘                   │
//! │                                                                     │
//! │  Total used: 5200 kbps                                             │
//! │  Headroom: 2800 kbps                                               │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Focus Mode
//!
//! When a client requests focus on a camera:
//! 1. Check if bandwidth budget allows HIGH stream
//! 2. If yes, upgrade that camera and notify all clients
//! 3. If no, either deny or downgrade another focused camera
//!
//! ## Adaptive Behavior
//!
//! When BWE reports bandwidth changes:
//! 1. If bandwidth increased → allow more focus streams
//! 2. If bandwidth decreased → may need to downgrade focused cameras

use crate::api::VideoLayer;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Typical bitrates for different stream types (kbps)
#[derive(Debug, Clone, Copy)]
pub struct StreamBitrates {
    /// Sub/low quality stream bitrate
    pub sub_kbps: u32,
    /// Main/high quality stream bitrate  
    pub main_kbps: u32,
    /// Medium quality (interpolated)
    pub medium_kbps: u32,
}

impl Default for StreamBitrates {
    fn default() -> Self {
        Self {
            sub_kbps: 400,      // Typical IP camera sub stream
            main_kbps: 4000,    // Typical IP camera main stream (4 Mbps)
            medium_kbps: 1500,  // Medium quality
        }
    }
}

/// Allocation status for a single camera stream
#[derive(Debug, Clone)]
pub struct StreamAllocation {
    /// Camera ID
    pub camera_id: Uuid,
    /// Client/session ID that owns this allocation
    pub session_id: Uuid,
    /// Current layer/quality
    pub layer: VideoLayer,
    /// Estimated bitrate consumption (kbps)
    pub bitrate_kbps: u32,
    /// Whether this camera has "focus" (upgraded quality)
    pub is_focused: bool,
    /// When this allocation was created
    pub created_at: Instant,
    /// When focus was granted (if focused)
    pub focused_at: Option<Instant>,
}

impl StreamAllocation {
    pub fn new(camera_id: Uuid, session_id: Uuid, layer: VideoLayer, bitrate_kbps: u32) -> Self {
        Self {
            camera_id,
            session_id,
            layer,
            bitrate_kbps,
            is_focused: layer == VideoLayer::High,
            created_at: Instant::now(),
            focused_at: if layer == VideoLayer::High {
                Some(Instant::now())
            } else {
                None
            },
        }
    }
}

/// Result of a focus request
#[derive(Debug, Clone)]
pub enum FocusResult {
    /// Focus granted - camera upgraded to high quality
    Granted {
        camera_id: Uuid,
        new_layer: VideoLayer,
        bitrate_kbps: u32,
    },
    /// Focus denied - not enough bandwidth
    Denied {
        camera_id: Uuid,
        reason: String,
        available_kbps: u32,
        required_kbps: u32,
    },
    /// Focus granted but another camera was downgraded
    GrantedWithDowngrade {
        camera_id: Uuid,
        new_layer: VideoLayer,
        downgraded_camera: Uuid,
        downgraded_to: VideoLayer,
    },
}

/// Event broadcast when allocations change
#[derive(Debug, Clone)]
pub enum BandwidthEvent {
    /// A camera's layer changed
    LayerChanged {
        camera_id: Uuid,
        session_id: Uuid,
        old_layer: VideoLayer,
        new_layer: VideoLayer,
        reason: String,
    },
    /// Bandwidth estimate updated
    BudgetUpdated {
        total_kbps: u32,
        used_kbps: u32,
        available_kbps: u32,
    },
    /// Focus granted
    FocusGranted {
        camera_id: Uuid,
        session_id: Uuid,
    },
    /// Focus revoked (due to bandwidth or explicit release)
    FocusRevoked {
        camera_id: Uuid,
        session_id: Uuid,
        reason: String,
    },
}

/// Configuration for the bandwidth manager
#[derive(Debug, Clone)]
pub struct BandwidthManagerConfig {
    /// Typical stream bitrates
    pub bitrates: StreamBitrates,
    /// Maximum number of simultaneous focus streams
    pub max_focus_streams: usize,
    /// Minimum bandwidth headroom to maintain (kbps)
    pub min_headroom_kbps: u32,
    /// How long before an inactive focus can be reclaimed
    pub focus_timeout: Duration,
    /// Initial bandwidth estimate if none received yet
    pub initial_bandwidth_kbps: u32,
}

impl Default for BandwidthManagerConfig {
    fn default() -> Self {
        Self {
            bitrates: StreamBitrates::default(),
            max_focus_streams: 2,        // Allow up to 2 HD streams
            min_headroom_kbps: 500,      // Keep 500kbps buffer
            focus_timeout: Duration::from_secs(300), // 5 minutes
            initial_bandwidth_kbps: 5000, // Start assuming 5 Mbps
        }
    }
}

/// Central bandwidth budget manager
pub struct BandwidthBudgetManager {
    /// Configuration
    config: BandwidthManagerConfig,
    /// Current allocations by session ID
    allocations: RwLock<HashMap<Uuid, StreamAllocation>>,
    /// Estimated total available bandwidth (kbps)
    estimated_bandwidth_kbps: RwLock<u32>,
    /// Last BWE update time
    last_bwe_update: RwLock<Option<Instant>>,
    /// Event broadcaster
    event_tx: broadcast::Sender<BandwidthEvent>,
}

impl BandwidthBudgetManager {
    /// Create a new bandwidth manager
    pub fn new(config: BandwidthManagerConfig) -> Arc<Self> {
        let (event_tx, _) = broadcast::channel(64);
        let initial_bw = config.initial_bandwidth_kbps;
        
        Arc::new(Self {
            config,
            allocations: RwLock::new(HashMap::new()),
            estimated_bandwidth_kbps: RwLock::new(initial_bw),
            last_bwe_update: RwLock::new(None),
            event_tx,
        })
    }

    /// Subscribe to bandwidth events
    pub fn subscribe(&self) -> broadcast::Receiver<BandwidthEvent> {
        self.event_tx.subscribe()
    }

    /// Register a new stream (starts at LOW by default)
    pub fn register_stream(&self, camera_id: Uuid, session_id: Uuid) -> StreamAllocation {
        let allocation = StreamAllocation::new(
            camera_id,
            session_id,
            VideoLayer::Low,
            self.config.bitrates.sub_kbps,
        );
        
        self.allocations.write().insert(session_id, allocation.clone());
        
        info!(
            camera_id = %camera_id,
            session_id = %session_id,
            "Registered new stream allocation (LOW)"
        );
        
        self.broadcast_budget_update();
        allocation
    }

    /// Unregister a stream when session ends
    pub fn unregister_stream(&self, session_id: Uuid) {
        if let Some(allocation) = self.allocations.write().remove(&session_id) {
            info!(
                camera_id = %allocation.camera_id,
                session_id = %session_id,
                was_focused = allocation.is_focused,
                "Unregistered stream allocation"
            );
            
            if allocation.is_focused {
                let _ = self.event_tx.send(BandwidthEvent::FocusRevoked {
                    camera_id: allocation.camera_id,
                    session_id,
                    reason: "Session ended".to_string(),
                });
            }
            
            self.broadcast_budget_update();
        }
    }

    /// Update bandwidth estimate from BWE feedback
    pub fn update_bandwidth_estimate(&self, bandwidth_kbps: u32) {
        let old_bw = *self.estimated_bandwidth_kbps.read();
        *self.estimated_bandwidth_kbps.write() = bandwidth_kbps;
        *self.last_bwe_update.write() = Some(Instant::now());
        
        debug!(
            old_kbps = old_bw,
            new_kbps = bandwidth_kbps,
            "Bandwidth estimate updated"
        );
        
        // Check if we need to downgrade any focused streams
        if bandwidth_kbps < old_bw {
            self.check_for_required_downgrades();
        }
        
        self.broadcast_budget_update();
    }

    /// Request focus (upgrade to HIGH) for a camera
    pub fn request_focus(&self, session_id: Uuid) -> FocusResult {
        let mut allocations = self.allocations.write();
        
        let Some(allocation) = allocations.get(&session_id) else {
            return FocusResult::Denied {
                camera_id: Uuid::nil(),
                reason: "Session not registered".to_string(),
                available_kbps: 0,
                required_kbps: 0,
            };
        };
        
        let camera_id = allocation.camera_id;
        
        // Already focused?
        if allocation.is_focused {
            return FocusResult::Granted {
                camera_id,
                new_layer: VideoLayer::High,
                bitrate_kbps: self.config.bitrates.main_kbps,
            };
        }
        
        // Calculate available bandwidth
        let total_bw = *self.estimated_bandwidth_kbps.read();
        let used_bw: u32 = allocations.values().map(|a| a.bitrate_kbps).sum();
        let upgrade_cost = self.config.bitrates.main_kbps - self.config.bitrates.sub_kbps;
        let available = total_bw.saturating_sub(used_bw + self.config.min_headroom_kbps);
        
        // Count current focus streams
        let focus_count = allocations.values().filter(|a| a.is_focused).count();
        
        // Can we afford it?
        if available >= upgrade_cost && focus_count < self.config.max_focus_streams {
            // Grant focus
            let allocation = allocations.get_mut(&session_id).unwrap();
            let old_layer = allocation.layer;
            allocation.layer = VideoLayer::High;
            allocation.bitrate_kbps = self.config.bitrates.main_kbps;
            allocation.is_focused = true;
            allocation.focused_at = Some(Instant::now());
            
            info!(
                camera_id = %camera_id,
                session_id = %session_id,
                "Focus granted"
            );
            
            drop(allocations);
            
            let _ = self.event_tx.send(BandwidthEvent::FocusGranted {
                camera_id,
                session_id,
            });
            let _ = self.event_tx.send(BandwidthEvent::LayerChanged {
                camera_id,
                session_id,
                old_layer,
                new_layer: VideoLayer::High,
                reason: "Focus requested".to_string(),
            });
            
            self.broadcast_budget_update();
            
            FocusResult::Granted {
                camera_id,
                new_layer: VideoLayer::High,
                bitrate_kbps: self.config.bitrates.main_kbps,
            }
        } else if focus_count >= self.config.max_focus_streams {
            // Try to steal focus from oldest focused camera
            let oldest_focus = allocations
                .iter()
                .filter(|(_, a)| a.is_focused && a.session_id != session_id)
                .min_by_key(|(_, a)| a.focused_at);
            
            if let Some((oldest_session_id, oldest_allocation)) = oldest_focus {
                let downgraded_camera = oldest_allocation.camera_id;
                let oldest_session_id = *oldest_session_id;
                
                // Downgrade the oldest
                if let Some(old_alloc) = allocations.get_mut(&oldest_session_id) {
                    old_alloc.layer = VideoLayer::Low;
                    old_alloc.bitrate_kbps = self.config.bitrates.sub_kbps;
                    old_alloc.is_focused = false;
                    old_alloc.focused_at = None;
                }
                
                // Upgrade the requester
                let allocation = allocations.get_mut(&session_id).unwrap();
                allocation.layer = VideoLayer::High;
                allocation.bitrate_kbps = self.config.bitrates.main_kbps;
                allocation.is_focused = true;
                allocation.focused_at = Some(Instant::now());
                
                info!(
                    camera_id = %camera_id,
                    session_id = %session_id,
                    downgraded = %downgraded_camera,
                    "Focus granted with downgrade"
                );
                
                drop(allocations);
                
                let _ = self.event_tx.send(BandwidthEvent::FocusRevoked {
                    camera_id: downgraded_camera,
                    session_id: oldest_session_id,
                    reason: "Replaced by newer focus request".to_string(),
                });
                let _ = self.event_tx.send(BandwidthEvent::FocusGranted {
                    camera_id,
                    session_id,
                });
                
                self.broadcast_budget_update();
                
                FocusResult::GrantedWithDowngrade {
                    camera_id,
                    new_layer: VideoLayer::High,
                    downgraded_camera,
                    downgraded_to: VideoLayer::Low,
                }
            } else {
                FocusResult::Denied {
                    camera_id,
                    reason: "Maximum focus streams reached".to_string(),
                    available_kbps: available,
                    required_kbps: upgrade_cost,
                }
            }
        } else {
            FocusResult::Denied {
                camera_id,
                reason: "Insufficient bandwidth".to_string(),
                available_kbps: available,
                required_kbps: upgrade_cost,
            }
        }
    }

    /// Release focus (downgrade back to LOW)
    pub fn release_focus(&self, session_id: Uuid) {
        let mut allocations = self.allocations.write();
        
        if let Some(allocation) = allocations.get_mut(&session_id) {
            if allocation.is_focused {
                let camera_id = allocation.camera_id;
                let old_layer = allocation.layer;
                
                allocation.layer = VideoLayer::Low;
                allocation.bitrate_kbps = self.config.bitrates.sub_kbps;
                allocation.is_focused = false;
                allocation.focused_at = None;
                
                info!(
                    camera_id = %camera_id,
                    session_id = %session_id,
                    "Focus released"
                );
                
                drop(allocations);
                
                let _ = self.event_tx.send(BandwidthEvent::FocusRevoked {
                    camera_id,
                    session_id,
                    reason: "Released by client".to_string(),
                });
                let _ = self.event_tx.send(BandwidthEvent::LayerChanged {
                    camera_id,
                    session_id,
                    old_layer,
                    new_layer: VideoLayer::Low,
                    reason: "Focus released".to_string(),
                });
                
                self.broadcast_budget_update();
            }
        }
    }

    /// Get current budget summary
    pub fn get_budget_summary(&self) -> BudgetSummary {
        let allocations = self.allocations.read();
        let total_kbps = *self.estimated_bandwidth_kbps.read();
        let used_kbps: u32 = allocations.values().map(|a| a.bitrate_kbps).sum();
        let focus_count = allocations.values().filter(|a| a.is_focused).count();
        let stream_count = allocations.len();
        
        let available_kbps = total_kbps.saturating_sub(used_kbps + self.config.min_headroom_kbps);
        let can_upgrade = available_kbps >= (self.config.bitrates.main_kbps - self.config.bitrates.sub_kbps)
            && focus_count < self.config.max_focus_streams;
        
        BudgetSummary {
            total_kbps,
            used_kbps,
            available_kbps,
            headroom_kbps: total_kbps.saturating_sub(used_kbps),
            stream_count,
            focus_count,
            can_upgrade,
            recommendation: if can_upgrade {
                "upgrade_available".to_string()
            } else if used_kbps > total_kbps {
                "oversubscribed".to_string()
            } else {
                "at_capacity".to_string()
            },
        }
    }

    /// Get allocation for a session
    pub fn get_allocation(&self, session_id: Uuid) -> Option<StreamAllocation> {
        self.allocations.read().get(&session_id).cloned()
    }

    /// Get all allocations (for debugging/admin)
    pub fn get_all_allocations(&self) -> Vec<StreamAllocation> {
        self.allocations.read().values().cloned().collect()
    }

    /// Check if any focused streams need to be downgraded due to bandwidth
    fn check_for_required_downgrades(&self) {
        let summary = self.get_budget_summary();
        
        if summary.used_kbps <= summary.total_kbps {
            return; // No action needed
        }
        
        // Need to downgrade - find focused streams to downgrade
        let mut allocations = self.allocations.write();
        let overage = summary.used_kbps - summary.total_kbps;
        let mut freed = 0u32;
        
        // Collect focused streams sorted by focus time (oldest first)
        let mut focused: Vec<_> = allocations
            .iter()
            .filter(|(_, a)| a.is_focused)
            .map(|(id, a)| (*id, a.focused_at))
            .collect();
        focused.sort_by_key(|(_, t)| *t);
        
        for (session_id, _) in focused {
            if freed >= overage {
                break;
            }
            
            if let Some(allocation) = allocations.get_mut(&session_id) {
                let camera_id = allocation.camera_id;
                let savings = allocation.bitrate_kbps - self.config.bitrates.sub_kbps;
                
                allocation.layer = VideoLayer::Low;
                allocation.bitrate_kbps = self.config.bitrates.sub_kbps;
                allocation.is_focused = false;
                allocation.focused_at = None;
                
                freed += savings;
                
                warn!(
                    camera_id = %camera_id,
                    session_id = %session_id,
                    savings_kbps = savings,
                    "Auto-downgraded due to bandwidth constraint"
                );
                
                // Note: Can't send events while holding lock, would need to collect and send after
            }
        }
    }

    fn broadcast_budget_update(&self) {
        let summary = self.get_budget_summary();
        let _ = self.event_tx.send(BandwidthEvent::BudgetUpdated {
            total_kbps: summary.total_kbps,
            used_kbps: summary.used_kbps,
            available_kbps: summary.available_kbps,
        });
    }
}

/// Summary of current bandwidth budget
#[derive(Debug, Clone)]
pub struct BudgetSummary {
    /// Total estimated bandwidth (kbps)
    pub total_kbps: u32,
    /// Currently used bandwidth (kbps)
    pub used_kbps: u32,
    /// Available for new allocations (kbps)
    pub available_kbps: u32,
    /// Headroom above used (kbps)
    pub headroom_kbps: u32,
    /// Number of active streams
    pub stream_count: usize,
    /// Number of focused (HIGH) streams
    pub focus_count: usize,
    /// Can afford another HIGH stream?
    pub can_upgrade: bool,
    /// Recommendation string
    pub recommendation: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_stream() {
        let manager = BandwidthBudgetManager::new(BandwidthManagerConfig::default());
        let camera_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        
        let allocation = manager.register_stream(camera_id, session_id);
        
        assert_eq!(allocation.layer, VideoLayer::Low);
        assert_eq!(allocation.bitrate_kbps, 400);
        assert!(!allocation.is_focused);
    }

    #[test]
    fn test_request_focus() {
        let manager = BandwidthBudgetManager::new(BandwidthManagerConfig::default());
        let camera_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        
        manager.register_stream(camera_id, session_id);
        
        // Should be able to get focus with default 5000 kbps budget
        let result = manager.request_focus(session_id);
        
        match result {
            FocusResult::Granted { new_layer, .. } => {
                assert_eq!(new_layer, VideoLayer::High);
            }
            _ => panic!("Expected focus to be granted"),
        }
        
        // Check allocation was updated
        let allocation = manager.get_allocation(session_id).unwrap();
        assert!(allocation.is_focused);
        assert_eq!(allocation.layer, VideoLayer::High);
    }

    #[test]
    fn test_focus_denied_bandwidth() {
        let mut config = BandwidthManagerConfig::default();
        config.initial_bandwidth_kbps = 500; // Very low bandwidth
        
        let manager = BandwidthBudgetManager::new(config);
        let camera_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        
        manager.register_stream(camera_id, session_id);
        
        // Should be denied due to low bandwidth
        let result = manager.request_focus(session_id);
        
        match result {
            FocusResult::Denied { reason, .. } => {
                assert!(reason.contains("bandwidth"));
            }
            _ => panic!("Expected focus to be denied"),
        }
    }

    #[test]
    fn test_focus_with_downgrade() {
        let mut config = BandwidthManagerConfig::default();
        config.max_focus_streams = 1;
        config.initial_bandwidth_kbps = 10000;
        
        let manager = BandwidthBudgetManager::new(config);
        
        // Register two cameras
        let camera1 = Uuid::new_v4();
        let session1 = Uuid::new_v4();
        let camera2 = Uuid::new_v4();
        let session2 = Uuid::new_v4();
        
        manager.register_stream(camera1, session1);
        manager.register_stream(camera2, session2);
        
        // Focus camera 1
        let _ = manager.request_focus(session1);
        
        // Focus camera 2 - should downgrade camera 1
        let result = manager.request_focus(session2);
        
        match result {
            FocusResult::GrantedWithDowngrade { downgraded_camera, .. } => {
                assert_eq!(downgraded_camera, camera1);
            }
            _ => panic!("Expected focus with downgrade"),
        }
        
        // Check camera 1 was downgraded
        let alloc1 = manager.get_allocation(session1).unwrap();
        assert!(!alloc1.is_focused);
        assert_eq!(alloc1.layer, VideoLayer::Low);
    }

    #[test]
    fn test_budget_summary() {
        let manager = BandwidthBudgetManager::new(BandwidthManagerConfig::default());
        
        // Add some streams
        for _ in 0..4 {
            let camera_id = Uuid::new_v4();
            let session_id = Uuid::new_v4();
            manager.register_stream(camera_id, session_id);
        }
        
        let summary = manager.get_budget_summary();
        
        assert_eq!(summary.stream_count, 4);
        assert_eq!(summary.used_kbps, 4 * 400); // 4 × sub stream
        assert!(summary.total_kbps >= summary.used_kbps);
    }
}
