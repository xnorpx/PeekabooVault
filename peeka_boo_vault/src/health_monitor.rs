//! Health Monitoring for PeekabooVault
//!
//! This module implements:
//! - Periodic health checks for cameras (every 30s by default)
//! - State machine for camera health (online/offline transitions)
//! - Event normalization using Scrypted-style topic stripping
//! - Health history tracking in the database

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::api::CameraStatus;
use crate::db::Database;

/// Health check configuration
#[derive(Debug, Clone)]
pub struct HealthConfig {
    /// Interval between health checks (default: 30 seconds)
    pub check_interval: Duration,
    /// Timeout for health check probes (default: 10 seconds)
    pub probe_timeout: Duration,
    /// Number of consecutive failures before marking offline
    pub offline_threshold: u32,
    /// Number of consecutive successes before marking online
    pub online_threshold: u32,
    /// How long to keep health history (default: 24 hours)
    pub history_retention: Duration,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            check_interval: Duration::from_secs(30),
            probe_timeout: Duration::from_secs(10),
            offline_threshold: 3,
            online_threshold: 1,
            history_retention: Duration::from_secs(24 * 60 * 60),
        }
    }
}

/// Camera health state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthState {
    /// Camera is online and responding
    Online,
    /// Camera is offline (not responding)
    Offline,
    /// Camera is in degraded state (responding but with issues)
    Degraded,
    /// Health state is unknown (not yet checked)
    Unknown,
}

impl From<HealthState> for CameraStatus {
    fn from(state: HealthState) -> Self {
        match state {
            HealthState::Online => CameraStatus::Online,
            HealthState::Offline => CameraStatus::Offline,
            HealthState::Degraded => CameraStatus::Online, // Degraded is still "online-ish"
            HealthState::Unknown => CameraStatus::Unknown,
        }
    }
}

/// Health check result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    /// Camera ID
    pub camera_id: Uuid,
    /// Check timestamp
    pub timestamp: DateTime<Utc>,
    /// Whether the check was successful
    pub success: bool,
    /// Response latency in milliseconds
    pub latency_ms: Option<u32>,
    /// Error message if failed
    pub error: Option<String>,
    /// Additional details
    pub details: Option<String>,
}

/// Camera health tracker
#[derive(Debug)]
pub struct CameraHealth {
    /// Camera ID
    pub camera_id: Uuid,
    /// Current health state
    pub state: HealthState,
    /// Last successful check time
    pub last_success: Option<DateTime<Utc>>,
    /// Last failure time
    pub last_failure: Option<DateTime<Utc>>,
    /// Consecutive successes
    pub consecutive_successes: u32,
    /// Consecutive failures
    pub consecutive_failures: u32,
    /// Average latency (rolling)
    pub avg_latency_ms: Option<f64>,
    /// Recent health checks
    pub recent_checks: Vec<HealthCheck>,
}

impl CameraHealth {
    /// Create a new camera health tracker
    pub fn new(camera_id: Uuid) -> Self {
        Self {
            camera_id,
            state: HealthState::Unknown,
            last_success: None,
            last_failure: None,
            consecutive_successes: 0,
            consecutive_failures: 0,
            avg_latency_ms: None,
            recent_checks: Vec::new(),
        }
    }

    /// Record a health check result
    pub fn record_check(&mut self, check: HealthCheck, config: &HealthConfig) {
        if check.success {
            self.consecutive_successes += 1;
            self.consecutive_failures = 0;
            self.last_success = Some(check.timestamp);

            // Update rolling average latency
            if let Some(latency) = check.latency_ms {
                self.avg_latency_ms = Some(match self.avg_latency_ms {
                    Some(avg) => avg * 0.8 + latency as f64 * 0.2,
                    None => latency as f64,
                });
            }

            // Transition to online if threshold met
            if self.consecutive_successes >= config.online_threshold {
                if self.state != HealthState::Online {
                    info!(camera_id = %self.camera_id, "Camera is now online");
                }
                self.state = HealthState::Online;
            }
        } else {
            self.consecutive_failures += 1;
            self.consecutive_successes = 0;
            self.last_failure = Some(check.timestamp);

            // Transition to offline if threshold met
            if self.consecutive_failures >= config.offline_threshold {
                if self.state != HealthState::Offline {
                    warn!(
                        camera_id = %self.camera_id,
                        failures = self.consecutive_failures,
                        "Camera is now offline"
                    );
                }
                self.state = HealthState::Offline;
            } else if self.state == HealthState::Online {
                // Degraded state during failure threshold
                self.state = HealthState::Degraded;
            }
        }

        // Keep recent checks (last 100)
        self.recent_checks.push(check);
        if self.recent_checks.len() > 100 {
            self.recent_checks.remove(0);
        }
    }
}

