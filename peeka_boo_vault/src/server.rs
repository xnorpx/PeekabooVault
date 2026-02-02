//! HTTP server implementation
//!
//! Axum server following the blue-onyx pattern with graceful shutdown.

use crate::api::{
    AddCameraRequest, ApiResponse, Camera, CameraStatus, DiscoverRequest, DiscoverResponse,
    ProbeRequest, ProbeResponse, ServerStatus, SetCredentialsRequest, SetCredentialsResponse,
    StreamProfile,
};
use crate::config::Config;
use crate::discovery::{discover_devices, get_stream_uris, probe_device};
use crate::state::{CameraCredentials, ServerState};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::{error, info};
use uuid::Uuid;

/// Run the HTTP server with graceful shutdown
pub async fn run_server(
    config: Config,
    cancellation_token: CancellationToken,
) -> anyhow::Result<()> {
    let bind_address = config.server.bind_address;
    let static_dir = config.server.static_dir.clone();

    // Create shared state
    let state = ServerState::new(config, cancellation_token.clone());

    // Build API router
    let api_router = Router::new()
        .route("/status", get(get_status))
        .route("/discovery/scan", post(scan_devices))
        .route("/discovery/devices", get(get_discovered_devices))
        .route("/cameras", get(list_cameras))
        .route("/cameras", post(add_camera))
        .route("/cameras/{id}", get(get_camera))
        .route("/cameras/{id}", delete(delete_camera))
        .route("/cameras/{id}/credentials", post(set_credentials))
        .route("/cameras/{id}/probe", post(probe_camera));

    // Build main router
    let mut app = Router::new().nest("/api", api_router);

    // Add static file serving if configured
    if let Some(ref dir) = static_dir {
        if dir.exists() {
            info!(?dir, "Serving static files");
            app = app.fallback_service(ServeDir::new(dir).append_index_html_on_directories(true));
        } else {
            info!(?dir, "Static directory not found, serving API only");
        }
    }

    // Add middleware
    let app = app
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state.clone());

    // Bind and serve
    let listener = TcpListener::bind(bind_address).await?;
    info!(%bind_address, "Server listening");

    // Auto-discover on startup if configured
    if state.config.discovery.auto_discover {
        let state_clone = state.clone();
        tokio::spawn(async move {
            info!("Running auto-discovery on startup");
            if let Err(e) = run_discovery(&state_clone).await {
                error!(error = %e, "Auto-discovery failed");
            }
        });
    }

    // Serve with graceful shutdown
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            cancellation_token.cancelled().await;
            info!("Shutting down server gracefully");
        })
        .await?;

    Ok(())
}

/// Run a discovery scan and update state
async fn run_discovery(state: &Arc<ServerState>) -> anyhow::Result<()> {
    // Set discovery in progress
    {
        let mut in_progress = state.discovery_in_progress.lock().await;
        if *in_progress {
            anyhow::bail!("Discovery already in progress");
        }
        *in_progress = true;
    }

    // Clear existing discovered devices
    state.clear_discovered_devices().await;

    // Run discovery
    let duration = state.config.discovery.default_duration_secs;
    let result = discover_devices(duration).await;

    // Update state
    match &result {
        Ok(devices) => {
            // Check which devices are already onboarded
            let cameras = state.get_cameras().await;
            let onboarded_addresses: std::collections::HashSet<_> = cameras
                .iter()
                .filter_map(|c| c.onvif_address.clone())
                .collect();

            for mut device in devices.clone() {
                device.is_onboarded = onboarded_addresses.contains(&device.address);
                state.upsert_discovered_device(device).await;
            }
        }
        Err(e) => {
            error!(error = %e, "Discovery failed");
        }
    }

    // Clear discovery in progress
    {
        let mut in_progress = state.discovery_in_progress.lock().await;
        *in_progress = false;
    }

    result.map(|_| ())
}

// === API Handlers ===

/// GET /api/status - Get server status
async fn get_status(State(state): State<Arc<ServerState>>) -> Json<ServerStatus> {
    let (camera_count, cameras_online) = state.get_camera_stats().await;
    let discovery_in_progress = *state.discovery_in_progress.lock().await;

    Json(ServerStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs: state.uptime_secs(),
        camera_count,
        cameras_online,
        discovery_in_progress,
    })
}

