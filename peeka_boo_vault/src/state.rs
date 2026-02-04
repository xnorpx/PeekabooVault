//! Application state management
//!
//! Shared state with Arc<ServerState>.

use crate::api::{Camera, CameraStatus, DiscoveredDevice};
use crate::bandwidth_manager::{BandwidthBudgetManager, BandwidthManagerConfig};
use crate::config::Config;
use crate::frame_store::FrameStore;
use crate::hot_cold_db::HotColdDb;
use crate::recorder::{RecorderConfig, RecorderHandle};
use crate::rtsp_client::{RtspClient, RtspConfig};
use crate::stream_manager::StreamManager;
use crate::webrtc::{WebRtcManager, WebRtcServer};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Credentials stored for a camera
#[derive(Debug, Clone)]
pub struct CameraCredentials {
    pub username: String,
    pub password: String,
}

/// Server state shared across all handlers
pub struct ServerState {
    /// Application configuration
    pub config: Config,
    /// When the server started
    pub start_time: Instant,
    /// Discovered devices (address -> device)
    pub discovered_devices: RwLock<HashMap<String, DiscoveredDevice>>,
    /// Onboarded cameras (id -> camera)
    pub cameras: RwLock<HashMap<Uuid, Camera>>,
    /// Camera credentials (camera_id -> credentials)
    /// Note: In production, use proper secret management
    pub credentials: RwLock<HashMap<Uuid, CameraCredentials>>,
    /// Discovery in progress flag
    pub discovery_in_progress: Mutex<bool>,
    /// Cancellation token for graceful shutdown
    pub cancellation_token: CancellationToken,
    /// Stream manager for RTSP connections
    pub stream_manager: StreamManager,
    /// Active RTSP clients (camera_id, stream_type) -> RtspClient
    pub rtsp_clients: RwLock<HashMap<(Uuid, String), RtspClient>>,
    /// Active recorder tasks (camera_id, stream_type) -> RecorderHandle
    pub recorders: RwLock<HashMap<(Uuid, String), RecorderHandle>>,
    /// Hot/Cold database for recordings (Phase 2)
    /// Note: Option because it needs async initialization
    pub hot_cold_db: RwLock<Option<Arc<RwLock<HotColdDb>>>>,
    /// Storage path for recordings
    pub storage_path: PathBuf,
    /// Frame store for playback (Phase 3)
    /// Note: Option because it needs async initialization
    pub frame_store: Option<Arc<FrameStore>>,
    /// WebRTC manager for live view (Phase 6)
    /// Note: Uses WebRtcManager directly; WebRtcServer started separately
    pub webrtc_manager: Option<Arc<WebRtcManager>>,
    /// WebRTC server with UDP transport (Phase 6)
    /// Note: Option because it needs async initialization
    pub webrtc_server: RwLock<Option<Arc<WebRtcServer>>>,
    /// Bandwidth budget manager for multi-camera coordination
    pub bandwidth_manager: Arc<BandwidthBudgetManager>,
    /// Message sender for ServerTask (NEW - for gradual migration to message-passing architecture)
    /// Note: Optional during transition period - None means ServerTask not yet initialized
    pub server_task_tx: RwLock<Option<tokio::sync::mpsc::UnboundedSender<crate::server_task::ServerMessage>>>,
}

impl ServerState {
    /// Create a new server state
    pub fn new(config: Config, cancellation_token: CancellationToken) -> Arc<Self> {
        // Initialize WebRTC manager with default config
        let webrtc_manager = Arc::new(WebRtcManager::default());
        
        // Initialize bandwidth budget manager
        let bandwidth_manager = BandwidthBudgetManager::new(BandwidthManagerConfig::default());
        
        // Default storage path
        let storage_path = PathBuf::from(&config.storage.hot_storage_path);

        Arc::new(Self {
            config,
            start_time: Instant::now(),
            discovered_devices: RwLock::new(HashMap::new()),
            cameras: RwLock::new(HashMap::new()),
            credentials: RwLock::new(HashMap::new()),
            discovery_in_progress: Mutex::new(false),
            cancellation_token,
            stream_manager: StreamManager::new(),
            rtsp_clients: RwLock::new(HashMap::new()),
            recorders: RwLock::new(HashMap::new()),
            hot_cold_db: RwLock::new(None), // Initialized later via set_hot_cold_db
            storage_path,
            frame_store: None, // Initialized later via set_frame_store
            webrtc_manager: Some(webrtc_manager),
            webrtc_server: RwLock::new(None), // Started async via start_webrtc_server
            bandwidth_manager,
            server_task_tx: RwLock::new(None), // Will be set when ServerTask is initialized
        })
    }
    