/// Health monitor service
pub struct HealthMonitor {
    /// Health configuration
    config: HealthConfig,
    /// Camera health states
    cameras: Arc<RwLock<HashMap<Uuid, CameraHealth>>>,
    /// Event sender for health state changes
    event_tx: broadcast::Sender<HealthEvent>,
    /// Database for persistence
    db: Option<Arc<Database>>,
}

/// Health event for notifications
#[derive(Debug, Clone)]
pub struct HealthEvent {
    /// Camera ID
    pub camera_id: Uuid,
    /// Previous state
    pub previous_state: HealthState,
    /// New state
    pub new_state: HealthState,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
}

impl HealthMonitor {
    /// Create a new health monitor
    pub fn new(config: HealthConfig) -> Self {
        let (event_tx, _) = broadcast::channel(100);
        Self {
            config,
            cameras: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            db: None,
        }
    }

    /// Set the database for persistence
    pub fn with_database(mut self, db: Arc<Database>) -> Self {
        self.db = Some(db);
        self
    }

    /// Subscribe to health events
    pub fn subscribe(&self) -> broadcast::Receiver<HealthEvent> {
        self.event_tx.subscribe()
    }

    /// Register a camera for health monitoring
    pub async fn register_camera(&self, camera_id: Uuid) {
        let mut cameras = self.cameras.write().await;
        cameras
            .entry(camera_id)
            .or_insert_with(|| CameraHealth::new(camera_id));
        debug!(camera_id = %camera_id, "Registered camera for health monitoring");
    }

    /// Unregister a camera from health monitoring
    pub async fn unregister_camera(&self, camera_id: &Uuid) {
        let mut cameras = self.cameras.write().await;
        cameras.remove(camera_id);
        debug!(camera_id = %camera_id, "Unregistered camera from health monitoring");
    }

    /// Record a health check result
    pub async fn record_health_check(&self, check: HealthCheck) -> Result<()> {
        let previous_state;
        let new_state;

        {
            let mut cameras = self.cameras.write().await;
            let health = cameras
                .entry(check.camera_id)
                .or_insert_with(|| CameraHealth::new(check.camera_id));

            previous_state = health.state;
            health.record_check(check.clone(), &self.config);
            new_state = health.state;
        }

        // Persist to database
        if let Some(ref db) = self.db {
            self.persist_health_check(db, &check).await?;
        }

        // Emit event if state changed
        if previous_state != new_state {
            let event = HealthEvent {
                camera_id: check.camera_id,
                previous_state,
                new_state,
                timestamp: check.timestamp,
            };
            let _ = self.event_tx.send(event);
        }

        Ok(())
    }

    /// Get the current health state of a camera
    pub async fn get_camera_health(&self, camera_id: &Uuid) -> Option<HealthState> {
        let cameras = self.cameras.read().await;
        cameras.get(camera_id).map(|h| h.state)
    }

    /// Get detailed health info for a camera
    pub async fn get_camera_health_details(&self, camera_id: &Uuid) -> Option<CameraHealthInfo> {
        let cameras = self.cameras.read().await;
        cameras.get(camera_id).map(|h| CameraHealthInfo {
            camera_id: h.camera_id,
            state: h.state,
            last_success: h.last_success,
            last_failure: h.last_failure,
            consecutive_successes: h.consecutive_successes,
            consecutive_failures: h.consecutive_failures,
            avg_latency_ms: h.avg_latency_ms,
            recent_checks_count: h.recent_checks.len(),
        })
    }

