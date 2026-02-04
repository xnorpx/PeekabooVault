//! RTSP Client - Wrapper around retina for camera streaming
//!
//! Provides RTSP client functionality for connecting to IP cameras and
//! receiving video streams. Integrates with the Stream Manager.

use crate::stream_manager::{StreamChunk, StreamMetadata, StreamSession, VideoCodec};
use anyhow::{Context, Result};
use bytes::Bytes;
use futures::StreamExt;
use parking_lot::RwLock;
use retina::client::{Demuxed, SetupOptions, SessionGroup, SessionOptions, Transport};
use retina::codec::{CodecItem, ParametersRef, VideoFrame};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use url::Url;

/// RTSP client configuration
#[derive(Debug, Clone)]
pub struct RtspConfig {
    /// RTSP URL
    pub url: String,
    /// Username for authentication
    pub username: Option<String>,
    /// Password for authentication
    pub password: Option<String>,
    /// Use TCP transport (more reliable, default)
    pub use_tcp: bool,
    /// Timeout for connection in seconds
    pub timeout_secs: u64,
}

impl Default for RtspConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            username: None,
            password: None,
            use_tcp: true,
            timeout_secs: 10,
        }
    }
}

/// RTSP client handle for controlling a connection
pub struct RtspClient {
    config: RtspConfig,
    state: Arc<RwLock<ClientState>>,
    /// Channel to signal shutdown
    shutdown_tx: Option<mpsc::Sender<()>>,
    /// Optional message sender for ServerTask integration (NEW architecture)
    server_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::server_task::ServerMessage>>,
    /// Camera ID (for ServerTask message-passing)
    camera_id: Option<uuid::Uuid>,
    /// Stream type (for ServerTask message-passing, e.g., "main", "sub")
    stream_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Variants used for state machine, some not yet wired up
enum ClientState {
    Idle,
    Connecting,
    Running,
    Stopping,
    Error,
}

impl RtspClient {
    /// Create a new RTSP client
    pub fn new(config: RtspConfig) -> Self {
        Self {
            config,
            state: Arc::new(RwLock::new(ClientState::Idle)),
            shutdown_tx: None,
            server_tx: None,
            camera_id: None,
            stream_type: None,
        }
    }

    /// Set ServerTask message sender (for message-passing architecture)
    pub fn with_server_task(
        mut self,
        tx: tokio::sync::mpsc::UnboundedSender<crate::server_task::ServerMessage>,
        camera_id: uuid::Uuid,
        stream_type: String,
    ) -> Self {
        self.server_tx = Some(tx);
        self.camera_id = Some(camera_id);
        self.stream_type = Some(stream_type);
        self
    }

    /// Start streaming to a session
    pub async fn start(&mut self, session: Arc<StreamSession>) -> Result<()> {
        if *self.state.read() != ClientState::Idle {
            anyhow::bail!("Client already running or stopping");
        }

        *self.state.write() = ClientState::Connecting;

        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        self.shutdown_tx = Some(shutdown_tx);

        let config = self.config.clone();
        let state = self.state.clone();
        let server_tx = self.server_tx.clone();
        let camera_id = self.camera_id;
        let stream_type = self.stream_type.clone();

        tokio::spawn(async move {
            let result = run_stream(
                config,
                session.clone(),
                shutdown_rx,
                server_tx.clone(),
                camera_id,
                stream_type.clone(),
            )
            .await;

            match &result {
                Ok(()) => {
                    info!("RTSP stream ended normally");
                    session.on_disconnected(false);

                    // Send disconnected message to ServerTask if available
                    if let (Some(tx), Some(cam_id), Some(stream)) = (server_tx, camera_id, stream_type) {
                        let _ = tx.send(crate::server_task::ServerMessage::RtspDisconnected {
                            camera_id: cam_id,
                            stream_type: stream,
                            error: None,
                        });
                    }
                }
                Err(e) => {
                    error!("RTSP stream error: {}", e);
                    session.on_disconnected(true); // Will try to reconnect

                    // Send disconnected message to ServerTask if available
                    if let (Some(tx), Some(cam_id), Some(stream)) = (server_tx, camera_id, stream_type) {
                        let _ = tx.send(crate::server_task::ServerMessage::RtspDisconnected {
                            camera_id: cam_id,
                            stream_type: stream,
                            error: Some(e.to_string()),
                        });
                    }
                }
            }

            *state.write() = ClientState::Idle;
        });

        Ok(())
    }

    /// Stop the stream
    pub async fn stop(&mut self) {
        *self.state.write() = ClientState::Stopping;
        
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()).await;
        }
    }

    /// Check if the client is running
    pub fn is_running(&self) -> bool {
        matches!(*self.state.read(), ClientState::Connecting | ClientState::Running)
    }
}