/// POST /api/discovery/scan - Start a discovery scan
async fn scan_devices(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<Option<DiscoverRequest>>,
) -> Json<DiscoverResponse> {
    let request = request.unwrap_or_default();
    let start = Instant::now();

    // Check if already in progress
    {
        let in_progress = state.discovery_in_progress.lock().await;
        if *in_progress {
            return Json(DiscoverResponse {
                success: false,
                error: Some("Discovery already in progress".to_string()),
                devices: vec![],
                scan_duration_ms: 0,
            });
        }
    }

    // Set discovery in progress
    {
        let mut in_progress = state.discovery_in_progress.lock().await;
        *in_progress = true;
    }

    // Clear and run discovery
    state.clear_discovered_devices().await;

    let result = discover_devices(request.duration_secs).await;
    let scan_duration_ms = start.elapsed().as_millis() as u64;

    // Update state and return response
    match result {
        Ok(devices) => {
            // Check which devices are already onboarded
            let cameras = state.get_cameras().await;
            let onboarded_addresses: std::collections::HashSet<_> = cameras
                .iter()
                .filter_map(|c| c.onvif_address.clone())
                .collect();

            let mut updated_devices = Vec::new();
            for mut device in devices {
                device.is_onboarded = onboarded_addresses.contains(&device.address);
                state.upsert_discovered_device(device.clone()).await;
                updated_devices.push(device);
            }

            // Clear discovery flag
            {
                let mut in_progress = state.discovery_in_progress.lock().await;
                *in_progress = false;
            }

            Json(DiscoverResponse {
                success: true,
                error: None,
                devices: updated_devices,
                scan_duration_ms,
            })
        }
        Err(e) => {
            // Clear discovery flag
            {
                let mut in_progress = state.discovery_in_progress.lock().await;
                *in_progress = false;
            }

            Json(DiscoverResponse {
                success: false,
                error: Some(e.to_string()),
                devices: vec![],
                scan_duration_ms,
            })
        }
    }
}

/// GET /api/discovery/devices - Get discovered devices
async fn get_discovered_devices(
    State(state): State<Arc<ServerState>>,
) -> Json<ApiResponse<Vec<crate::api::DiscoveredDevice>>> {
    let devices = state.get_discovered_devices().await;
    Json(ApiResponse::ok(devices))
}

/// GET /api/cameras - List all cameras
async fn list_cameras(State(state): State<Arc<ServerState>>) -> Json<ApiResponse<Vec<Camera>>> {
    let cameras = state.get_cameras().await;
    Json(ApiResponse::ok(cameras))
}

/// Common ONVIF URL patterns to try when discovering a camera
const ONVIF_URL_PATTERNS: &[&str] = &[
    "/onvif/device_service",      // Most common: Reolink, Hikvision, Dahua
    "/onvif/device",              // Some cameras
    ":8080/onvif/device_service", // Some cameras use port 8080
    ":554/onvif/device_service",  // RTSP port
    "/onvif/services",            // Alternative path
];