    /// Get all camera health states
    pub async fn get_all_health(&self) -> Vec<CameraHealthInfo> {
        let cameras = self.cameras.read().await;
        cameras
            .values()
            .map(|h| CameraHealthInfo {
                camera_id: h.camera_id,
                state: h.state,
                last_success: h.last_success,
                last_failure: h.last_failure,
                consecutive_successes: h.consecutive_successes,
                consecutive_failures: h.consecutive_failures,
                avg_latency_ms: h.avg_latency_ms,
                recent_checks_count: h.recent_checks.len(),
            })
            .collect()
    }

    /// Persist health check to database
    async fn persist_health_check(&self, _db: &Database, check: &HealthCheck) -> Result<()> {
        // This would insert into the health_check table
        // For now, we'll just log - actual DB integration depends on camera_id mapping
        debug!(
            camera_id = %check.camera_id,
            success = check.success,
            latency_ms = ?check.latency_ms,
            "Persisting health check"
        );
        Ok(())
    }

    /// Run the health monitoring loop
    pub async fn run(
        self: Arc<Self>,
        mut shutdown: broadcast::Receiver<()>,
        probe_fn: impl Fn(Uuid) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Duration>> + Send>> + Send + Sync + 'static,
    ) {
        info!(
            interval_secs = self.config.check_interval.as_secs(),
            "Starting health monitor loop"
        );

        let mut interval = tokio::time::interval(self.config.check_interval);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    self.run_health_checks(&probe_fn).await;
                }
                _ = shutdown.recv() => {
                    info!("Health monitor shutting down");
                    break;
                }
            }
        }
    }

    /// Run health checks for all registered cameras
    async fn run_health_checks(
        &self,
        probe_fn: &impl Fn(Uuid) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Duration>> + Send>>,
    ) {
        let camera_ids: Vec<Uuid> = {
            let cameras = self.cameras.read().await;
            cameras.keys().copied().collect()
        };

        for camera_id in camera_ids {
            let start = Instant::now();
            let probe_future = probe_fn(camera_id);

            let result = tokio::time::timeout(self.config.probe_timeout, probe_future).await;

            let check = match result {
                Ok(Ok(latency)) => HealthCheck {
                    camera_id,
                    timestamp: Utc::now(),
                    success: true,
                    latency_ms: Some(latency.as_millis() as u32),
                    error: None,
                    details: None,
                },
                Ok(Err(e)) => HealthCheck {
                    camera_id,
                    timestamp: Utc::now(),
                    success: false,
                    latency_ms: Some(start.elapsed().as_millis() as u32),
                    error: Some(e.to_string()),
                    details: None,
                },
                Err(_) => HealthCheck {
                    camera_id,
                    timestamp: Utc::now(),
                    success: false,
                    latency_ms: None,
                    error: Some("Health check timed out".to_string()),
                    details: None,
                },
            };

            if let Err(e) = self.record_health_check(check).await {
                error!(camera_id = %camera_id, error = %e, "Failed to record health check");
            }
        }
    }
}

/// Camera health info for API responses
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraHealthInfo {
    pub camera_id: Uuid,
    pub state: HealthState,
    pub last_success: Option<DateTime<Utc>>,
    pub last_failure: Option<DateTime<Utc>>,
    pub consecutive_successes: u32,
    pub consecutive_failures: u32,
    pub avg_latency_ms: Option<f64>,
    pub recent_checks_count: usize,
}

// ============================================================================
// Event Normalization (Scrypted-style topic stripping)
// ============================================================================

/// Normalized event types
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NormalizedEventType {
    /// Motion detected
    Motion,
    /// Motion ended
    MotionEnd,
    /// Audio detected
    Audio,
    /// Audio ended
    AudioEnd,
    /// Digital input triggered
    DigitalInput,
    /// Tampering detected
    Tamper,
    /// Line crossing
    LineCrossing,
    /// Object detected
    ObjectDetected,
    /// Face detected
    FaceDetected,
    /// Vehicle detected
    VehicleDetected,
    /// Person detected
    PersonDetected,
    /// Unknown event type
    Unknown(String),
}