/// Main streaming loop
async fn run_stream(
    config: RtspConfig,
    session: Arc<StreamSession>,
    mut shutdown_rx: mpsc::Receiver<()>,
    server_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::server_task::ServerMessage>>,
    camera_id: Option<uuid::Uuid>,
    stream_type: Option<String>,
) -> Result<()> {
    let url = Url::parse(&config.url).context("Invalid RTSP URL")?;
    
    // Build credentials
    let creds = match (&config.username, &config.password) {
        (Some(u), Some(p)) => Some(retina::client::Credentials {
            username: u.clone(),
            password: p.clone(),
        }),
        _ => None,
    };

    // Session options
    let session_opts = SessionOptions::default()
        .creds(creds);

    // Transport
    let transport = if config.use_tcp {
        Transport::Tcp(retina::client::TcpTransportOptions::default())
    } else {
        Transport::Udp(Default::default())
    };

    // Connect
    info!(url = %config.url, "Connecting to RTSP stream");
    
    let _session_group = Arc::new(SessionGroup::default());
    let mut rtsp_session = retina::client::Session::describe(url, session_opts)
        .await
        .context("Failed to describe RTSP session")?;

    // Find video stream
    let video_stream_idx = rtsp_session
        .streams()
        .iter()
        .position(|s| s.media() == "video")
        .context("No video stream found")?;

    // Setup video stream
    let setup_opts = SetupOptions::default().transport(transport);
    rtsp_session
        .setup(video_stream_idx, setup_opts)
        .await
        .context("Failed to setup video stream")?;

    // Detect codec from stream parameters
    let stream = &rtsp_session.streams()[video_stream_idx];
    let codec = detect_codec(stream);

    // Extract codec parameters
    let metadata = extract_metadata(stream, codec);
    let width = metadata.width;
    let height = metadata.height;
    session.set_metadata(metadata);

    info!(
        codec = %codec,
        "Video stream setup complete"
    );

    // Start playback
    let demuxed = rtsp_session
        .play(retina::client::PlayOptions::default())
        .await
        .context("Failed to start playback")?
        .demuxed()
        .context("Failed to demux stream")?;

    session.on_connected();

    // Send connected message to ServerTask if available
    if let (Some(tx), Some(cam_id), Some(stream)) = (&server_tx, camera_id, &stream_type) {
        let _ = tx.send(crate::server_task::ServerMessage::RtspConnected {
            camera_id: cam_id,
            stream_type: stream.clone(),
            codec_info: crate::server_task::CodecInfo {
                codec: codec.to_string(),
                width,
                height,
            },
        });
    }

    // Process frames
    process_frames(
        demuxed,
        session,
        &mut shutdown_rx,
        video_stream_idx,
        server_tx,
        camera_id,
        stream_type,
    )
    .await
}

/// Detect video codec from stream
fn detect_codec(stream: &retina::client::Stream) -> VideoCodec {
    let encoding = stream.encoding_name();
    match encoding.to_uppercase().as_str() {
        "H264" => VideoCodec::H264,
        "H265" | "HEVC" => VideoCodec::H265,
        _ => {
            warn!(encoding = %encoding, "Unknown video encoding");
            VideoCodec::Unknown
        }
    }
}

/// Extract metadata from stream parameters
fn extract_metadata(stream: &retina::client::Stream, codec: VideoCodec) -> StreamMetadata {
    let mut metadata = StreamMetadata {
        codec,
        ..Default::default()
    };

    // Try to get SPS/PPS from stream parameters
    if let Some(ParametersRef::Video(video)) = stream.parameters() {
        // Get dimensions if available
        let dims = video.pixel_dimensions();
        metadata.width = Some(dims.0);
        metadata.height = Some(dims.1);

        // Get codec-specific data
        let extra_data = video.extra_data();
        if !extra_data.is_empty() {
            // For H.264, this contains SPS/PPS in AVC format
            // For H.265, this contains VPS/SPS/PPS in HEVC format
            // We'll parse these when needed
            debug!(len = %extra_data.len(), "Got codec extra data");
        }
    }

    metadata
}

/// Process frames from the demuxed stream
async fn process_frames(
    mut demuxed: Demuxed,
    session: Arc<StreamSession>,
    shutdown_rx: &mut mpsc::Receiver<()>,
    video_stream_idx: usize,
    server_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::server_task::ServerMessage>>,
    camera_id: Option<uuid::Uuid>,
    stream_type: Option<String>,
) -> Result<()> {
    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                info!("Shutdown signal received");
                break;
            }
            item = demuxed.next() => {
                match item {
                    Some(Ok(item)) => {
                        if let CodecItem::VideoFrame(frame) = item
                            && frame.stream_id() == video_stream_idx
                        {
                            handle_video_frame(
                                &session,
                                frame,
                                server_tx.as_ref(),
                                camera_id,
                                stream_type.as_ref(),
                            );
                        }
                    }
                    Some(Err(e)) => {
                        error!("Stream error: {}", e);
                        return Err(anyhow::anyhow!("Stream error: {}", e));
                    }
                    None => {
                        info!("Stream ended");
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

/// Handle a received video frame
fn handle_video_frame(
    session: &StreamSession,
    frame: VideoFrame,
    server_tx: Option<&tokio::sync::mpsc::UnboundedSender<crate::server_task::ServerMessage>>,
    camera_id: Option<uuid::Uuid>,
    stream_type: Option<&String>,
) {
    let is_keyframe = frame.is_random_access_point();
    let timestamp = frame.timestamp().timestamp();

    // retina's into_data() returns Vec<u8>, wrap in Bytes for zero-copy sharing
    let data = Bytes::from(frame.into_data());

    // Send to ServerTask if available (NEW architecture)
    if let (Some(tx), Some(cam_id), Some(stream)) = (server_tx, camera_id, stream_type) {
        let _ = tx.send(crate::server_task::ServerMessage::RtspFrame {
            camera_id: cam_id,
            stream_type: stream.clone(),
            frame: crate::server_task::VideoFrame {
                data: data.to_vec(),
                timestamp,
                is_keyframe,
            },
        });
    }

    // Also send to StreamSession (OLD architecture, for backward compatibility)
    let chunk = StreamChunk {
        data,
        timestamp: timestamp as u32,
        is_keyframe,
        received_at: Instant::now(),
        codec_extra: None,
    };

    session.on_chunk(chunk);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rtsp_config_default() {
        let config = RtspConfig::default();
        assert!(config.use_tcp);
        assert_eq!(config.timeout_secs, 10);
    }

    #[test]
    fn test_client_creation() {
        let config = RtspConfig {
            url: "rtsp://test:554/stream".to_string(),
            ..Default::default()
        };
        let client = RtspClient::new(config);
        assert!(!client.is_running());
    }
}
