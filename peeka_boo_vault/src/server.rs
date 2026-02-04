//! HTTP server implementation
//!
//! Axum server following the blue-onyx pattern with graceful shutdown.

use crate::api::{
    AddCameraRequest, ApiResponse, Camera, CameraStatus, ClipExportInfo, ClipExportRequest,
    DiscoverRequest, DiscoverResponse, FrameRangeRequest, FrameRangeResponse, IceCandidateRequest,
    IceCandidateResponse, KeyframeTimeline, MultiCameraTimeline, MultiReplayRequest,
    MultiReplayResponse, PlaybackFrame, ProbeRequest, ProbeResponse, RecordingRequest,
    RecordingStatus, RecordingsQuery, RecordingsResponse, ReplayCameraInfo, SeekDirection,
    SeekRequest, ServerStatus, SetCredentialsRequest, SetCredentialsResponse, StreamProfile,
    StreamState, StreamStats, StreamType, TimelineSegment, TimelineSegmentInfo, TimelineSummary,
    WebRtcAnswerResponse, WebRtcOfferRequest, WebRtcSessionInfo, WebRtcSessionsResponse,
};
use crate::clip_export::{export_clip_data, prepare_clip};
use crate::config::Config;
use crate::discovery::{discover_devices, get_stream_uris, probe_device};
use crate::frame_store::SeekDirection as FrameSeekDirection;
use crate::state::{CameraCredentials, ServerState};
use crate::stream_manager::SessionState;
use crate::ui::ReplayCameraSlot;
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{delete, get, patch, post},
};
use chrono::{Duration, Utc};
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn};
use uuid::Uuid;