/// A normalized event from any source
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedEvent {
    /// Unique event ID
    pub id: Uuid,
    /// Camera ID
    pub camera_id: Uuid,
    /// Event type
    pub event_type: NormalizedEventType,
    /// Event timestamp
    pub timestamp: DateTime<Utc>,
    /// Event end timestamp (for duration events)
    pub end_timestamp: Option<DateTime<Utc>>,
    /// Confidence score (0.0-1.0)
    pub confidence: Option<f64>,
    /// Bounding box if applicable [x, y, width, height] normalized 0-1
    pub bounding_box: Option<[f64; 4]>,
    /// Additional metadata as JSON
    pub metadata: Option<serde_json::Value>,
    /// Original event topic (for debugging)
    pub original_topic: Option<String>,
}

/// Event normalizer for converting vendor-specific events
pub struct EventNormalizer;

impl EventNormalizer {
    /// Normalize an ONVIF event topic to a standard event type
    ///
    /// Scrypted-style topic stripping:
    /// - tns1:RuleEngine/CellMotionDetector/Motion -> Motion
    /// - tns1:VideoSource/MotionAlarm -> Motion
    /// - tns1:AudioAnalytics/Audio/DetectedSound -> Audio
    pub fn normalize_onvif_topic(topic: &str) -> NormalizedEventType {
        let topic_lower = topic.to_lowercase();

        // Motion detection patterns
        if topic_lower.contains("motion")
            || topic_lower.contains("cellmotiondetector")
            || topic_lower.contains("motionalarm")
        {
            // Check if it's an end event
            if topic_lower.contains("end") || topic_lower.contains("stop") {
                return NormalizedEventType::MotionEnd;
            }
            return NormalizedEventType::Motion;
        }

        // Audio detection
        if topic_lower.contains("audio") || topic_lower.contains("sound") {
            if topic_lower.contains("end") || topic_lower.contains("stop") {
                return NormalizedEventType::AudioEnd;
            }
            return NormalizedEventType::Audio;
        }

        // Tampering
        if topic_lower.contains("tamper") {
            return NormalizedEventType::Tamper;
        }

        // Line crossing
        if topic_lower.contains("linecross") || topic_lower.contains("tripwire") {
            return NormalizedEventType::LineCrossing;
        }

        // Digital input
        if topic_lower.contains("digitalinput") || topic_lower.contains("input") {
            return NormalizedEventType::DigitalInput;
        }

        // Object detection
        if topic_lower.contains("objectdetect") {
            return NormalizedEventType::ObjectDetected;
        }

        // Face detection
        if topic_lower.contains("face") {
            return NormalizedEventType::FaceDetected;
        }

        // Vehicle detection
        if topic_lower.contains("vehicle") || topic_lower.contains("car") {
            return NormalizedEventType::VehicleDetected;
        }

        // Person detection
        if topic_lower.contains("person") || topic_lower.contains("human") || topic_lower.contains("pedestrian") {
            return NormalizedEventType::PersonDetected;
        }

        NormalizedEventType::Unknown(topic.to_string())
    }