/// POST /api/cameras - Add a new camera
async fn add_camera(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<AddCameraRequest>,
) -> (StatusCode, Json<ApiResponse<Camera>>) {
    let host = request.host.trim();

    // Extract host/IP - strip any scheme if provided
    let clean_host = host
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/');

    // Try to find a working ONVIF URL
    let username = request.username.as_deref();
    let password = request.password.as_deref();

    let mut working_url: Option<url::Url> = None;
    let mut probe_info = None;

    for pattern in ONVIF_URL_PATTERNS {
        let test_url = if pattern.starts_with(':') {
            // Pattern includes port
            format!(
                "http://{}{}",
                clean_host.split(':').next().unwrap_or(clean_host),
                pattern
            )
        } else {
            // Use default port 80
            let base = if clean_host.contains(':') {
                clean_host.to_string()
            } else {
                format!("{}:80", clean_host)
            };
            format!("http://{}{}", base, pattern)
        };

        info!("Trying ONVIF URL: {}", test_url);

        if let Ok(url) = url::Url::parse(&test_url) {
            // Try to probe this URL
            match probe_device(&url, username, password).await {
                Ok(info) => {
                    info!("Found working ONVIF URL: {}", test_url);
                    working_url = Some(url);
                    probe_info = Some(info);
                    break;
                }
                Err(e) => {
                    info!("ONVIF URL {} failed: {}", test_url, e);
                }
            }
        }
    }

    let Some(onvif_url) = working_url else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::err(format!(
                "Could not connect to camera at {}. Tried common ONVIF URL patterns. Make sure ONVIF is enabled on your camera.",
                clean_host
            ))),
        );
    };

    let info = probe_info.unwrap();

    let mut camera = Camera {
        id: Uuid::new_v4(),
        name: request.name,
        onvif_address: Some(clean_host.to_string()),
        onvif_url: Some(onvif_url.clone()),
        main_stream_uri: None,
        sub_stream_uri: None,
        stream_profiles: Vec::new(),
        has_credentials: username.is_some() && password.is_some(),
        status: CameraStatus::Online,
        last_seen: Some(chrono::Utc::now()),
        manufacturer: info.manufacturer,
        model: info.model,
        firmware_version: info.firmware_version,
        serial_number: info.serial_number,
        hardware_id: info.hardware_id,
    };

    // If credentials were provided, store them and get stream URIs
    if let (Some(user), Some(pass)) = (username, password) {
        state
            .set_credentials(
                camera.id,
                CameraCredentials {
                    username: user.to_string(),
                    password: pass.to_string(),
                },
            )
            .await;

        // Try to get stream URIs
        match get_stream_uris(&onvif_url, Some(user), Some(pass)).await {
            Ok(profiles) => {
                // Convert to API StreamProfile type
                camera.stream_profiles = profiles
                    .iter()
                    .map(|p| StreamProfile {
                        token: p.profile_token.clone(),
                        name: p.profile_name.clone(),
                        stream_uri: p.stream_uri.clone(),
                        encoding: p.encoding.clone(),
                        width: p.width,
                        height: p.height,
                        frame_rate: p.frame_rate,
                        bitrate_kbps: p.bitrate_limit,
                        quality: p.quality,
                        gop_length: p.gov_length,
                        codec_profile: p.h264_profile.clone(),
                        encoding_interval: p.encoding_interval,
                        guaranteed_frame_rate: p.guaranteed_frame_rate,
                    })
                    .collect();

                // Get main stream (highest resolution)
                if let Some(main) = profiles
                    .iter()
                    .max_by_key(|p| p.width.unwrap_or(0) * p.height.unwrap_or(0))
                {
                    camera.main_stream_uri = Some(main.stream_uri.clone());
                }
                // Get sub stream (second profile or lowest resolution)
                if profiles.len() > 1
                    && let Some(sub) = profiles
                        .iter()
                        .min_by_key(|p| p.width.unwrap_or(0) * p.height.unwrap_or(0))
                {
                    camera.sub_stream_uri = Some(sub.stream_uri.clone());
                }
            }
            Err(e) => {
                info!("Could not get stream URIs: {}", e);
            }
        }
    }

    let id = state.add_camera(camera.clone()).await;
    let camera = state.get_camera(&id).await.unwrap();

    (StatusCode::CREATED, Json(ApiResponse::ok(camera)))
}