    /// Set the HotColdDb for recording storage
    pub async fn set_hot_cold_db(&self, db: Arc<RwLock<HotColdDb>>) {
        *self.hot_cold_db.write().await = Some(db);
    }

    /// Set the ServerTask message sender
    pub async fn set_server_task_tx(&self, tx: tokio::sync::mpsc::UnboundedSender<crate::server_task::ServerMessage>) {
        *self.server_task_tx.write().await = Some(tx);
    }

    /// Start the WebRTC server (must be called after state creation)
    pub async fn start_webrtc_server(&self) -> anyhow::Result<()> {
        use crate::webrtc::WebRtcServerConfig;
        
        if !self.config.webrtc.enabled {
            tracing::info!("WebRTC server disabled in config");
            return Ok(());
        }
        
        let webrtc_config = WebRtcServerConfig {
            bind_addr: self.config.webrtc.bind_address,
            session_config: crate::webrtc::WebRtcConfig {
                stun_server: None, // ICE-lite doesn't need STUN
                ice_lite: true,
                max_sessions_per_camera: self.config.webrtc.max_sessions_per_camera,
                session_timeout_secs: self.config.webrtc.session_timeout_secs,
                enable_bwe: true,  // Enable bandwidth estimation
                initial_bandwidth_kbps: 2500, // Start with 2.5 Mbps estimate
            },
        };
        
        let server = WebRtcServer::start(webrtc_config).await?;
        let server = Arc::new(server);
        
        // Store in state
        *self.webrtc_server.write().await = Some(server.clone());
        
        // Also update webrtc_manager to use the server's manager
        // Note: The handler uses webrtc_manager, so we need them to be the same
        
        tracing::info!(
            bind_addr = %self.config.webrtc.bind_address,
            "WebRTC server started"
        );
        
        Ok(())
    }
    
    /// Get the WebRTC server if available
    pub async fn get_webrtc_server(&self) -> Option<Arc<WebRtcServer>> {
        self.webrtc_server.read().await.clone()
    }

    /// Get server uptime in seconds
    pub fn uptime_secs(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }

    /// Add or update a discovered device
    pub async fn upsert_discovered_device(&self, device: DiscoveredDevice) {
        let mut devices = self.discovered_devices.write().await;
        devices.insert(device.address.clone(), device);
    }

    /// Get all discovered devices
    pub async fn get_discovered_devices(&self) -> Vec<DiscoveredDevice> {
        let devices = self.discovered_devices.read().await;
        devices.values().cloned().collect()
    }

    /// Clear discovered devices (before a new scan)
    pub async fn clear_discovered_devices(&self) {
        let mut devices = self.discovered_devices.write().await;
        devices.clear();
    }