    /// Normalize an ONVIF event to a standard format
    pub fn normalize_onvif_event(
        camera_id: Uuid,
        topic: &str,
        timestamp: DateTime<Utc>,
        data: Option<&serde_json::Value>,
    ) -> NormalizedEvent {
        let event_type = Self::normalize_onvif_topic(topic);

        // Extract confidence if present in data
        let confidence = data.and_then(|d| {
            d.get("confidence")
                .or(d.get("score"))
                .and_then(|v| v.as_f64())
        });

        // Extract bounding box if present
        let bounding_box = data.and_then(|d| {
            if let (Some(x), Some(y), Some(w), Some(h)) = (
                d.get("x").and_then(|v| v.as_f64()),
                d.get("y").and_then(|v| v.as_f64()),
                d.get("width").and_then(|v| v.as_f64()),
                d.get("height").and_then(|v| v.as_f64()),
            ) {
                Some([x, y, w, h])
            } else {
                None
            }
        });

        NormalizedEvent {
            id: Uuid::new_v4(),
            camera_id,
            event_type,
            timestamp,
            end_timestamp: None,
            confidence,
            bounding_box,
            metadata: data.cloned(),
            original_topic: Some(topic.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_state_transitions() {
        let config = HealthConfig {
            offline_threshold: 3,
            online_threshold: 2,
            ..Default::default()
        };

        let mut health = CameraHealth::new(Uuid::new_v4());
        assert_eq!(health.state, HealthState::Unknown);

        // Two successes should bring it online
        for _ in 0..2 {
            health.record_check(
                HealthCheck {
                    camera_id: health.camera_id,
                    timestamp: Utc::now(),
                    success: true,
                    latency_ms: Some(50),
                    error: None,
                    details: None,
                },
                &config,
            );
        }
        assert_eq!(health.state, HealthState::Online);

        // One failure should make it degraded
        health.record_check(
            HealthCheck {
                camera_id: health.camera_id,
                timestamp: Utc::now(),
                success: false,
                latency_ms: None,
                error: Some("Connection refused".into()),
                details: None,
            },
            &config,
        );
        assert_eq!(health.state, HealthState::Degraded);

        // Two more failures should make it offline
        for _ in 0..2 {
            health.record_check(
                HealthCheck {
                    camera_id: health.camera_id,
                    timestamp: Utc::now(),
                    success: false,
                    latency_ms: None,
                    error: Some("Connection refused".into()),
                    details: None,
                },
                &config,
            );
        }
        assert_eq!(health.state, HealthState::Offline);
    }

    #[test]
    fn test_event_normalization() {
        // Test motion detection
        assert_eq!(
            EventNormalizer::normalize_onvif_topic("tns1:RuleEngine/CellMotionDetector/Motion"),
            NormalizedEventType::Motion
        );
        assert_eq!(
            EventNormalizer::normalize_onvif_topic("tns1:VideoSource/MotionAlarm"),
            NormalizedEventType::Motion
        );

        // Test audio detection
        assert_eq!(
            EventNormalizer::normalize_onvif_topic("tns1:AudioAnalytics/Audio/DetectedSound"),
            NormalizedEventType::Audio
        );

        // Test tampering
        assert_eq!(
            EventNormalizer::normalize_onvif_topic("tns1:RuleEngine/TamperDetector/Tamper"),
            NormalizedEventType::Tamper
        );

        // Test line crossing
        assert_eq!(
            EventNormalizer::normalize_onvif_topic("tns1:RuleEngine/LineDetector/LineCrossing"),
            NormalizedEventType::LineCrossing
        );

        // Test person detection
        assert_eq!(
            EventNormalizer::normalize_onvif_topic("tns1:RuleEngine/PersonDetector"),
            NormalizedEventType::PersonDetected
        );

        // Test unknown
        matches!(
            EventNormalizer::normalize_onvif_topic("tns1:SomeWeird/CustomEvent"),
            NormalizedEventType::Unknown(_)
        );
    }

    #[tokio::test]
    async fn test_health_monitor_creation() {
        let monitor = HealthMonitor::new(HealthConfig::default());
        let camera_id = Uuid::new_v4();

        monitor.register_camera(camera_id).await;
        
        let health = monitor.get_camera_health(&camera_id).await;
        assert_eq!(health, Some(HealthState::Unknown));
    }

    #[tokio::test]
    async fn test_health_check_recording() {
        let monitor = HealthMonitor::new(HealthConfig {
            online_threshold: 1,
            ..Default::default()
        });
        let camera_id = Uuid::new_v4();

        monitor.register_camera(camera_id).await;

        // Record a successful check
        monitor
            .record_health_check(HealthCheck {
                camera_id,
                timestamp: Utc::now(),
                success: true,
                latency_ms: Some(50),
                error: None,
                details: None,
            })
            .await
            .unwrap();

        let health = monitor.get_camera_health(&camera_id).await;
        assert_eq!(health, Some(HealthState::Online));
    }
}