/// GET /api/cameras/:id - Get a camera by ID
async fn get_camera(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<Camera>>, StatusCode> {
    match state.get_camera(&id).await {
        Some(camera) => Ok(Json(ApiResponse::ok(camera))),
        None => Err(StatusCode::NOT_FOUND),
    }
}

/// DELETE /api/cameras/:id - Delete a camera
async fn delete_camera(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<Camera>>, StatusCode> {
    match state.delete_camera(&id).await {
        Some(camera) => Ok(Json(ApiResponse::ok(camera))),
        None => Err(StatusCode::NOT_FOUND),
    }
}

/// POST /api/cameras/:id/credentials - Set camera credentials
async fn set_credentials(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<Uuid>,
    Json(request): Json<SetCredentialsRequest>,
) -> Result<Json<SetCredentialsResponse>, StatusCode> {
    // Check camera exists
    let mut camera = match state.get_camera(&id).await {
        Some(c) => c,
        None => return Err(StatusCode::NOT_FOUND),
    };

    // Store credentials
    state
        .set_credentials(
            id,
            CameraCredentials {
                username: request.username.clone(),
                password: request.password.clone(),
            },
        )
        .await;

    // Try to probe the camera with new credentials
    if let Some(ref url) = camera.onvif_url {
        match probe_device(url, Some(&request.username), Some(&request.password)).await {
            Ok(info) => {
                camera.manufacturer = info.manufacturer;
                camera.model = info.model;
                camera.firmware_version = info.firmware_version;
                camera.status = CameraStatus::Online;
                camera.last_seen = Some(chrono::Utc::now());
                camera.has_credentials = true;
                state.update_camera(camera.clone()).await;
            }
            Err(e) => {
                camera.status = CameraStatus::Unauthorized;
                camera.has_credentials = true;
                state.update_camera(camera.clone()).await;

                return Ok(Json(SetCredentialsResponse {
                    success: false,
                    error: Some(format!("Failed to validate credentials: {e}")),
                    camera: Some(camera),
                }));
            }
        }
    }

    Ok(Json(SetCredentialsResponse {
        success: true,
        error: None,
        camera: Some(camera),
    }))
}

/// POST /api/cameras/:id/probe - Probe a camera for device info and streams
async fn probe_camera(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<Uuid>,
    Json(request): Json<Option<ProbeRequest>>,
) -> Result<Json<ProbeResponse>, StatusCode> {
    let request = request.unwrap_or_default();

    // Get camera
    let mut camera = match state.get_camera(&id).await {
        Some(c) => c,
        None => return Err(StatusCode::NOT_FOUND),
    };

    let url = match &camera.onvif_url {
        Some(u) => u.clone(),
        None => {
            return Ok(Json(ProbeResponse {
                success: false,
                error: Some("Camera has no ONVIF URL configured".to_string()),
                camera: Some(camera),
                profiles: vec![],
            }));
        }
    };

    // Get credentials if available
    let creds = state.get_credentials(&id).await;
    let (username, password) = match &creds {
        Some(c) => (Some(c.username.as_str()), Some(c.password.as_str())),
        None => (None, None),
    };

    camera.status = CameraStatus::Probing;
    state.update_camera(camera.clone()).await;

    // Probe device info
    match probe_device(&url, username, password).await {
        Ok(info) => {
            camera.manufacturer = info.manufacturer;
            camera.model = info.model;
            camera.firmware_version = info.firmware_version;
            camera.status = CameraStatus::Online;
            camera.last_seen = Some(chrono::Utc::now());
        }
        Err(e) => {
            camera.status = if creds.is_some() {
                CameraStatus::Unauthorized
            } else {
                CameraStatus::Offline
            };
            state.update_camera(camera.clone()).await;

            return Ok(Json(ProbeResponse {
                success: false,
                error: Some(format!("Failed to probe device: {e}")),
                camera: Some(camera),
                profiles: vec![],
            }));
        }
    }

    // Get stream URIs if requested
    let mut profiles = Vec::new();
    if request.refresh_streams {
        match get_stream_uris(&url, username, password).await {
            Ok(streams) => {
                // Update camera stream profiles
                camera.stream_profiles = streams
                    .iter()
                    .map(|s| StreamProfile {
                        token: s.profile_token.clone(),
                        name: s.profile_name.clone(),
                        stream_uri: s.stream_uri.clone(),
                        encoding: s.encoding.clone(),
                        width: s.width,
                        height: s.height,
                        frame_rate: s.frame_rate,
                        bitrate_kbps: s.bitrate_limit,
                        quality: s.quality,
                        gop_length: s.gov_length,
                        codec_profile: s.h264_profile.clone(),
                        encoding_interval: s.encoding_interval,
                        guaranteed_frame_rate: s.guaranteed_frame_rate,
                    })
                    .collect();

                // Set main stream (highest resolution)
                if let Some(main) = streams
                    .iter()
                    .max_by_key(|p| p.width.unwrap_or(0) * p.height.unwrap_or(0))
                {
                    camera.main_stream_uri = Some(main.stream_uri.clone());
                }
                // Set sub stream (lowest resolution)
                if streams.len() > 1
                    && let Some(sub) = streams
                        .iter()
                        .min_by_key(|p| p.width.unwrap_or(0) * p.height.unwrap_or(0))
                {
                    camera.sub_stream_uri = Some(sub.stream_uri.clone());
                }

                profiles = camera.stream_profiles.clone();
            }
            Err(e) => {
                // Don't fail the whole probe, just log warning
                tracing::warn!(error = %e, "Failed to get stream URIs");
            }
        }
    }

    state.update_camera(camera.clone()).await;

    Ok(Json(ProbeResponse {
        success: true,
        error: None,
        camera: Some(camera),
        profiles,
    }))
}