/// Run the HTTP server with graceful shutdown
pub async fn run_server(
    config: Config,
    cancellation_token: CancellationToken,
) -> anyhow::Result<()> {
    let bind_address = config.server.bind_address;
    let static_dir = config.server.static_dir.clone();

    // Create shared state
    let state = ServerState::new(config.clone(), cancellation_token.clone());

    // Initialize database and frame store
    let hot_db_path = config.storage.hot_db_path.clone();
    let cold_db_path = config.storage.cold_db_path.clone();
    let hot_storage_path = config.storage.hot_storage_path.clone();
    let cold_storage_path = config.storage.cold_storage_path.clone();

    // Ensure storage directories exist
    tokio::fs::create_dir_all(&hot_storage_path).await?;
    tokio::fs::create_dir_all(&cold_storage_path).await?;

    // Open databases
    let hot_cold_db = crate::HotColdDb::open(
        &hot_db_path,
        &cold_db_path,
        &hot_storage_path,
        &cold_storage_path,
        config.storage.retention.clone(),
    )
    .await?;
    let hot_cold_db = Arc::new(tokio::sync::RwLock::new(hot_cold_db));

    // Set database in state
    state.set_hot_cold_db(hot_cold_db.clone()).await;

    // Initialize frame store
    let frame_store_db_path = config.storage.hot_db_path.parent()
        .unwrap_or(std::path::Path::new("."))
        .join("frame_store.db");
    let frame_store = crate::FrameStore::open(&frame_store_db_path, &hot_storage_path).await?;
    let frame_store = Arc::new(frame_store);

    // Create and start ServerTask
    let (server_task, server_task_tx) = crate::ServerTask::new(
        config.clone(),
        hot_cold_db.clone(),
        frame_store.clone(),
        cancellation_token.clone(),
    );

    // Set ServerTask message sender in state
    state.set_server_task_tx(server_task_tx.clone()).await;

    // Spawn ServerTask event loop
    tokio::spawn(server_task.run());

    info!("ServerTask initialized and started");

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
        .route("/cameras/{id}/probe", post(probe_camera))
        // Recording routes
        .route("/cameras/{id}/recording/start", post(start_recording))
        .route("/cameras/{id}/recording/stop", post(stop_recording))
        .route("/cameras/{id}/recording/status", get(get_recording_status))
        .route("/cameras/{id}/stream/stats", get(get_stream_stats))
        .route("/recordings", get(list_recordings))
        // Clip export routes
        .route("/clips/info", post(get_clip_info))
        .route("/clips/export", post(export_clip))
        // Playback routes
        .route("/cameras/{id}/playback/seek", post(seek_to_keyframe))
        .route("/cameras/{id}/playback/frames", post(get_frames))
        .route("/cameras/{id}/timeline", get(get_timeline))
        .route("/cameras/{id}/timeline/keyframes", get(get_keyframe_timeline))
        // WebRTC routes
        .route("/webrtc/offer", post(webrtc_offer))
        .route("/webrtc/ice-candidate", post(webrtc_ice_candidate))
        .route("/webrtc/multi-replay", post(webrtc_multi_replay))
        .route("/webrtc/sessions", get(list_webrtc_sessions))
        .route("/webrtc/sessions/{id}", delete(close_webrtc_session))
        // Storage/Retention routes
        .route("/storage/stats", get(get_storage_stats))
        .route("/storage/retention", get(get_retention_config))
        .route("/storage/migration/status", get(get_migration_status))
        .route("/storage/migration/trigger", post(trigger_migration))
        .route("/storage/usage", get(get_storage_usage))
        // Config routes
        .route("/config", get(get_config))
        .route("/config", patch(update_config))
        .route("/timeline", post(query_timeline))
        // Health/Events routes
        .route("/health", get(get_all_health))
        .route("/health/{id}", get(get_camera_health))
        .route("/health/{id}/history", get(get_health_history))
        .route("/events", get(list_events))
        .route("/cameras/{id}/events", get(get_camera_events))
        .route("/cameras/{id}/combined-timeline", get(get_combined_timeline))
        .route("/timeline-ui/{camera_id}", get(timeline_ui_page))
        // Multi-camera replay
        .route("/multi-replay", post(create_multi_replay));

    // Build main router - API only, SvelteKit handles all UI
    let mut app = Router::new()
        .nest("/api", api_router);

    // Add static file serving for SvelteKit app
    if let Some(ref dir) = static_dir {
        if dir.exists() {
            info!(?dir, "Serving SvelteKit UI");
            app = app.fallback_service(ServeDir::new(dir).append_index_html_on_directories(true));
        } else {
            anyhow::bail!("Static directory not found: {:?}", dir);
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

    // Start WebRTC server if enabled
    if state.config.webrtc.enabled {
        let state_clone = state.clone();
        tokio::spawn(async move {
            info!("Starting WebRTC server");
            if let Err(e) = state_clone.start_webrtc_server().await {
                error!(error = %e, "Failed to start WebRTC server");
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
    
    // Get stream manager stats
    let streams_recording = state.stream_manager.list_sessions()
        .iter()
        .filter(|(_, _, s)| *s == SessionState::Active)
        .count();

    Json(ServerStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs: state.uptime_secs(),
        camera_count,
        cameras_online,
        discovery_in_progress,
        streams_recording,
        total_recordings: 0, // TODO: Query from database
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
    // Try message-passing if ServerTask is available
    let server_task_tx_opt = state.server_task_tx.read().await.clone();

    if let Some(tx) = server_task_tx_opt {
        // Message-passing path
        let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();

        if tx.send(crate::server_task::ServerMessage::ListDiscoveredDevices {
            respond_to: respond_tx,
        }).is_err() {
            let devices = state.get_discovered_devices().await;
            return Json(ApiResponse::ok(devices));
        }

        match respond_rx.await {
            Ok(devices) => return Json(ApiResponse::ok(devices)),
            Err(_) => {
                let devices = state.get_discovered_devices().await;
                return Json(ApiResponse::ok(devices));
            }
        }
    }

    // Fall back to direct state access
    let devices = state.get_discovered_devices().await;
    Json(ApiResponse::ok(devices))
}

/// GET /api/cameras - List all cameras
async fn list_cameras(State(state): State<Arc<ServerState>>) -> Json<ApiResponse<Vec<Camera>>> {
    // Try message-passing if ServerTask is available, otherwise fall back to direct state access
    let server_task_tx_opt = state.server_task_tx.read().await.clone();

    if let Some(tx) = server_task_tx_opt {
        // Message-passing path (NEW architecture)
        let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();

        if tx.send(crate::server_task::ServerMessage::ListCameras {
            respond_to: respond_tx,
        }).is_err() {
            // ServerTask dropped, fall back to direct access
            let cameras = state.get_cameras().await;
            return Json(ApiResponse::ok(cameras));
        }

        match respond_rx.await {
            Ok(cameras) => Json(ApiResponse::ok(cameras)),
            Err(_) => {
                // Response channel dropped, fall back to direct access
                let cameras = state.get_cameras().await;
                Json(ApiResponse::ok(cameras))
            }
        }
    } else {
        // Direct state access path (OLD architecture)
        let cameras = state.get_cameras().await;
        Json(ApiResponse::ok(cameras))
    }
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
    // Try message-passing if ServerTask is available
    let server_task_tx_opt = state.server_task_tx.read().await.clone();

    if let Some(tx) = server_task_tx_opt {
        // Message-passing path
        let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();

        if tx.send(crate::server_task::ServerMessage::GetCamera {
            camera_id: id,
            respond_to: respond_tx,
        }).is_ok() {
            if let Ok(Some(camera)) = respond_rx.await {
                return Ok(Json(ApiResponse::ok(camera)));
            }
        }
    }

    // Fall back to direct state access
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
    // Try message-passing if ServerTask is available
    let server_task_tx_opt = state.server_task_tx.read().await.clone();

    if let Some(tx) = server_task_tx_opt {
        // Message-passing path
        let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();

        if tx.send(crate::server_task::ServerMessage::DeleteCamera {
            camera_id: id,
            respond_to: respond_tx,
        }).is_ok() {
            if let Ok(Some(camera)) = respond_rx.await {
                return Ok(Json(ApiResponse::ok(camera)));
            }
        }
    }

    // Fall back to direct state access
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

// === Recording Handlers ===

/// POST /api/cameras/{id}/recording/start - Start recording for a camera
async fn start_recording(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<Uuid>,
    Json(request): Json<Option<RecordingRequest>>,
) -> Result<Json<ApiResponse<RecordingStatus>>, StatusCode> {
    let request = request.unwrap_or_default();
    
    // Get the camera
    let camera = state.get_camera(&id).await.ok_or(StatusCode::NOT_FOUND)?;
    
    // Get the RTSP URL for the requested stream
    let rtsp_url = match request.stream_type {
        StreamType::Main => camera.main_stream_uri.clone(),
        StreamType::Sub => camera.sub_stream_uri.clone(),
    };
    
    let Some(rtsp_url) = rtsp_url else {
        return Ok(Json(ApiResponse::err("No RTSP URL configured for this stream type")));
    };
    
    let stream_type_str = request.stream_type.to_string();

    // Start recording (RTSP client + recorder task)
    if let Err(e) = state.start_recording(id, &stream_type_str, &rtsp_url).await {
        // Fall back to just starting RTSP client if DB not initialized
        tracing::warn!("Full recording failed, falling back to stream-only: {}", e);
        if let Err(e) = state.start_rtsp_client(id, &stream_type_str, &rtsp_url).await {
            return Ok(Json(ApiResponse::err(format!("Failed to start streaming: {}", e))));
        }
    }
    
    // Get the session for status
    let session = state.stream_manager.get_or_create_session(
        id,
        &stream_type_str,
        &rtsp_url,
    );
    
    let stats = session.stats();
    let is_recording = state.is_recording(id, &stream_type_str).await;
    
    Ok(Json(ApiResponse::ok(RecordingStatus {
        camera_id: id,
        is_recording: is_recording || session.state() == SessionState::Active,
        stream_type: Some(request.stream_type),
        started_at: stats.connected_at.map(|_| chrono::Utc::now()),
        bytes_recorded: stats.bytes_received,
        frames_recorded: stats.frames_received,
        current_file: None,
    })))
}

/// POST /api/cameras/{id}/recording/stop - Stop recording for a camera
async fn stop_recording(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<Uuid>,
    Json(request): Json<Option<RecordingRequest>>,
) -> Result<Json<ApiResponse<RecordingStatus>>, StatusCode> {
    let request = request.unwrap_or_default();
    let stream_type_str = request.stream_type.to_string();
    
    // Stop recording (recorder task + RTSP client)
    state.stop_recording(id, &stream_type_str).await;
    
    // Get the session if it exists for final stats
    let session = state.stream_manager.get_session(id, &stream_type_str);
    
    if let Some(session) = session {
        let stats = session.stats();
        
        Ok(Json(ApiResponse::ok(RecordingStatus {
            camera_id: id,
            is_recording: false,
            stream_type: Some(request.stream_type),
            started_at: None,
            bytes_recorded: stats.bytes_received,
            frames_recorded: stats.frames_received,
            current_file: None,
        })))
    } else {
        Ok(Json(ApiResponse::ok(RecordingStatus {
            camera_id: id,
            is_recording: false,
            stream_type: Some(request.stream_type),
            started_at: None,
            bytes_recorded: 0,
            frames_recorded: 0,
            current_file: None,
        })))
    }
}

/// GET /api/cameras/{id}/recording/status - Get recording status for a camera
async fn get_recording_status(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<RecordingStatus>>, StatusCode> {
    // Check camera exists
    let _camera = state.get_camera(&id).await.ok_or(StatusCode::NOT_FOUND)?;
    
    // Check for main stream session
    let main_session = state.stream_manager.get_session(id, "main");
    
    if let Some(session) = main_session {
        let stats = session.stats();
        Ok(Json(ApiResponse::ok(RecordingStatus {
            camera_id: id,
            is_recording: session.state() == SessionState::Active,
            stream_type: Some(StreamType::Main),
            started_at: stats.connected_at.map(|_| chrono::Utc::now()),
            bytes_recorded: stats.bytes_received,
            frames_recorded: stats.frames_received,
            current_file: None,
        })))
    } else {
        Ok(Json(ApiResponse::ok(RecordingStatus {
            camera_id: id,
            is_recording: false,
            stream_type: None,
            started_at: None,
            bytes_recorded: 0,
            frames_recorded: 0,
            current_file: None,
        })))
    }
}

/// GET /api/cameras/{id}/stream/stats - Get stream statistics
async fn get_stream_stats(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<Vec<StreamStats>>>, StatusCode> {
    // Check camera exists
    let _camera = state.get_camera(&id).await.ok_or(StatusCode::NOT_FOUND)?;
    
    let mut stats = Vec::new();
    
    // Check main stream
    if let Some(session) = state.stream_manager.get_session(id, "main") {
        let session_stats = session.stats();
        let metadata = session.metadata();
        let state_val = session.state();
        
        stats.push(StreamStats {
            camera_id: id,
            stream_type: StreamType::Main,
            state: match state_val {
                SessionState::Disconnected => StreamState::Disconnected,
                SessionState::Connecting => StreamState::Connecting,
                SessionState::Active => StreamState::Active,
                SessionState::Reconnecting => StreamState::Reconnecting,
                SessionState::Stopped => StreamState::Stopped,
            },
            codec: Some(metadata.codec.to_string()),
            width: metadata.width,
            height: metadata.height,
            frames_received: session_stats.frames_received,
            keyframes_received: session_stats.keyframes_received,
            bytes_received: session_stats.bytes_received,
            connected_secs: session_stats.connected_at.map(|t| t.elapsed().as_secs_f64()),
            reconnect_count: session_stats.reconnect_count,
        });
    }
    
    // Check sub stream
    if let Some(session) = state.stream_manager.get_session(id, "sub") {
        let session_stats = session.stats();
        let metadata = session.metadata();
        let state_val = session.state();
        
        stats.push(StreamStats {
            camera_id: id,
            stream_type: StreamType::Sub,
            state: match state_val {
                SessionState::Disconnected => StreamState::Disconnected,
                SessionState::Connecting => StreamState::Connecting,
                SessionState::Active => StreamState::Active,
                SessionState::Reconnecting => StreamState::Reconnecting,
                SessionState::Stopped => StreamState::Stopped,
            },
            codec: Some(metadata.codec.to_string()),
            width: metadata.width,
            height: metadata.height,
            frames_received: session_stats.frames_received,
            keyframes_received: session_stats.keyframes_received,
            bytes_received: session_stats.bytes_received,
            connected_secs: session_stats.connected_at.map(|t| t.elapsed().as_secs_f64()),
            reconnect_count: session_stats.reconnect_count,
        });
    }
    
    Ok(Json(ApiResponse::ok(stats)))
}

/// GET /api/recordings - List recordings
async fn list_recordings(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<RecordingsQuery>,
) -> Json<ApiResponse<RecordingsResponse>> {
    // Get database
    let db_guard = state.hot_cold_db.read().await;
    let db = match db_guard.as_ref() {
        Some(db) => db.clone(),
        None => {
            return Json(ApiResponse::err("Database not initialized"));
        }
    };
    drop(db_guard);

    // Determine which streams to query
    let stream_ids = if let Some(camera_id) = query.camera_id {
        // Query specific camera
        let db_read = db.read().await;
        match db_read.get_camera_stream_ids(&camera_id.to_string()).await {
            Ok(streams) => {
                // Filter by stream type if specified
                if let Some(ref stream_type) = query.stream_type {
                    streams.into_iter()
                        .filter(|(stype, _)| stype == &stream_type.to_string())
                        .map(|(_, id)| (camera_id, id))
                        .collect::<Vec<_>>()
                } else {
                    streams.into_iter().map(|(_, id)| (camera_id, id)).collect::<Vec<_>>()
                }
            }
            Err(e) => {
                return Json(ApiResponse::err(format!("Failed to query streams: {}", e)));
            }
        }
    } else {
        // Query all cameras - not implemented yet
        return Json(ApiResponse::err("Querying all cameras not yet supported. Please specify camera_id."));
    };

    // Determine time range
    let start_time = query.start_time.unwrap_or_else(|| chrono::Utc::now() - chrono::Duration::hours(24));
    let end_time = query.end_time.unwrap_or_else(chrono::Utc::now);

    // Query recordings for each stream
    let mut all_recordings = Vec::new();
    for (camera_id, stream_id) in stream_ids {
        let db_read = db.read().await;
        match db_read.query_recordings(stream_id, start_time, end_time).await {
            Ok(unified_recordings) => {
                // Convert UnifiedRecording to Recording API type
                for ur in unified_recordings {
                    let recording = crate::api::Recording {
                        id: ur.recording.id.unwrap_or(0),
                        camera_id,
                        stream_type: crate::api::StreamType::Main, // TODO: Look up from stream table
                        start_time: crate::db::ticks_to_datetime(ur.recording.start_ticks),
                        end_time: Some(crate::db::ticks_to_datetime(
                            ur.recording.start_ticks + ur.recording.duration_ticks
                        )),
                        duration_secs: Some(ur.recording.duration_ticks as f64 / crate::db::TICKS_PER_SEC as f64),
                        size_bytes: ur.recording.sample_file_bytes as u64,
                        file_path: ur.file_path,
                        codec: None, // TODO: Get from stream table
                        resolution: None, // TODO: Get from stream table
                    };
                    all_recordings.push(recording);
                }
            }
            Err(e) => {
                tracing::warn!("Failed to query recordings for stream {}: {}", stream_id, e);
            }
        }
    }

    // Apply pagination
    let limit = query.limit.min(1000) as usize; // Cap at 1000
    let offset = query.offset as usize;
    let total = all_recordings.len() as u64;

    let recordings = all_recordings
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect();

    Json(ApiResponse::ok(RecordingsResponse {
        total,
        recordings,
    }))
}

// ============================================================================
// Clip Export Handlers
// ============================================================================

/// POST /api/clips/info - Get clip export info (size, duration, etc.)
async fn get_clip_info(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<ClipExportRequest>,
) -> Json<ApiResponse<ClipExportInfo>> {
    // Check camera exists
    if state.get_camera(&request.camera_id).await.is_none() {
        return Json(ApiResponse::err("Camera not found"));
    }

    // Get the database
    let db_guard = state.hot_cold_db.read().await;
    let db = match db_guard.as_ref() {
        Some(db) => db.clone(),
        None => {
            return Json(ApiResponse::err("Database not initialized"));
        }
    };
    drop(db_guard);

    // Look up stream_id from database
    let stream_id = {
        let db_read = db.read().await;
        match db_read.get_stream_id(
            &request.camera_id.to_string(),
            &request.stream_type.to_string()
        ).await {
            Ok(Some(id)) => id,
            Ok(None) => {
                return Json(ApiResponse::err(format!(
                    "Stream not found for camera {} and type {}",
                    request.camera_id, request.stream_type
                )));
            }
            Err(e) => {
                return Json(ApiResponse::err(format!("Failed to query stream: {}", e)));
            }
        }
    };

    match prepare_clip(&db, stream_id, &request).await {
        Ok(prepared) => Json(ApiResponse::ok(prepared.info)),
        Err(e) => Json(ApiResponse::err(format!("Failed to prepare clip: {}", e))),
    }
}

/// POST /api/clips/export - Export and download a video clip
async fn export_clip(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<ClipExportRequest>,
) -> Result<Response, StatusCode> {
    // Check camera exists
    let _camera = state
        .get_camera(&request.camera_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    // Get the database
    let db_guard = state.hot_cold_db.read().await;
    let db = match db_guard.as_ref() {
        Some(db) => db.clone(),
        None => {
            return Ok((
                StatusCode::SERVICE_UNAVAILABLE,
                "Database not initialized",
            )
                .into_response());
        }
    };
    drop(db_guard);

    // Look up stream_id from database
    let stream_id = {
        let db_read = db.read().await;
        match db_read.get_stream_id(
            &request.camera_id.to_string(),
            &request.stream_type.to_string()
        ).await {
            Ok(Some(id)) => id,
            Ok(None) => {
                return Ok((
                    StatusCode::NOT_FOUND,
                    format!("Stream not found for camera {} and type {}", request.camera_id, request.stream_type),
                )
                    .into_response());
            }
            Err(e) => {
                error!("Failed to query stream: {}", e);
                return Ok((StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to query stream: {}", e))
                    .into_response());
            }
        }
    };

    // Prepare the clip
    let prepared = match prepare_clip(&db, stream_id, &request).await {
        Ok(p) => p,
        Err(e) => {
            error!("Failed to prepare clip: {}", e);
            return Ok((StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to prepare clip: {}", e))
                .into_response());
        }
    };

    // Check limits
    if prepared.info.exceeds_limit {
        return Ok((
            StatusCode::BAD_REQUEST,
            prepared.info.error.unwrap_or_else(|| "Clip exceeds size limits".to_string()),
        )
            .into_response());
    }

    if prepared.recordings.is_empty() {
        return Ok((
            StatusCode::NOT_FOUND,
            prepared.info.error.unwrap_or_else(|| "No recordings found".to_string()),
        )
            .into_response());
    }

    // Export the clip
    let data = match export_clip_data(&prepared).await {
        Ok(d) => d,
        Err(e) => {
            error!("Failed to export clip: {}", e);
            return Ok((StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to export clip: {}", e))
                .into_response());
        }
    };

    info!(
        camera_id = %request.camera_id,
        size_bytes = data.len(),
        duration_secs = prepared.info.duration_secs,
        "Clip export successful"
    );

    // Return as downloadable MP4
    let filename = prepared.info.suggested_filename;
    
    Ok((
        [
            (header::CONTENT_TYPE, "video/mp4"),
            (
                header::CONTENT_DISPOSITION,
                &format!("attachment; filename=\"{}\"", filename),
            ),
            (header::CONTENT_LENGTH, &data.len().to_string()),
        ],
        Body::from(data),
    )
        .into_response())
}

// ============================================================================
// Playback Handlers (Phase 3)
// ============================================================================

/// Query parameters for timeline requests
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineQuery {
    /// Stream type (main/sub)
    #[serde(default)]
    pub stream_type: Option<StreamType>,
    /// Start time for range queries
    pub start_time: Option<chrono::DateTime<chrono::Utc>>,
    /// End time for range queries
    pub end_time: Option<chrono::DateTime<chrono::Utc>>,
}

/// POST /api/cameras/{id}/playback/seek - Seek to nearest keyframe
async fn seek_to_keyframe(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<Uuid>,
    Json(request): Json<SeekRequest>,
) -> Result<Json<ApiResponse<PlaybackFrame>>, StatusCode> {
    // Check camera exists
    let _camera = state.get_camera(&id).await.ok_or(StatusCode::NOT_FOUND)?;
    
    // Get frame store if available
    let Some(ref frame_store) = state.frame_store else {
        return Ok(Json(ApiResponse::err("Frame store not initialized")));
    };
    
    // Convert seek direction
    let direction = match request.direction.unwrap_or(SeekDirection::Backward) {
        SeekDirection::Backward => FrameSeekDirection::Backward,
        SeekDirection::Forward => FrameSeekDirection::Forward,
        SeekDirection::Nearest => FrameSeekDirection::Nearest,
    };
    
    // Find the keyframe
    match frame_store
        .find_keyframe(id, "main", request.timestamp, direction)
        .await
    {
        Ok(Some(frame)) => Ok(Json(ApiResponse::ok(PlaybackFrame {
            id: frame.id,
            timestamp: frame.timestamp,
            sequence_num: frame.sequence_num,
            is_keyframe: frame.is_keyframe,
            frame_size: frame.frame_size,
            codec: frame.codec,
        }))),
        Ok(None) => Ok(Json(ApiResponse::err("No keyframe found near the specified time"))),
        Err(e) => Ok(Json(ApiResponse::err(format!("Failed to seek: {}", e)))),
    }
}

/// POST /api/cameras/{id}/playback/frames - Get frames for a time range
async fn get_frames(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<Uuid>,
    Json(request): Json<FrameRangeRequest>,
) -> Result<Json<ApiResponse<FrameRangeResponse>>, StatusCode> {
    // Check camera exists
    let _camera = state.get_camera(&id).await.ok_or(StatusCode::NOT_FOUND)?;
    
    // Get frame store if available
    let Some(ref frame_store) = state.frame_store else {
        return Ok(Json(ApiResponse::err("Frame store not initialized")));
    };
    
    // Query frames
    match frame_store
        .query_frames(
            id,
            "main",
            request.start_time,
            request.end_time,
            request.start_from_keyframe,
        )
        .await
    {
        Ok(frames) => {
            let actual_start = frames.first().map(|f| f.timestamp).unwrap_or(request.start_time);
            let playback_frames: Vec<PlaybackFrame> = frames
                .into_iter()
                .map(|f| PlaybackFrame {
                    id: f.id,
                    timestamp: f.timestamp,
                    sequence_num: f.sequence_num,
                    is_keyframe: f.is_keyframe,
                    frame_size: f.frame_size,
                    codec: f.codec,
                })
                .collect();
            
            Ok(Json(ApiResponse::ok(FrameRangeResponse {
                camera_id: id,
                stream_type: StreamType::Main,
                actual_start_time: actual_start,
                end_time: request.end_time,
                frame_count: playback_frames.len(),
                frames: playback_frames,
            })))
        }
        Err(e) => Ok(Json(ApiResponse::err(format!("Failed to query frames: {}", e)))),
    }
}

/// GET /api/cameras/{id}/timeline - Get timeline summary
async fn get_timeline(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<Uuid>,
    Query(query): Query<TimelineQuery>,
) -> Result<Json<ApiResponse<TimelineSummary>>, StatusCode> {
    // Check camera exists
    let _camera = state.get_camera(&id).await.ok_or(StatusCode::NOT_FOUND)?;
    
    // Get frame store if available
    let Some(ref frame_store) = state.frame_store else {
        return Ok(Json(ApiResponse::err("Frame store not initialized")));
    };
    
    let stream_type = query.stream_type.unwrap_or(StreamType::Main).to_string();
    
    // Query segments
    match frame_store
        .query_segments(id, &stream_type, query.start_time, query.end_time)
        .await
    {
        Ok(segments) => {
            let earliest = segments.first().map(|s| s.start_time);
            let latest = segments.last().and_then(|s| s.end_time.or(Some(s.start_time)));
            
            let total_duration: f64 = segments
                .iter()
                .map(|s| {
                    s.end_time
                        .map(|e| (e - s.start_time).num_milliseconds() as f64 / 1000.0)
                        .unwrap_or(0.0)
                })
                .sum();
            
            let timeline_segments: Vec<TimelineSegment> = segments
                .into_iter()
                .map(|s| {
                    let duration = s
                        .end_time
                        .map(|e| (e - s.start_time).num_milliseconds() as f64 / 1000.0)
                        .unwrap_or(0.0);
                    TimelineSegment {
                        start_time: s.start_time,
                        end_time: s.end_time,
                        duration_secs: duration,
                        keyframe_count: s.keyframe_count,
                        is_complete: s.is_complete,
                    }
                })
                .collect();
            
            Ok(Json(ApiResponse::ok(TimelineSummary {
                camera_id: id,
                stream_type: query.stream_type.unwrap_or(StreamType::Main),
                earliest_time: earliest,
                latest_time: latest,
                total_duration_secs: total_duration,
                segments: timeline_segments,
            })))
        }
        Err(e) => Ok(Json(ApiResponse::err(format!("Failed to query timeline: {}", e)))),
    }
}

/// GET /api/cameras/{id}/timeline/keyframes - Get keyframe timestamps for seeking
async fn get_keyframe_timeline(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<Uuid>,
    Query(query): Query<TimelineQuery>,
) -> Result<Json<ApiResponse<KeyframeTimeline>>, StatusCode> {
    // Check camera exists
    let _camera = state.get_camera(&id).await.ok_or(StatusCode::NOT_FOUND)?;
    
    // Get frame store if available
    let Some(ref frame_store) = state.frame_store else {
        return Ok(Json(ApiResponse::err("Frame store not initialized")));
    };
    
    let stream_type = query.stream_type.unwrap_or(StreamType::Main).to_string();
    let start_time = query.start_time.unwrap_or_else(|| {
        chrono::Utc::now() - chrono::Duration::hours(1)
    });
    let end_time = query.end_time.unwrap_or_else(chrono::Utc::now);
    
    match frame_store.get_timeline(id, &stream_type, start_time, end_time).await {
        Ok(keyframes) => Ok(Json(ApiResponse::ok(KeyframeTimeline {
            camera_id: id,
            stream_type: query.stream_type.unwrap_or(StreamType::Main),
            start_time,
            end_time,
            keyframes,
        }))),
        Err(e) => Ok(Json(ApiResponse::err(format!("Failed to get keyframes: {}", e)))),
    }
}

// =============================================================================
// WebRTC Handlers (Phase 6)
// =============================================================================

/// POST /api/webrtc/offer - Process SDP offer and return answer
async fn webrtc_offer(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<WebRtcOfferRequest>,
) -> Result<Json<ApiResponse<WebRtcAnswerResponse>>, StatusCode> {
    // Check camera exists
    let _camera = state
        .get_camera(&request.camera_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    // Get WebRTC manager
    let Some(ref webrtc_manager) = state.webrtc_manager else {
        return Ok(Json(ApiResponse::err("WebRTC not available")));
    };

    // Create a new session
    let stream_type = request.stream_type.to_string();
    let session_id = match webrtc_manager.create_session(request.camera_id, &stream_type) {
        Ok(id) => id,
        Err(e) => {
            error!(error = %e, camera_id = %request.camera_id, "Failed to create WebRTC session");
            return Ok(Json(ApiResponse::ok(WebRtcAnswerResponse {
                success: false,
                error: Some(e.to_string()),
                session_id: None,
                sdp: None,
            })));
        }
    };

    // Get the session and process the offer
    let session = webrtc_manager
        .get_session(session_id)
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    // Add a local candidate (server's IP)
    // In production, you'd get this from configuration or network discovery
    let local_addr = state.config.server.bind_address;
    if let Err(e) = session.add_local_candidate(local_addr) {
        warn!(error = %e, "Failed to add local candidate");
    }

    // Process the SDP offer
    match session.process_offer(&request.sdp) {
        Ok(answer) => {
            info!(session_id = %session_id, camera_id = %request.camera_id, "WebRTC session created");
            
            // Get both main and sub streams for layer switching
            let main_stream = state.stream_manager.get_session(request.camera_id, "main");
            let sub_stream = state.stream_manager.get_session(request.camera_id, "sub");
            
            // Build forwarding context with available streams
            let mut context = crate::webrtc::ForwardingContext::new();
            if let Some(main) = main_stream {
                context = context.with_main(main);
            }
            if let Some(sub) = sub_stream {
                context = context.with_sub(sub);
            }
            
            // Check if we have at least one stream
            if context.main_stream.is_some() || context.sub_stream.is_some() {
                info!(
                    session_id = %session_id,
                    camera_id = %request.camera_id,
                    has_main = context.main_stream.is_some(),
                    has_sub = context.sub_stream.is_some(),
                    "Starting enhanced forwarding with command handling"
                );
                let (_handle, _shutdown) = crate::webrtc::start_forwarding_with_commands(
                    session.clone(),
                    context,
                );
                // Note: In production, store handle/shutdown_tx to stop forwarding when session closes
            } else {
                warn!(
                    camera_id = %request.camera_id,
                    "No active streams for camera, WebRTC session created but not forwarding"
                );
            }
            
            Ok(Json(ApiResponse::ok(WebRtcAnswerResponse {
                success: true,
                error: None,
                session_id: Some(session_id),
                sdp: Some(answer),
            })))
        }
        Err(e) => {
            error!(error = %e, session_id = %session_id, "Failed to process SDP offer");
            // Clean up the session on failure
            webrtc_manager.remove_session(session_id);
            Ok(Json(ApiResponse::ok(WebRtcAnswerResponse {
                success: false,
                error: Some(e.to_string()),
                session_id: None,
                sdp: None,
            })))
        }
    }
}

/// POST /api/webrtc/ice-candidate - Add ICE candidate
async fn webrtc_ice_candidate(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<IceCandidateRequest>,
) -> Result<Json<ApiResponse<IceCandidateResponse>>, StatusCode> {
    let Some(ref webrtc_manager) = state.webrtc_manager else {
        return Ok(Json(ApiResponse::err("WebRTC not available")));
    };

    let Some(session) = webrtc_manager.get_session(request.session_id) else {
        return Ok(Json(ApiResponse::ok(IceCandidateResponse {
            success: false,
            error: Some("Session not found".to_string()),
        })));
    };

    match session.add_ice_candidate(&request.candidate) {
        Ok(()) => Ok(Json(ApiResponse::ok(IceCandidateResponse {
            success: true,
            error: None,
        }))),
        Err(e) => {
            warn!(error = %e, session_id = %request.session_id, "Failed to add ICE candidate");
            Ok(Json(ApiResponse::ok(IceCandidateResponse {
                success: false,
                error: Some(e.to_string()),
            })))
        }
    }
}

/// POST /api/webrtc/multi-replay - Create multi-camera replay WebRTC session
async fn webrtc_multi_replay(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<crate::webrtc::MultiReplayWebRtcRequest>,
) -> Result<Json<ApiResponse<crate::webrtc::MultiReplayWebRtcResponse>>, StatusCode> {
    use crate::webrtc::MultiReplayWebRtcResponse;
    
    // Validate camera count
    if request.camera_ids.is_empty() {
        return Ok(Json(ApiResponse::ok(MultiReplayWebRtcResponse {
            success: false,
            error: Some("At least one camera is required".to_string()),
            session_id: None,
            sdp_answer: None,
        })));
    }
    if request.camera_ids.len() > 9 {
        return Ok(Json(ApiResponse::ok(MultiReplayWebRtcResponse {
            success: false,
            error: Some("Maximum 9 cameras allowed".to_string()),
            session_id: None,
            sdp_answer: None,
        })));
    }
    
    // Validate cameras exist
    let cameras_state = state.cameras.read().await;
    for camera_id in &request.camera_ids {
        if !cameras_state.contains_key(camera_id) {
            return Ok(Json(ApiResponse::ok(MultiReplayWebRtcResponse {
                success: false,
                error: Some(format!("Camera {} not found", camera_id)),
                session_id: None,
                sdp_answer: None,
            })));
        }
    }
    drop(cameras_state);
    
    // Get WebRTC manager
    let Some(ref webrtc_manager) = state.webrtc_manager else {
        return Ok(Json(ApiResponse::ok(MultiReplayWebRtcResponse {
            success: false,
            error: Some("WebRTC not available".to_string()),
            session_id: None,
            sdp_answer: None,
        })));
    };
    
    // Create multi-replay session
    let session = match webrtc_manager.create_multi_replay_session(
        request.camera_ids.clone(),
        request.start_time,
        request.end_time,
    ) {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "Failed to create multi-replay session");
            return Ok(Json(ApiResponse::ok(MultiReplayWebRtcResponse {
                success: false,
                error: Some(e.to_string()),
                session_id: None,
                sdp_answer: None,
            })));
        }
    };
    
    let session_id = session.id;
    
    // Add local candidate
    let local_addr = state.config.server.bind_address;
    if let Err(e) = session.add_local_candidate(local_addr) {
        warn!(error = %e, "Failed to add local candidate for multi-replay session");
    }
    
    // Process SDP offer
    match session.process_offer(&request.sdp_offer) {
        Ok(answer) => {
            info!(
                session_id = %session_id,
                camera_count = request.camera_ids.len(),
                "Multi-replay WebRTC session created"
            );
            
            // Start the replay forwarding loop
            let coordinator = session.coordinator();
            if let Some(command_rx) = session.take_command_rx() {
                // Spawn task to handle commands and forward frames
                let session_clone = session.clone();
                let frame_store = state.frame_store.clone();
                tokio::spawn(async move {
                    run_multi_replay_loop(session_clone, coordinator, command_rx, frame_store).await;
                });
            }
            
            Ok(Json(ApiResponse::ok(MultiReplayWebRtcResponse {
                success: true,
                error: None,
                session_id: Some(session_id),
                sdp_answer: Some(answer),
            })))
        }
        Err(e) => {
            error!(error = %e, session_id = %session_id, "Failed to process multi-replay SDP offer");
            webrtc_manager.remove_multi_replay_session(session_id);
            Ok(Json(ApiResponse::ok(MultiReplayWebRtcResponse {
                success: false,
                error: Some(e.to_string()),
                session_id: None,
                sdp_answer: None,
            })))
        }
    }
}

/// Run the multi-replay forwarding loop
async fn run_multi_replay_loop(
    session: std::sync::Arc<crate::webrtc::MultiReplaySession>,
    coordinator: std::sync::Arc<crate::replay_coordinator::ReplayCoordinator>,
    mut command_rx: tokio::sync::mpsc::UnboundedReceiver<crate::api::MultiReplayCommand>,
    frame_store: Option<std::sync::Arc<crate::frame_store::FrameStore>>,
) {
    use crate::replay_coordinator::PlaybackState;
    use tokio::time::{interval, Duration};
    
    let mut playback_interval = interval(Duration::from_millis(33)); // ~30fps
    let mut position_update_interval = interval(Duration::from_millis(500)); // Position updates 2x/sec
    let mut stats_interval = interval(Duration::from_secs(30)); // Log stats every 30s

    // Performance tracking
    let mut frames_sent = 0u64;
    let mut frames_missed = 0u64;
    let mut frames_errored = 0u64;
    let start_time = std::time::Instant::now();

    info!(session_id = %session.id, "Starting multi-replay forwarding loop");
    
    loop {
        tokio::select! {
            // Handle incoming commands
            Some(cmd) = command_rx.recv() => {
                coordinator.handle_command(cmd, frame_store.as_deref()).await;
            }
            
            // Advance playhead and send frames
            _ = playback_interval.tick() => {
                // Advance playhead if playing
                if let Some(new_time) = coordinator.advance_playhead(33) {
                    // Get frames for all cameras at current playhead
                    let requests = coordinator.get_frame_requests();
                    
                    if let Some(ref store) = frame_store {
                        for (track_idx, camera_id, layer) in requests {
                            // Determine which stream to use based on layer
                            let stream_type = match layer {
                                crate::replay_coordinator::SimulcastLayer::High => "main",
                                crate::replay_coordinator::SimulcastLayer::Low => "sub",
                            };
                            
                            // Try to read frame from store
                            // NOTE: In a real implementation, this would be a batch query
                            // For now, query per-camera (can be optimized later)
                            match store.read_frame_at_time(
                                camera_id,
                                stream_type,
                                new_time,
                            ).await {
                                Ok(Some(frame_data)) => {
                                    // Create VideoFrame and send to track
                                    let frame = crate::webrtc::VideoFrame {
                                        data: bytes::Bytes::from(frame_data.data),
                                        rtp_timestamp: (frame_data.pts as u32) % (1 << 31),
                                        wallclock: std::time::Instant::now(),
                                        is_keyframe: frame_data.is_keyframe,
                                        codec: crate::webrtc::VideoCodec::H264,
                                    };

                                    if let Err(e) = session.send_frame_to_track(track_idx, &frame) {
                                        tracing::trace!(
                                            session_id = %session.id,
                                            track = track_idx,
                                            error = %e,
                                            "Failed to send frame to track"
                                        );
                                        frames_errored += 1;
                                    } else {
                                        tracing::trace!(
                                            session_id = %session.id,
                                            track = track_idx,
                                            camera_id = %camera_id,
                                            timestamp = %new_time,
                                            is_keyframe = frame_data.is_keyframe,
                                            "Sent frame to track"
                                        );
                                        frames_sent += 1;
                                    }

                                    coordinator.update_track_frame_time(track_idx, new_time);
                                    // Mark track as having recording (exits gap if was in one)
                                    coordinator.mark_track_gap(track_idx, false);
                                }
                                Ok(None) => {
                                    // No frame at this time - mark track as in gap
                                    tracing::debug!(
                                        session_id = %session.id,
                                        track = track_idx,
                                        camera_id = %camera_id,
                                        timestamp = %new_time,
                                        "No frame available - camera in gap"
                                    );
                                    frames_missed += 1;
                                    coordinator.mark_track_gap(track_idx, true);
                                }
                                Err(e) => {
                                    // Error reading frame - log and mark as gap
                                    tracing::warn!(
                                        session_id = %session.id,
                                        track = track_idx,
                                        camera_id = %camera_id,
                                        error = %e,
                                        "Error reading frame from store"
                                    );
                                    frames_errored += 1;
                                    coordinator.mark_track_gap(track_idx, true);
                                }
                            }
                        }
                    }
                }
            }
            
            // Send periodic position updates
            _ = position_update_interval.tick() => {
                if matches!(coordinator.playback_state(), PlaybackState::Playing { .. }) {
                    let playhead = coordinator.playhead();
                    let state = coordinator.playback_state();
                    let (is_playing, speed) = match state {
                        PlaybackState::Playing { speed } => (true, speed),
                        _ => (false, 1.0),
                    };

                    if let Err(e) = session.send_data_channel_message(
                        &crate::api::MultiReplayMessage::Position {
                            timestamp: playhead,
                            is_playing,
                            speed,
                        }
                    ) {
                        tracing::trace!(error = %e, "Failed to send position update");
                    }
                }
            }

            // Log statistics periodically
            _ = stats_interval.tick() => {
                let elapsed = start_time.elapsed().as_secs();
                let total_attempts = frames_sent + frames_missed + frames_errored;
                let success_rate = if total_attempts > 0 {
                    (frames_sent as f64 / total_attempts as f64) * 100.0
                } else {
                    0.0
                };

                info!(
                    session_id = %session.id,
                    elapsed_secs = elapsed,
                    frames_sent = frames_sent,
                    frames_missed = frames_missed,
                    frames_errored = frames_errored,
                    success_rate = format!("{:.1}%", success_rate),
                    "Multi-replay session statistics"
                );
            }
        }
        
        // Check if session is still active
        if session.state() == crate::webrtc::SessionState::Closed {
            info!(session_id = %session.id, "Multi-replay session closed, exiting loop");
            break;
        }
    }

    // Log final statistics
    let elapsed = start_time.elapsed();
    let total_attempts = frames_sent + frames_missed + frames_errored;
    let success_rate = if total_attempts > 0 {
        (frames_sent as f64 / total_attempts as f64) * 100.0
    } else {
        0.0
    };

    info!(
        session_id = %session.id,
        duration_secs = elapsed.as_secs(),
        frames_sent = frames_sent,
        frames_missed = frames_missed,
        frames_errored = frames_errored,
        success_rate = format!("{:.1}%", success_rate),
        avg_fps = if elapsed.as_secs() > 0 { frames_sent / elapsed.as_secs() } else { 0 },
        "Multi-replay session ended - final statistics"
    );
}

/// GET /api/webrtc/sessions - List all WebRTC sessions
async fn list_webrtc_sessions(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<ApiResponse<WebRtcSessionsResponse>>, StatusCode> {
    let Some(ref webrtc_manager) = state.webrtc_manager else {
        return Ok(Json(ApiResponse::err("WebRTC not available")));
    };

    let sessions: Vec<WebRtcSessionInfo> = webrtc_manager
        .list_sessions()
        .into_iter()
        .map(|(session_id, camera_id, stream_type, session_state)| WebRtcSessionInfo {
            session_id,
            camera_id,
            stream_type,
            state: format!("{:?}", session_state),
            created_at: chrono::Utc::now(), // Note: Would need to store creation time properly
        })
        .collect();

    let count = sessions.len();

    Ok(Json(ApiResponse::ok(WebRtcSessionsResponse {
        sessions,
        total_count: count,
    })))
}

/// DELETE /api/webrtc/sessions/{id} - Close a WebRTC session
async fn close_webrtc_session(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<()>>, StatusCode> {
    let Some(ref webrtc_manager) = state.webrtc_manager else {
        return Ok(Json(ApiResponse::err("WebRTC not available")));
    };

    webrtc_manager.remove_session(id);
    info!(session_id = %id, "WebRTC session closed");

    Ok(Json(ApiResponse::ok(())))
}

// =============================================================================
// UI Pages (Phase 6+ Enhancements)
// =============================================================================

/// GET /viewer/{camera_id} - Serve WebRTC live viewer page
async fn webrtc_viewer_page(
    State(state): State<Arc<ServerState>>,
    Path(camera_id): Path<Uuid>,
) -> Result<Html<String>, StatusCode> {
    // Verify camera exists
    let camera = state
        .get_camera(&camera_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    let page = crate::ui::generate_live_viewer(&camera_id, &camera.name);
    Ok(Html(page))
}

/// GET /replay/{camera_id} - Serve replay/review page
async fn replay_page(
    State(state): State<Arc<ServerState>>,
    Path(camera_id): Path<Uuid>,
) -> Result<Html<String>, StatusCode> {
    let camera = state
        .get_camera(&camera_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    let page = crate::ui::generate_replay_page(&camera_id, &camera.name);
    Ok(Html(page))
}

// ============================================================================
// Multi-Camera Replay
// ============================================================================

/// Query parameters for multi-replay page
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiReplayQuery {
    /// Comma-separated camera IDs
    pub cameras: Option<String>,
    /// Start time (ISO 8601)
    pub start: Option<String>,
    /// End time (ISO 8601)
    pub end: Option<String>,
}

/// GET /multi-replay - Serve multi-camera replay page
async fn multi_replay_page(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<MultiReplayQuery>,
) -> Html<String> {
    // Parse camera IDs from query or use all online cameras
    let camera_ids: Vec<Uuid> = if let Some(cameras_str) = query.cameras {
        cameras_str
            .split(',')
            .filter_map(|s| Uuid::parse_str(s.trim()).ok())
            .take(9) // Max 9 cameras
            .collect()
    } else {
        // Default to all online cameras (up to 9)
        let cameras_state = state.cameras.read().await;
        cameras_state
            .iter()
            .filter(|(_, cam)| cam.status == CameraStatus::Online)
            .take(9)
            .map(|(id, _)| *id)
            .collect()
    };
    
    // Parse time range
    let end_time = query.end
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);
    
    let start_time = query.start
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|| end_time - Duration::hours(1));
    
    // Build camera slots
    let cameras_state = state.cameras.read().await;
    let camera_slots: Vec<ReplayCameraSlot> = camera_ids
        .iter()
        .filter_map(|id| {
            cameras_state.get(id).map(|cam| ReplayCameraSlot {
                id: *id,
                name: cam.name.clone(),
                has_recordings: true, // TODO: Check actual recordings
            })
        })
        .collect();
    drop(cameras_state);
    
    let page = crate::ui::generate_multi_replay_page(
        &camera_slots,
        &start_time.to_rfc3339(),
        &end_time.to_rfc3339(),
    );
    
    Html(page)
}

/// POST /api/multi-replay - Create multi-camera replay session
async fn create_multi_replay(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<MultiReplayRequest>,
) -> Result<Json<ApiResponse<MultiReplayResponse>>, StatusCode> {
    // Validate camera count
    if request.camera_ids.is_empty() {
        return Ok(Json(ApiResponse::err("At least one camera is required")));
    }
    if request.camera_ids.len() > 9 {
        return Ok(Json(ApiResponse::err("Maximum 9 cameras allowed")));
    }
    
    // Validate cameras exist
    let cameras_state = state.cameras.read().await;
    let mut camera_infos = Vec::new();
    
    for (slot, camera_id) in request.camera_ids.iter().enumerate() {
        let camera = match cameras_state.get(camera_id) {
            Some(cam) => cam,
            None => {
                return Ok(Json(ApiResponse::err(format!(
                    "Camera {} not found",
                    camera_id
                ))));
            }
        };
        
        // TODO: Query actual recordings for this camera in the time range
        // For now, assume recordings exist
        let end = request.end_time.unwrap_or_else(Utc::now);
        let duration_secs = (end - request.start_time).num_seconds() as f64;
        let segments = vec![TimelineSegmentInfo {
            start_time: request.start_time,
            end_time: end,
            duration_secs,
            size_bytes: 0, // TODO: Calculate from actual recordings
            is_cold: false,
        }];
        
        camera_infos.push(ReplayCameraInfo {
            camera_id: *camera_id,
            name: camera.name.clone(),
            slot: slot as u8,
            has_recordings: !segments.is_empty(),
            segments,
        });
    }
    drop(cameras_state);
    
    let end_time = request.end_time.unwrap_or_else(Utc::now);
    
    // Build combined timeline
    let timeline = MultiCameraTimeline {
        start_time: request.start_time,
        end_time,
        any_recording: vec![(request.start_time, end_time)], // TODO: Calculate actual coverage
        all_recording: vec![(request.start_time, end_time)],
        gaps: vec![],
    };
    
    let session_id = Uuid::new_v4();
    
    let response = MultiReplayResponse {
        session_id,
        cameras: camera_infos,
        start_time: request.start_time,
        end_time,
        timeline,
    };
    
    info!(
        session_id = %session_id,
        camera_count = request.camera_ids.len(),
        "Created multi-camera replay session"
    );
    
    Ok(Json(ApiResponse::ok(response)))
}

/// GET / - Serve dashboard page
async fn dashboard_page(
    State(state): State<Arc<ServerState>>,
) -> Html<String> {
    let cameras_state = state.cameras.read().await;
    let cameras: Vec<(Uuid, String, bool)> = cameras_state
        .iter()
        .map(|(id, cam)| (*id, cam.name.clone(), cam.status == CameraStatus::Online))
        .collect();
    drop(cameras_state);
    
    Html(crate::ui::generate_dashboard(&cameras))
}

// ============================================================================
// Storage/Retention API Handlers (Phase 4)
// ============================================================================

use crate::api::{
    StorageStatsResponse, StorageTierStats,
    RetentionConfigResponse, MigrationStatusResponse,
    TimelineQuery as StorageTimelineQuery, TimelineResponse,
};

/// Calculate the total size of a directory recursively
fn calculate_directory_size(path: &std::path::Path) -> std::io::Result<u64> {
    let mut total = 0;

    if !path.exists() {
        return Ok(0);
    }

    if path.is_file() {
        return Ok(path.metadata()?.len());
    }

    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;

        if metadata.is_file() {
            total += metadata.len();
        } else if metadata.is_dir() {
            total += calculate_directory_size(&entry.path())?;
        }
    }

    Ok(total)
}

/// Get storage statistics for hot and cold storage
async fn get_storage_stats(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<ApiResponse<StorageStatsResponse>>, StatusCode> {
    // Get retention config for quota info
    let retention = &state.config.storage.retention;
    
    // In a real implementation, we'd query the HotColdDb
    // For now, return placeholder stats
    let response = StorageStatsResponse {
        hot: StorageTierStats {
            bytes_used: 0,
            quota_bytes: retention.hot_quota_bytes,
            usage_percent: 0.0,
            recording_count: 0,
            oldest_recording: None,
            newest_recording: None,
        },
        cold: StorageTierStats {
            bytes_used: 0,
            quota_bytes: retention.cold_quota_bytes,
            usage_percent: 0.0,
            recording_count: 0,
            oldest_recording: None,
            newest_recording: None,
        },
        streams: vec![],
    };
    
    Ok(Json(ApiResponse::ok(response)))
}

/// Get retention configuration
async fn get_retention_config(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<ApiResponse<RetentionConfigResponse>>, StatusCode> {
    let retention = &state.config.storage.retention;
    
    let response = RetentionConfigResponse {
        hot_quota_bytes: retention.hot_quota_bytes,
        cold_quota_bytes: retention.cold_quota_bytes,
        hot_max_age_secs: retention.hot_max_age_secs,
        cold_max_age_secs: retention.cold_max_age_secs,
        migration_interval_secs: retention.migration_interval_secs,
        migration_threshold: retention.migration_threshold,
    };
    
    Ok(Json(ApiResponse::ok(response)))
}

/// Get migration job status
async fn get_migration_status(
    State(_state): State<Arc<ServerState>>,
) -> Result<Json<ApiResponse<MigrationStatusResponse>>, StatusCode> {
    // In a real implementation, we'd query the migration job status
    // For now, return placeholder
    let response = MigrationStatusResponse {
        is_running: false,
        last_run: None,
        last_migrated_count: 0,
        last_migrated_bytes: 0,
        last_deleted_count: 0,
        next_run: None,
    };
    
    Ok(Json(ApiResponse::ok(response)))
}

/// Manually trigger a migration job
async fn trigger_migration(
    State(_state): State<Arc<ServerState>>,
) -> Result<Json<ApiResponse<()>>, StatusCode> {
    // In a real implementation, we'd trigger the migration job
    info!("Manual migration triggered");

    Ok(Json(ApiResponse::ok(())))
}

/// Get storage usage statistics
async fn get_storage_usage(
    State(state): State<Arc<ServerState>>,
) -> Json<ApiResponse<crate::api::StorageUsageResponse>> {
    // Get hot storage usage
    let hot_path = &state.config.storage.hot_storage_path;
    let hot_used = calculate_directory_size(hot_path).unwrap_or(0);

    // Get cold storage usage
    let cold_path = &state.config.storage.cold_storage_path;
    let cold_used = calculate_directory_size(cold_path).unwrap_or(0);

    let response = crate::api::StorageUsageResponse {
        hot_path: hot_path.to_string_lossy().to_string(),
        hot_used_bytes: hot_used,
        hot_quota_bytes: state.config.storage.retention.hot_quota_bytes,
        hot_usage_percent: (hot_used as f64 / state.config.storage.retention.hot_quota_bytes as f64) * 100.0,
        cold_path: cold_path.to_string_lossy().to_string(),
        cold_used_bytes: cold_used,
        cold_quota_bytes: state.config.storage.retention.cold_quota_bytes,
        cold_usage_percent: (cold_used as f64 / state.config.storage.retention.cold_quota_bytes as f64) * 100.0,
    };

    Json(ApiResponse::ok(response))
}

/// Get current configuration
async fn get_config(
    State(state): State<Arc<ServerState>>,
) -> Json<ApiResponse<Config>> {
    Json(ApiResponse::ok(state.config.clone()))
}

/// Update configuration
async fn update_config(
    State(state): State<Arc<ServerState>>,
    Json(new_config): Json<Config>,
) -> Result<Json<ApiResponse<Config>>, StatusCode> {
    // Save to config file
    // Note: In production, you'd need to track the config file path
    let config_path = crate::paths::default_config_path();

    if let Err(e) = new_config.save(&config_path) {
        error!("Failed to save config: {}", e);
        return Ok(Json(ApiResponse::err(format!("Failed to save configuration: {}", e))));
    }

    info!("Configuration updated and saved");

    // Note: Changes won't take effect until server restart
    // In a production system, you'd want to apply changes dynamically where possible

    Ok(Json(ApiResponse::ok(new_config)))
}

/// Query timeline across hot and cold storage
async fn query_timeline(
    State(_state): State<Arc<ServerState>>,
    Json(query): Json<StorageTimelineQuery>,
) -> Result<Json<ApiResponse<TimelineResponse>>, StatusCode> {
    // In a real implementation, we'd query the HotColdDb
    // For now, return empty timeline
    let duration_secs = (query.end_time - query.start_time).num_seconds() as f64;
    let response = TimelineResponse {
        camera_id: query.camera_id,
        start_time: query.start_time,
        end_time: query.end_time,
        segments: vec![],
        total_duration_secs: 0.0,
        total_gap_secs: duration_secs,
    };
    
    Ok(Json(ApiResponse::ok(response)))
}

// ============================================================================
// Health/Events API Handlers (Phase 5)
// ============================================================================

use crate::api::{
    AllHealthResponse, CameraHealthResponse, HealthHistoryResponse,
    HealthState as ApiHealthState, EventsQuery, EventsResponse,
    CombinedTimelineResponse,
};

/// GET /api/health - Get health status for all cameras
async fn get_all_health(
    State(state): State<Arc<ServerState>>,
) -> Result<Json<ApiResponse<AllHealthResponse>>, StatusCode> {
    let cameras = state.get_cameras().await;
    
    let health_info: Vec<CameraHealthResponse> = cameras
        .iter()
        .map(|c| CameraHealthResponse {
            camera_id: c.id,
            state: match c.status {
                CameraStatus::Online => ApiHealthState::Online,
                CameraStatus::Offline => ApiHealthState::Offline,
                _ => ApiHealthState::Unknown,
            },
            last_success: c.last_seen,
            last_failure: None,
            consecutive_successes: 0,
            consecutive_failures: 0,
            avg_latency_ms: None,
        })
        .collect();

    let online_count = health_info.iter().filter(|h| h.state == ApiHealthState::Online).count();
    let offline_count = health_info.iter().filter(|h| h.state == ApiHealthState::Offline).count();
    let degraded_count = health_info.iter().filter(|h| h.state == ApiHealthState::Degraded).count();

    Ok(Json(ApiResponse::ok(AllHealthResponse {
        cameras: health_info,
        online_count,
        offline_count,
        degraded_count,
    })))
}

/// GET /api/health/{id} - Get health status for a specific camera
async fn get_camera_health(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<CameraHealthResponse>>, StatusCode> {
    let camera = state.get_camera(&id).await.ok_or(StatusCode::NOT_FOUND)?;
    
    let response = CameraHealthResponse {
        camera_id: camera.id,
        state: match camera.status {
            CameraStatus::Online => ApiHealthState::Online,
            CameraStatus::Offline => ApiHealthState::Offline,
            _ => ApiHealthState::Unknown,
        },
        last_success: camera.last_seen,
        last_failure: None,
        consecutive_successes: if camera.status == CameraStatus::Online { 1 } else { 0 },
        consecutive_failures: if camera.status == CameraStatus::Offline { 1 } else { 0 },
        avg_latency_ms: None,
    };

    Ok(Json(ApiResponse::ok(response)))
}

/// GET /api/health/{id}/history - Get health check history for a camera
async fn get_health_history(
    State(_state): State<Arc<ServerState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ApiResponse<HealthHistoryResponse>>, StatusCode> {
    // In a real implementation, we'd query the database
    Ok(Json(ApiResponse::ok(HealthHistoryResponse {
        camera_id: id,
        checks: vec![],
        total_count: 0,
    })))
}

/// GET /api/events - List all events
async fn list_events(
    State(_state): State<Arc<ServerState>>,
    Query(_query): Query<EventsQuery>,
) -> Result<Json<ApiResponse<EventsResponse>>, StatusCode> {
    // In a real implementation, we'd query the database
    Ok(Json(ApiResponse::ok(EventsResponse {
        total: 0,
        events: vec![],
    })))
}

/// GET /api/cameras/{id}/events - Get events for a specific camera
async fn get_camera_events(
    State(_state): State<Arc<ServerState>>,
    Path(_id): Path<Uuid>,
    Query(_query): Query<EventsQuery>,
) -> Result<Json<ApiResponse<EventsResponse>>, StatusCode> {
    // In a real implementation, we'd query the database filtered by camera_id
    Ok(Json(ApiResponse::ok(EventsResponse {
        total: 0,
        events: vec![],
    })))
}

/// GET /api/cameras/{id}/combined-timeline - Get combined timeline view
async fn get_combined_timeline(
    State(state): State<Arc<ServerState>>,
    Path(id): Path<Uuid>,
    Query(query): Query<TimelineQuery>,
) -> Result<Json<ApiResponse<CombinedTimelineResponse>>, StatusCode> {
    let _camera = state.get_camera(&id).await.ok_or(StatusCode::NOT_FOUND)?;
    
    let now = chrono::Utc::now();
    let start_time = query.start_time.unwrap_or(now - chrono::Duration::hours(24));
    let end_time = query.end_time.unwrap_or(now);

    Ok(Json(ApiResponse::ok(CombinedTimelineResponse {
        camera_id: id,
        start_time,
        end_time,
        recordings: vec![],
        events: vec![],
        health_changes: vec![],
    })))
}

/// GET /api/timeline-ui/{camera_id} - Serve the timeline UI page
async fn timeline_ui_page(
    State(state): State<Arc<ServerState>>,
    Path(camera_id): Path<Uuid>,
) -> Result<Html<String>, StatusCode> {
    let camera = state.get_camera(&camera_id).await;
    let camera_name = camera
        .as_ref()
        .map(|c| c.name.clone())
        .unwrap_or_else(|| "Unknown Camera".to_string());

    Ok(Html(generate_timeline_html(&camera_name, &camera_id)))
}

/// Generate the timeline UI HTML
fn generate_timeline_html(camera_name: &str, camera_id: &Uuid) -> String {
    format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Timeline - {camera_name}</title>
    <style>
        :root {{
            --bg-dark: #1a1a2e;
            --bg-card: #16213e;
            --accent: #0f3460;
            --highlight: #e94560;
            --text: #eee;
            --text-dim: #888;
            --online: #4ade80;
            --offline: #f87171;
            --motion: #60a5fa;
            --recording: #4ade80;
        }}
        
        * {{ box-sizing: border-box; margin: 0; padding: 0; }}
        
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: var(--bg-dark);
            color: var(--text);
            min-height: 100vh;
        }}
        
        .header {{
            background: var(--bg-card);
            padding: 1rem 2rem;
            display: flex;
            justify-content: space-between;
            align-items: center;
            border-bottom: 1px solid var(--accent);
        }}
        
        .header h1 {{
            font-size: 1.5rem;
            font-weight: 500;
        }}
        
        .header .camera-name {{
            color: var(--highlight);
        }}
        
        .container {{
            max-width: 1400px;
            margin: 0 auto;
            padding: 2rem;
        }}
        
        .health-status {{
            display: flex;
            gap: 1rem;
            margin-bottom: 2rem;
        }}
        
        .health-card {{
            background: var(--bg-card);
            padding: 1rem 1.5rem;
            border-radius: 8px;
            flex: 1;
        }}
        
        .health-card h3 {{
            font-size: 0.875rem;
            color: var(--text-dim);
            margin-bottom: 0.5rem;
        }}
        
        .health-card .value {{
            font-size: 1.5rem;
            font-weight: 600;
        }}
        
        .health-card .value.online {{ color: var(--online); }}
        .health-card .value.offline {{ color: var(--offline); }}
        
        .timeline-controls {{
            background: var(--bg-card);
            padding: 1rem;
            border-radius: 8px;
            margin-bottom: 1rem;
            display: flex;
            gap: 1rem;
            align-items: center;
        }}
        
        .timeline-controls select,
        .timeline-controls input {{
            background: var(--accent);
            border: none;
            color: var(--text);
            padding: 0.5rem 1rem;
            border-radius: 4px;
            font-size: 0.875rem;
        }}
        
        .timeline-container {{
            background: var(--bg-card);
            border-radius: 8px;
            padding: 1rem;
            margin-bottom: 2rem;
        }}
        
        .timeline-header {{
            display: flex;
            justify-content: space-between;
            margin-bottom: 1rem;
        }}
        
        .timeline {{
            position: relative;
            height: 80px;
            background: var(--accent);
            border-radius: 4px;
            overflow: hidden;
        }}
        
        .timeline-ruler {{
            position: absolute;
            top: 0;
            left: 0;
            right: 0;
            height: 20px;
            display: flex;
            border-bottom: 1px solid var(--bg-dark);
        }}
        
        .timeline-tick {{
            flex: 1;
            border-right: 1px solid var(--bg-dark);
            font-size: 0.625rem;
            padding: 2px 4px;
            color: var(--text-dim);
        }}
        
        .timeline-track {{
            position: absolute;
            top: 25px;
            left: 0;
            right: 0;
            height: 20px;
        }}
        
        .timeline-segment {{
            position: absolute;
            height: 100%;
            background: var(--recording);
            opacity: 0.8;
            border-radius: 2px;
        }}
        
        .timeline-segment.cold {{
            background: #6366f1;
        }}
        
        .events-track {{
            position: absolute;
            top: 50px;
            left: 0;
            right: 0;
            height: 20px;
        }}
        
        .event-marker {{
            position: absolute;
            width: 8px;
            height: 8px;
            border-radius: 50%;
            background: var(--motion);
            top: 50%;
            transform: translate(-50%, -50%);
            cursor: pointer;
        }}
        
        .event-marker.motion {{ background: var(--motion); }}
        .event-marker.person {{ background: var(--highlight); }}
        .event-marker.vehicle {{ background: #fbbf24; }}
        
        .playhead {{
            position: absolute;
            top: 0;
            bottom: 0;
            width: 2px;
            background: var(--highlight);
            cursor: ew-resize;
        }}
        
        .playhead::after {{
            content: '';
            position: absolute;
            top: 0;
            left: -4px;
            border-left: 5px solid transparent;
            border-right: 5px solid transparent;
            border-top: 8px solid var(--highlight);
        }}
        
        .events-list {{
            background: var(--bg-card);
            border-radius: 8px;
            padding: 1rem;
        }}
        
        .events-list h2 {{
            font-size: 1rem;
            margin-bottom: 1rem;
            color: var(--text-dim);
        }}
        
        .event-item {{
            display: flex;
            align-items: center;
            gap: 1rem;
            padding: 0.75rem;
            background: var(--accent);
            border-radius: 4px;
            margin-bottom: 0.5rem;
            cursor: pointer;
            transition: background 0.2s;
        }}
        
        .event-item:hover {{
            background: #1e3a5f;
        }}
        
        .event-icon {{
            width: 32px;
            height: 32px;
            border-radius: 50%;
            background: var(--motion);
            display: flex;
            align-items: center;
            justify-content: center;
            font-size: 1rem;
        }}
        
        .event-icon.motion {{ background: var(--motion); }}
        .event-icon.person {{ background: var(--highlight); }}
        
        .event-details {{
            flex: 1;
        }}
        
        .event-type {{
            font-weight: 500;
        }}
        
        .event-time {{
            font-size: 0.75rem;
            color: var(--text-dim);
        }}
        
        .event-confidence {{
            font-size: 0.75rem;
            color: var(--text-dim);
        }}
        
        .no-events {{
            text-align: center;
            padding: 2rem;
            color: var(--text-dim);
        }}
        
        .live-link {{
            background: var(--highlight);
            color: white;
            padding: 0.5rem 1rem;
            border-radius: 4px;
            text-decoration: none;
            font-size: 0.875rem;
        }}
        
        .live-link:hover {{
            opacity: 0.9;
        }}
    </style>
</head>
<body>
    <div class="header">
        <h1>📹 <span class="camera-name">{camera_name}</span> Timeline</h1>
        <a href="/viewer/{camera_id}" class="live-link">🔴 Live View</a>
    </div>
    
    <div class="container">
        <div class="health-status">
            <div class="health-card">
                <h3>Status</h3>
                <div class="value online" id="health-status">Online</div>
            </div>
            <div class="health-card">
                <h3>Last Seen</h3>
                <div class="value" id="last-seen">Just now</div>
            </div>
            <div class="health-card">
                <h3>Avg Latency</h3>
                <div class="value" id="avg-latency">-- ms</div>
            </div>
            <div class="health-card">
                <h3>Events Today</h3>
                <div class="value" id="events-count">0</div>
            </div>
        </div>
        
        <div class="timeline-controls">
            <label>Range:</label>
            <select id="time-range">
                <option value="1">Last 1 hour</option>
                <option value="6">Last 6 hours</option>
                <option value="24" selected>Last 24 hours</option>
                <option value="168">Last 7 days</option>
            </select>
            <label>Event Filter:</label>
            <select id="event-filter">
                <option value="all">All Events</option>
                <option value="motion">Motion</option>
                <option value="person">Person</option>
                <option value="vehicle">Vehicle</option>
            </select>
        </div>
        
        <div class="timeline-container">
            <div class="timeline-header">
                <span id="timeline-start">00:00</span>
                <span id="timeline-end">24:00</span>
            </div>
            <div class="timeline" id="timeline">
                <div class="timeline-ruler" id="timeline-ruler"></div>
                <div class="timeline-track" id="recording-track"></div>
                <div class="events-track" id="events-track"></div>
                <div class="playhead" id="playhead" style="left: 50%;"></div>
            </div>
        </div>
        
        <div class="events-list">
            <h2>Recent Events</h2>
            <div id="events-container">
                <div class="no-events">No events in selected time range</div>
            </div>
        </div>
    </div>
    
    <script>
        const CAMERA_ID = '{camera_id}';
        const API_BASE = '/api';
        
        // State
        let currentRange = 24; // hours
        let events = [];
        let recordings = [];
        
        // Initialize
        async function init() {{
            await loadHealth();
            await loadTimeline();
            setupTimelineInteraction();
            
            // Refresh periodically
            setInterval(loadHealth, 30000);
            setInterval(loadTimeline, 60000);
        }}
        
        async function loadHealth() {{
            try {{
                const resp = await fetch(`${{API_BASE}}/health/${{CAMERA_ID}}`);
                const data = await resp.json();
                if (data.success && data.data) {{
                    updateHealthDisplay(data.data);
                }}
            }} catch (e) {{
                console.error('Failed to load health:', e);
            }}
        }}
        
        function updateHealthDisplay(health) {{
            const statusEl = document.getElementById('health-status');
            statusEl.textContent = health.state.charAt(0).toUpperCase() + health.state.slice(1);
            statusEl.className = 'value ' + health.state;
            
            if (health.lastSuccess) {{
                const lastSeen = new Date(health.lastSuccess);
                const diff = Date.now() - lastSeen.getTime();
                const minutes = Math.floor(diff / 60000);
                document.getElementById('last-seen').textContent = 
                    minutes < 1 ? 'Just now' : `${{minutes}}m ago`;
            }}
            
            if (health.avgLatencyMs) {{
                document.getElementById('avg-latency').textContent = 
                    `${{Math.round(health.avgLatencyMs)}} ms`;
            }}
        }}
        
        async function loadTimeline() {{
            const hours = parseInt(document.getElementById('time-range').value);
            const endTime = new Date().toISOString();
            const startTime = new Date(Date.now() - hours * 3600000).toISOString();
            
            try {{
                const resp = await fetch(
                    `${{API_BASE}}/cameras/${{CAMERA_ID}}/combined-timeline?startTime=${{startTime}}&endTime=${{endTime}}`
                );
                const data = await resp.json();
                if (data.success && data.data) {{
                    renderTimeline(data.data, hours);
                }}
            }} catch (e) {{
                console.error('Failed to load timeline:', e);
            }}
        }}
        
        function renderTimeline(data, hours) {{
            const startTime = new Date(data.startTime).getTime();
            const endTime = new Date(data.endTime).getTime();
            const duration = endTime - startTime;
            
            // Update ruler
            const ruler = document.getElementById('timeline-ruler');
            ruler.innerHTML = '';
            const tickCount = Math.min(12, hours);
            for (let i = 0; i <= tickCount; i++) {{
                const tick = document.createElement('div');
                tick.className = 'timeline-tick';
                const time = new Date(startTime + (duration * i / tickCount));
                tick.textContent = time.toLocaleTimeString([], {{hour: '2-digit', minute: '2-digit'}});
                ruler.appendChild(tick);
            }}
            
            // Render recordings
            const recordingTrack = document.getElementById('recording-track');
            recordingTrack.innerHTML = '';
            data.recordings.forEach(seg => {{
                const segStart = new Date(seg.startTime).getTime();
                const segEnd = new Date(seg.endTime).getTime();
                const left = ((segStart - startTime) / duration) * 100;
                const width = ((segEnd - segStart) / duration) * 100;
                
                const el = document.createElement('div');
                el.className = 'timeline-segment' + (seg.isCold ? ' cold' : '');
                el.style.left = `${{Math.max(0, left)}}%`;
                el.style.width = `${{Math.min(100 - left, width)}}%`;
                el.title = `${{new Date(seg.startTime).toLocaleString()}} - ${{seg.durationSecs.toFixed(0)}}s`;
                recordingTrack.appendChild(el);
            }});
            
            // Render events
            const eventsTrack = document.getElementById('events-track');
            eventsTrack.innerHTML = '';
            data.events.forEach(evt => {{
                const evtTime = new Date(evt.timestamp).getTime();
                const left = ((evtTime - startTime) / duration) * 100;
                
                const el = document.createElement('div');
                el.className = 'event-marker ' + getEventClass(evt.eventType);
                el.style.left = `${{left}}%`;
                el.title = `${{evt.eventType}} at ${{new Date(evt.timestamp).toLocaleTimeString()}}`;
                el.onclick = () => showEventDetails(evt);
                eventsTrack.appendChild(el);
            }});
            
            // Update events list
            renderEventsList(data.events);
            document.getElementById('events-count').textContent = data.events.length;
        }}
        
        function getEventClass(type) {{
            if (type.includes('person')) return 'person';
            if (type.includes('vehicle')) return 'vehicle';
            return 'motion';
        }}
        
        function renderEventsList(events) {{
            const container = document.getElementById('events-container');
            if (events.length === 0) {{
                container.innerHTML = '<div class="no-events">No events in selected time range</div>';
                return;
            }}
            
            container.innerHTML = events.slice(0, 20).map(evt => `
                <div class="event-item" onclick="showEventDetails(${{JSON.stringify(evt).replace(/"/g, '&quot;')}})">
                    <div class="event-icon ${{getEventClass(evt.eventType)}}">${{getEventIcon(evt.eventType)}}</div>
                    <div class="event-details">
                        <div class="event-type">${{formatEventType(evt.eventType)}}</div>
                        <div class="event-time">${{new Date(evt.timestamp).toLocaleString()}}</div>
                    </div>
                    ${{evt.confidence ? `<div class="event-confidence">${{(evt.confidence * 100).toFixed(0)}}%</div>` : ''}}
                </div>
            `).join('');
        }}
        
        function getEventIcon(type) {{
            if (type.includes('person')) return '🚶';
            if (type.includes('vehicle')) return '🚗';
            if (type.includes('motion')) return '📍';
            if (type.includes('audio')) return '🔊';
            return '⚡';
        }}
        
        function formatEventType(type) {{
            return type.split('_').map(w => w.charAt(0).toUpperCase() + w.slice(1)).join(' ');
        }}
        
        function showEventDetails(evt) {{
            console.log('Event details:', evt);
            // TODO: Show modal or navigate to clip
        }}
        
        function setupTimelineInteraction() {{
            const timeline = document.getElementById('timeline');
            const playhead = document.getElementById('playhead');
            
            let dragging = false;
            
            playhead.addEventListener('mousedown', () => dragging = true);
            document.addEventListener('mouseup', () => dragging = false);
            document.addEventListener('mousemove', (e) => {{
                if (!dragging) return;
                const rect = timeline.getBoundingClientRect();
                const x = Math.max(0, Math.min(rect.width, e.clientX - rect.left));
                playhead.style.left = `${{(x / rect.width) * 100}}%`;
            }});
            
            timeline.addEventListener('click', (e) => {{
                if (e.target === playhead) return;
                const rect = timeline.getBoundingClientRect();
                const x = e.clientX - rect.left;
                playhead.style.left = `${{(x / rect.width) * 100}}%`;
            }});
        }}
        
        // Event handlers
        document.getElementById('time-range').addEventListener('change', loadTimeline);
        document.getElementById('event-filter').addEventListener('change', loadTimeline);
        
        // Start
        init();
    </script>
</body>
</html>"#, camera_name = camera_name, camera_id = camera_id)
}