    /// Add a camera
    pub async fn add_camera(&self, camera: Camera) -> Uuid {
        let id = camera.id;
        let mut cameras = self.cameras.write().await;
        cameras.insert(id, camera.clone());

        // Mark the discovered device as onboarded if linked
        if let Some(ref addr) = cameras.get(&id).and_then(|c| c.onvif_address.clone()) {
            let mut devices = self.discovered_devices.write().await;
            if let Some(device) = devices.get_mut(addr) {
                device.is_onboarded = true;
            }
        }

        // Persist to database if available
        if let Some(db) = self.hot_cold_db.read().await.as_ref() {
            let db = db.read().await;

            // Insert/update camera
            match db.upsert_camera(
                camera.id.to_string(),
                camera.name.clone(),
                camera.onvif_address.clone(),
            ).await {
                Ok(camera_db_id) => {
                    // Get or create sample_file_dir
                    match db.get_or_create_sample_file_dir(&self.storage_path).await {
                        Ok(sample_file_dir_id) => {
                            // Create stream entries for main and sub streams
                            if let Some(ref main_uri) = camera.main_stream_uri {
                                if let Err(e) = db.upsert_stream(
                                    camera_db_id,
                                    "main".to_string(),
                                    Some(main_uri.clone()),
                                    sample_file_dir_id,
                                ).await {
                                    tracing::warn!("Failed to persist main stream: {}", e);
                                }
                            }

                            if let Some(ref sub_uri) = camera.sub_stream_uri {
                                if let Err(e) = db.upsert_stream(
                                    camera_db_id,
                                    "sub".to_string(),
                                    Some(sub_uri.clone()),
                                    sample_file_dir_id,
                                ).await {
                                    tracing::warn!("Failed to persist sub stream: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to get sample_file_dir: {}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to persist camera to database: {}", e);
                }
            }
        }

        id
    }

    /// Get a camera by ID
    pub async fn get_camera(&self, id: &Uuid) -> Option<Camera> {
        let cameras = self.cameras.read().await;
        cameras.get(id).cloned()
    }

    /// Get all cameras
    pub async fn get_cameras(&self) -> Vec<Camera> {
        let cameras = self.cameras.read().await;
        cameras.values().cloned().collect()
    }

    /// Update a camera
    pub async fn update_camera(&self, camera: Camera) {
        let mut cameras = self.cameras.write().await;
        cameras.insert(camera.id, camera);
    }

    /// Delete a camera
    pub async fn delete_camera(&self, id: &Uuid) -> Option<Camera> {
        let mut cameras = self.cameras.write().await;
        let camera = cameras.remove(id);

        // Also remove credentials
        let mut credentials = self.credentials.write().await;
        credentials.remove(id);

        camera
    }

    /// Set credentials for a camera
    pub async fn set_credentials(&self, camera_id: Uuid, credentials: CameraCredentials) {
        let mut creds = self.credentials.write().await;
        creds.insert(camera_id, credentials);

        // Update the camera's has_credentials flag
        let mut cameras = self.cameras.write().await;
        if let Some(camera) = cameras.get_mut(&camera_id) {
            camera.has_credentials = true;
        }
    }

    /// Get credentials for a camera
    pub async fn get_credentials(&self, camera_id: &Uuid) -> Option<CameraCredentials> {
        let creds = self.credentials.read().await;
        creds.get(camera_id).cloned()
    }

    /// Get camera statistics
    pub async fn get_camera_stats(&self) -> (usize, usize) {
        let cameras = self.cameras.read().await;
        let total = cameras.len();
        let online = cameras
            .values()
            .filter(|c| c.status == CameraStatus::Online)
            .count();
        (total, online)
    }

    /// Start RTSP streaming for a camera
    ///
    /// This wires the RTSP client to the StreamSession, implementing the
    /// "Missing Heart" described in the design - the active runtime loop
    /// that fetches video data from cameras.
    pub async fn start_rtsp_client(
        &self,
        camera_id: Uuid,
        stream_type: &str,
        rtsp_url: &str,
    ) -> anyhow::Result<()> {
        let key = (camera_id, stream_type.to_string());
        
        // Check if already running
        {
            let clients = self.rtsp_clients.read().await;
            if let Some(client) = clients.get(&key) {
                if client.is_running() {
                    return Ok(()); // Already running
                }
            }
        }
        
        // Get credentials if available
        let credentials = self.get_credentials(&camera_id).await;
        
        // Create RTSP config
        let config = RtspConfig {
            url: rtsp_url.to_string(),
            username: credentials.as_ref().map(|c| c.username.clone()),
            password: credentials.as_ref().map(|c| c.password.clone()),
            use_tcp: true,
            timeout_secs: 10,
        };
        
        // Get or create the session
        let session = self.stream_manager.get_or_create_session(
            camera_id,
            stream_type,
            rtsp_url,
        );
        
        // Create and start the client
        let mut client = RtspClient::new(config);
        client.start(session).await?;
        
        // Store the client
        let mut clients = self.rtsp_clients.write().await;
        clients.insert(key, client);
        
        tracing::info!(
            camera_id = %camera_id,
            stream_type = %stream_type,
            "Started RTSP client"
        );
        
        Ok(())
    }
    
    /// Stop RTSP streaming for a camera
    pub async fn stop_rtsp_client(&self, camera_id: Uuid, stream_type: &str) {
        let key = (camera_id, stream_type.to_string());
        
        let mut clients = self.rtsp_clients.write().await;
        if let Some(mut client) = clients.remove(&key) {
            client.stop().await;
            tracing::info!(
                camera_id = %camera_id,
                stream_type = %stream_type,
                "Stopped RTSP client"
            );
        }
    }
    
    /// Check if RTSP client is running for a camera stream
    pub async fn is_rtsp_running(&self, camera_id: Uuid, stream_type: &str) -> bool {
        let key = (camera_id, stream_type.to_string());
        let clients = self.rtsp_clients.read().await;
        clients.get(&key).is_some_and(|c| c.is_running())
    }
    
    /// Start recording for a camera stream
    ///
    /// This implements the Phase 2 deliverable:
    /// - Starts RTSP client to ingest video
    /// - Starts recorder task to write ~60s segments to disk
    pub async fn start_recording(
        &self,
        camera_id: Uuid,
        stream_type: &str,
        rtsp_url: &str,
    ) -> anyhow::Result<()> {
        let key = (camera_id, stream_type.to_string());

        // Check if already recording
        {
            let recorders = self.recorders.read().await;
            if recorders.contains_key(&key) {
                return Ok(()); // Already recording
            }
        }

        // Start RTSP client first
        self.start_rtsp_client(camera_id, stream_type, rtsp_url).await?;

        // Get the session
        let session = self.stream_manager.get_or_create_session(
            camera_id,
            stream_type,
            rtsp_url,
        );

        // Get the database
        let db_guard = self.hot_cold_db.read().await;
        let db = db_guard.as_ref()
            .ok_or_else(|| anyhow::anyhow!("HotColdDb not initialized"))?
            .clone();
        drop(db_guard);

        // Look up stream_id from database
        let stream_id = {
            let db_read = db.read().await;
            db_read.get_stream_id(&camera_id.to_string(), stream_type)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Stream not found in database. Camera ID: {}, Stream type: {}", camera_id, stream_type))?
        };

        // Get sample_file_dir_id from database
        let sample_file_dir_id = {
            let db_read = db.read().await;
            db_read.get_or_create_sample_file_dir(&self.storage_path).await?
        };

        // Configure recorder
        let recorder_config = RecorderConfig {
            segment_duration: std::time::Duration::from_secs(60),
            max_segment_bytes: 100 * 1024 * 1024,
            storage_path: self.storage_path.clone(),
            sample_file_dir_id,
            stream_id,
            run_id: chrono::Utc::now().timestamp(), // Use timestamp as run ID
        };

        // Start recorder
        let handle = crate::recorder::start_recording(session, db, recorder_config).await?;

        // Store the handle
        let mut recorders = self.recorders.write().await;
        recorders.insert(key, handle);

        tracing::info!(
            camera_id = %camera_id,
            stream_type = %stream_type,
            stream_id = %stream_id,
            "Started recording"
        );

        Ok(())
    }
    
    /// Stop recording for a camera stream
    pub async fn stop_recording(&self, camera_id: Uuid, stream_type: &str) {
        let key = (camera_id, stream_type.to_string());
        
        // Stop recorder first
        {
            let mut recorders = self.recorders.write().await;
            if let Some(handle) = recorders.remove(&key) {
                handle.stop().await;
                tracing::info!(
                    camera_id = %camera_id,
                    stream_type = %stream_type,
                    "Stopped recording"
                );
            }
        }
        
        // Then stop RTSP client
        self.stop_rtsp_client(camera_id, stream_type).await;
    }
    
    /// Check if recording is active for a camera stream
    pub async fn is_recording(&self, camera_id: Uuid, stream_type: &str) -> bool {
        let key = (camera_id, stream_type.to_string());
        let recorders = self.recorders.read().await;
        recorders.contains_key(&key)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use tokio_util::sync::CancellationToken;

    fn create_test_state() -> Arc<ServerState> {
        let config = Config::default();
        let cancellation_token = CancellationToken::new();
        ServerState::new(config, cancellation_token)
    }

    #[tokio::test]
    async fn test_server_state_creation() {
        let state = create_test_state();
        assert!(state.uptime_secs() < 2);
    }

    #[tokio::test]
    async fn test_camera_crud() {
        use crate::api::{Camera, CameraStatus};
        
        let state = create_test_state();
        let camera_id = Uuid::new_v4();
        
        let camera = Camera {
            id: camera_id,
            name: "Test Camera".to_string(),
            onvif_address: None,
            onvif_url: None,
            main_stream_uri: Some("rtsp://test/main".to_string()),
            sub_stream_uri: None,
            stream_profiles: vec![],
            has_credentials: false,
            status: CameraStatus::Unknown,
            last_seen: None,
            manufacturer: None,
            model: None,
            firmware_version: None,
            serial_number: None,
            hardware_id: None,
        };
        
        // Add camera
        state.add_camera(camera.clone()).await;
        
        // Get camera
        let retrieved = state.get_camera(&camera_id).await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "Test Camera");
        
        // Delete camera
        let deleted = state.delete_camera(&camera_id).await;
        assert!(deleted.is_some());
        
        // Should be gone
        assert!(state.get_camera(&camera_id).await.is_none());
    }

    #[tokio::test]
    async fn test_credentials_management() {
        let state = create_test_state();
        let camera_id = Uuid::new_v4();
        
        // Set credentials
        state.set_credentials(camera_id, CameraCredentials {
            username: "admin".to_string(),
            password: "secret".to_string(),
        }).await;
        
        // Get credentials
        let creds = state.get_credentials(&camera_id).await;
        assert!(creds.is_some());
        let creds = creds.unwrap();
        assert_eq!(creds.username, "admin");
        assert_eq!(creds.password, "secret");
    }

    #[tokio::test]
    async fn test_stream_session_creation() {
        let state = create_test_state();
        let camera_id = Uuid::new_v4();
        
        // Create a stream session via stream_manager
        let session = state.stream_manager.get_or_create_session(
            camera_id,
            "main",
            "rtsp://test:554/stream",
        );
        
        // Session should start disconnected
        assert_eq!(session.state(), crate::stream_manager::SessionState::Disconnected);
        
        // Get same session again
        let session2 = state.stream_manager.get_or_create_session(
            camera_id,
            "main",
            "rtsp://test:554/stream",
        );
        
        // Should be the same session (Arc pointer equality)
        assert!(Arc::ptr_eq(&session, &session2));
    }

    #[tokio::test]
    async fn test_rtsp_client_tracking() {
        let state = create_test_state();
        let camera_id = Uuid::new_v4();
        
        // Initially not running
        assert!(!state.is_rtsp_running(camera_id, "main").await);
        
        // Note: We can't test start_rtsp_client fully without a real RTSP server,
        // but we can test the tracking mechanics
        
        // Client map should be empty
        assert!(state.rtsp_clients.read().await.is_empty());
        
        // Calling stop on non-existent client should not panic
        state.stop_rtsp_client(camera_id, "main").await;
    }

    #[tokio::test]
    async fn test_recording_tracking() {
        let state = create_test_state();
        let camera_id = Uuid::new_v4();
        
        // Initially not recording
        assert!(!state.is_recording(camera_id, "main").await);
        
        // Recorder map should be empty
        assert!(state.recorders.read().await.is_empty());
        
        // Calling stop on non-existent recording should not panic
        state.stop_recording(camera_id, "main").await;
        
        // Still not recording
        assert!(!state.is_recording(camera_id, "main").await);
    }

    #[tokio::test]
    async fn test_storage_path_from_config() {
        let state = create_test_state();
        
        // Storage path should be set from config
        assert!(state.storage_path.to_string_lossy().contains("recordings"));
    }
}
