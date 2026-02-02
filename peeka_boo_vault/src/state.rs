//! Application state management
//!
//! Shared state following the blue-onyx pattern with Arc<ServerState>.

use crate::api::{Camera, CameraStatus, DiscoveredDevice};
use crate::config::Config;
use std::collections::HashMap;
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
}

impl ServerState {
    /// Create a new server state
    pub fn new(config: Config, cancellation_token: CancellationToken) -> Arc<Self> {
        Arc::new(Self {
            config,
            start_time: Instant::now(),
            discovered_devices: RwLock::new(HashMap::new()),
            cameras: RwLock::new(HashMap::new()),
            credentials: RwLock::new(HashMap::new()),
            discovery_in_progress: Mutex::new(false),
            cancellation_token,
        })
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
        cameras.insert(id, camera);

        // Mark the discovered device as onboarded if linked
        if let Some(ref addr) = cameras.get(&id).and_then(|c| c.onvif_address.clone()) {
            let mut devices = self.discovered_devices.write().await;
            if let Some(device) = devices.get_mut(addr) {
                device.is_onboarded = true;
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
}
