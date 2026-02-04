//! WebRTC Session Manager - Low-latency live video via str0m
//!
//! Architecture:
//! ```text
//!                                    ┌─────────────────────┐
//!                                    │   Browser/Client    │
//!                                    └──────────┬──────────┘
//!                                               │ UDP
//!                                               ▼
//!     ┌─────────────────────────────────────────────────────────────────┐
//!     │                    Single UDP Socket (Arc)                      │
//!     └─────────────────────────────────────────────────────────────────┘
//!                                               │
//!                                               ▼
//!     ┌─────────────────────────────────────────────────────────────────┐
//!     │                     UDP Receive Task                            │
//!     │  - recv_from() loop                                             │
//!     │  - Parse STUN to extract ufrag for demux                        │
//!     │  - Route to correct session via channels                        │
//!     └─────────────────────────────────────────────────────────────────┘
//!                         │              │              │
//!                         ▼              ▼              ▼
//!     ┌─────────────┐ ┌─────────────┐ ┌─────────────┐
//!     │ Session A   │ │ Session B   │ │ Session C   │
//!     │ (str0m Rtc) │ │ (str0m Rtc) │ │ (str0m Rtc) │
//!     │             │ │             │ │             │
//!     │ Blocking    │ │ Blocking    │ │ Blocking    │
//!     │ poll loop   │ │ poll loop   │ │ poll loop   │
//!     └──────┬──────┘ └──────┬──────┘ └──────┬──────┘
//!            │               │               │
//!            └───────────────┴───────────────┘
//!                            │
//!                            ▼ send_to()
//!     ┌─────────────────────────────────────────────────────────────────┐
//!     │                    Single UDP Socket (Arc)                      │
//!     └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! Forwarding Pipeline:
//! ```text
//!     RTSP Client                Stream Manager              WebRTC
//!     ─────────────────────────────────────────────────────────────────
//!     retina frame ──► StreamChunk ──► mpsc::Receiver ──► VideoFrame
//!        │                  │                │                 │
//!     Vec<u8>            Bytes            Bytes            &[u8]
//!        │                  │                │                 │
//!        └──────────────────┴────────────────┴─────────────────┘
//!                     Zero-copy via Bytes (Arc)
//! ```
//!
//! This module handles:
//! - WebRTC signaling (offer/answer SDP negotiation)
//! - ICE candidate exchange
//! - RTP forwarding from camera streams to WebRTC peers
//! - Instant playback via prebuffer (no waiting for keyframe)
//! - Shared UDP socket with ufrag-based demultiplexing
//! - Fast source address cache for packet routing (halfbrown)

use crate::api::{
    DataChannelCommand, DataChannelMessage, LayerChangeNotification, LayerChangeReason,
    LayerPreference, LayerSelectionMode, PlaybackMode, StreamType, UiContext, VideoLayer,
};
use crate::stream_manager::{StreamChunk, StreamSession, VideoCodec as StreamCodec};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use halfbrown::HashMap as FastHashMap;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use str0m::bwe::Bitrate;
use str0m::channel::ChannelId;
use str0m::change::SdpOffer;
use str0m::ice::StunMessage;
use str0m::media::{MediaKind, MediaTime, Mid};
use str0m::net::{DatagramRecv, Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc, RtcError};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn};
use uuid::Uuid;

/// WebRTC session configuration
#[derive(Debug, Clone)]
pub struct WebRtcConfig {
    /// STUN server URL
    pub stun_server: Option<String>,
    /// ICE lite mode (server doesn't gather candidates)
    pub ice_lite: bool,
    /// Maximum sessions per camera
    pub max_sessions_per_camera: usize,
    /// Session timeout in seconds
    pub session_timeout_secs: u64,
    /// Enable bandwidth estimation (BWE)
    pub enable_bwe: bool,
    /// Initial bandwidth estimate (kbps) for BWE
    pub initial_bandwidth_kbps: u32,
}

impl Default for WebRtcConfig {
    fn default() -> Self {
        Self {
            stun_server: Some("stun:stun.l.google.com:19302".to_string()),
            ice_lite: true,
            max_sessions_per_camera: 10,
            session_timeout_secs: 300,
            enable_bwe: true,
            initial_bandwidth_kbps: 2500, // Start assuming decent connection
        }
    }
}

/// A video frame to send via WebRTC
#[derive(Debug, Clone)]
pub struct VideoFrame {
    /// Raw NAL unit data
    pub data: Bytes,
    /// RTP timestamp (90kHz)
    pub rtp_timestamp: u32,
    /// Wallclock time when frame was captured
    pub wallclock: Instant,
    /// Whether this is a keyframe
    pub is_keyframe: bool,
    /// Codec (h264/h265)
    pub codec: VideoCodec,
}

/// Video codec type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    H265,
}

/// WebRTC session state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Waiting for SDP offer
    New,
    /// SDP negotiation in progress
    Negotiating,
    /// ICE gathering/connecting
    Connecting,
    /// Session active
    Active,
    /// Session closing
    Closing,
    /// Session closed
    Closed,
}

/// Network output from a WebRTC session
#[derive(Debug)]
pub struct NetworkOutput {
    /// Destination address
    pub destination: SocketAddr,
    /// Data to send
    pub contents: Vec<u8>,
}

// ============================================================================
// UDP Transport Infrastructure
// ============================================================================

/// Incoming UDP packet for routing to a session
#[derive(Debug)]
pub struct IncomingPacket {
    /// Source address of the packet
    pub source: SocketAddr,
    /// Raw packet data
    pub data: Vec<u8>,
    /// Receive timestamp
    pub received_at: Instant,
}

/// Channel sender for routing packets to a session
pub type SessionPacketSender = mpsc::UnboundedSender<IncomingPacket>;
/// Channel receiver for a session to receive packets
pub type SessionPacketReceiver = mpsc::UnboundedReceiver<IncomingPacket>;

/// Shared UDP transport for all WebRTC sessions
pub struct UdpTransport {
    /// The shared UDP socket
    socket: Arc<UdpSocket>,
    /// Local address the socket is bound to
    local_addr: SocketAddr,
}

impl UdpTransport {
    /// Create a new UDP transport bound to the given address
    pub async fn bind(addr: SocketAddr) -> Result<Self, WebRtcError> {
        let socket = UdpSocket::bind(addr)
            .await
            .map_err(|e| WebRtcError::Io(e.to_string()))?;
        
        let local_addr = socket
            .local_addr()
            .map_err(|e| WebRtcError::Io(e.to_string()))?;
        
        info!("WebRTC UDP transport bound to {}", local_addr);
        
        Ok(Self {
            socket: Arc::new(socket),
            local_addr,
        })
    }
    
    /// Get the local address
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
    
    /// Get a clone of the socket Arc for sending
    pub fn socket(&self) -> Arc<UdpSocket> {
        Arc::clone(&self.socket)
    }
    
    /// Send data to a destination
    pub async fn send_to(&self, data: &[u8], dest: SocketAddr) -> Result<usize, WebRtcError> {
        self.socket
            .send_to(data, dest)
            .await
            .map_err(|e| WebRtcError::Io(e.to_string()))
    }
    
    /// Receive data from the socket
    /// Returns (data, source_addr)
    pub async fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr), WebRtcError> {
        self.socket
            .recv_from(buf)
            .await
            .map_err(|e| WebRtcError::Io(e.to_string()))
    }
}

/// Extract the local ICE ufrag from a STUN binding request
/// Used for demultiplexing incoming packets to the correct session
///
/// In STUN binding requests, the USERNAME attribute contains "local:remote" ufrags
/// We extract the local ufrag (our server's ufrag) to identify which session owns this packet
pub fn extract_local_ufrag_from_stun(data: &[u8]) -> Option<String> {
    // Try to parse as STUN message
    let stun = StunMessage::parse(data).ok()?;
    
    // Extract username attribute which contains "local_ufrag:remote_ufrag"
    let (local_ufrag, _remote_ufrag) = stun.split_username()?;
    
    Some(local_ufrag.to_string())
}

/// Check if a packet is a STUN message (first byte 0 or 1, length >= 20)
pub fn is_stun_packet(data: &[u8]) -> bool {
    if data.len() < 20 {
        return false;
    }
    // STUN messages have first byte 0b00xxxxxx (0 or 1)
    data[0] < 2
}

/// A WebRTC session for a single viewer
pub struct WebRtcSession {
    /// Session ID
    pub id: Uuid,
    /// Camera ID being viewed
    pub camera_id: Uuid,
    /// Stream type (main/sub)
    pub stream_type: String,
    /// Session state
    state: RwLock<SessionState>,
    /// str0m RTC instance - needs exclusive access
    rtc: RwLock<Rtc>,
    /// Video media ID
    video_mid: RwLock<Option<Mid>>,
    /// Data channel ID (for control messages)
    data_channel_id: RwLock<Option<ChannelId>>,
    /// Playback state
    playback_state: RwLock<PlaybackState>,
    /// Layer state for adaptive streaming
    layer_state: RwLock<LayerState>,
    /// Created timestamp
    pub created_at: Instant,
    /// Last activity timestamp
    last_activity: RwLock<Instant>,
    /// Channel for sending commands to the playback handler
    command_tx: mpsc::UnboundedSender<DataChannelCommand>,
    /// Channel for receiving commands (used by playback handler)
    command_rx: RwLock<Option<mpsc::UnboundedReceiver<DataChannelCommand>>>,
    /// Channel for internal stream switch requests (from BWE events)
    stream_switch_tx: mpsc::UnboundedSender<StreamType>,
    /// Receiver for stream switch requests
    stream_switch_rx: RwLock<Option<mpsc::UnboundedReceiver<StreamType>>>,
}

/// Playback state for a session
#[derive(Debug, Clone)]
pub struct PlaybackState {
    /// Current mode (live or replay)
    pub mode: PlaybackMode,
    /// Whether currently playing
    pub is_playing: bool,
    /// Playback speed (1.0 = normal)
    pub speed: f32,
    /// Current position in replay mode
    pub current_timestamp: Option<DateTime<Utc>>,
    /// Current position in 90kHz ticks
    pub current_pts: i64,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            mode: PlaybackMode::Live,
            is_playing: true,
            speed: 1.0,
            current_timestamp: None,
            current_pts: 0,
        }
    }
}

/// Layer state for adaptive streaming
#[derive(Debug, Clone)]
pub struct LayerState {
    /// Current active layer
    pub active_layer: VideoLayer,
    /// Client's preference
    pub preference: LayerPreference,
    /// Current UI context hint
    pub ui_context: Option<UiContext>,
    /// Available layers for this stream
    pub available_layers: Vec<VideoLayer>,
    /// Current stream type
    pub stream_type: StreamType,
    /// Estimated bandwidth (kbps) based on RTCP feedback
    pub estimated_bandwidth_kbps: Option<u32>,
    /// Last layer change timestamp
    pub last_layer_change: Option<Instant>,
}

impl Default for LayerState {
    fn default() -> Self {
        Self {
            active_layer: VideoLayer::Auto,
            preference: LayerPreference::default(),
            ui_context: None,
            available_layers: vec![VideoLayer::High, VideoLayer::Low],
            stream_type: StreamType::Main,
            estimated_bandwidth_kbps: None,
            last_layer_change: None,
        }
    }
}

impl LayerState {
    /// Determine the effective layer based on mode and preferences
    pub fn compute_effective_layer(&self) -> VideoLayer {
        match self.preference.mode {
            LayerSelectionMode::Manual => self.preference.preferred_layer,
            LayerSelectionMode::Adaptive => {
                // Check bandwidth constraints
                if let (Some(max_bw), Some(est_bw)) = (
                    self.preference.max_bitrate_kbps,
                    self.estimated_bandwidth_kbps,
                ) {
                    if est_bw < max_bw / 2 {
                        return VideoLayer::Low;
                    } else if est_bw < max_bw {
                        return VideoLayer::Medium;
                    }
                }
                // Default to high for adaptive
                VideoLayer::High
            }
            LayerSelectionMode::UiContext => {
                self.ui_context
                    .map(|ctx| ctx.recommended_layer())
                    .unwrap_or(VideoLayer::High)
            }
        }
    }
    
    /// Check if we should switch layers
    pub fn should_switch(&self) -> Option<VideoLayer> {
        let effective = self.compute_effective_layer();
        if effective != self.active_layer {
            // Don't switch too frequently (min 2 seconds between switches)
            if let Some(last_change) = self.last_layer_change {
                if last_change.elapsed() < Duration::from_secs(2) {
                    return None;
                }
            }
            Some(effective)
        } else {
            None
        }
    }
}

impl WebRtcSession {
    /// Create a new WebRTC session
    pub fn new(
        camera_id: Uuid,
        stream_type: String,
        config: &WebRtcConfig,
    ) -> Result<Self, WebRtcError> {
        // Set up crypto provider (process-wide, safe to call multiple times)
        str0m_rust_crypto::default_provider().install_process_default();

        // Build RTC instance with optional BWE
        let mut builder = Rtc::builder().set_ice_lite(config.ice_lite);
        
        if config.enable_bwe {
            let initial_bitrate = Bitrate::kbps(config.initial_bandwidth_kbps as u64);
            builder = builder.enable_bwe(Some(initial_bitrate));
            debug!(
                initial_kbps = config.initial_bandwidth_kbps,
                "BWE enabled for WebRTC session"
            );
        }
        
        let rtc = builder.build(Instant::now());

        // Create command channel for data channel messages
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        
        // Create channel for internal stream switch requests (from BWE)
        let (stream_switch_tx, stream_switch_rx) = mpsc::unbounded_channel();
        
        // Initialize layer state based on stream type
        let initial_stream_type = if stream_type == "sub" {
            StreamType::Sub
        } else {
            StreamType::Main
        };
        let mut layer_state = LayerState::default();
        layer_state.stream_type = initial_stream_type;
        layer_state.active_layer = if initial_stream_type == StreamType::Main {
            VideoLayer::High
        } else {
            VideoLayer::Low
        };

        Ok(Self {
            id: Uuid::new_v4(),
            camera_id,
            stream_type,
            state: RwLock::new(SessionState::New),
            rtc: RwLock::new(rtc),
            video_mid: RwLock::new(None),
            data_channel_id: RwLock::new(None),
            playback_state: RwLock::new(PlaybackState::default()),
            layer_state: RwLock::new(layer_state),
            created_at: Instant::now(),
            last_activity: RwLock::new(Instant::now()),
            command_tx,
            command_rx: RwLock::new(Some(command_rx)),
            stream_switch_tx,
            stream_switch_rx: RwLock::new(Some(stream_switch_rx)),
        })
    }

    /// Take the command receiver (can only be called once)
    pub fn take_command_rx(&self) -> Option<mpsc::UnboundedReceiver<DataChannelCommand>> {
        self.command_rx.write().take()
    }
    
    /// Take the stream switch receiver (can only be called once)
    pub fn take_stream_switch_rx(&self) -> Option<mpsc::UnboundedReceiver<StreamType>> {
        self.stream_switch_rx.write().take()
    }
    
    /// Request a stream switch (called from BWE event handler)
    pub fn request_stream_switch(&self, stream_type: StreamType) {
        let _ = self.stream_switch_tx.send(stream_type);
    }

    /// Get a clone of the command sender
    pub fn command_sender(&self) -> mpsc::UnboundedSender<DataChannelCommand> {
        self.command_tx.clone()
    }

    /// Get current state
    pub fn state(&self) -> SessionState {
        *self.state.read()
    }

    /// Add a local ICE candidate (our server's address)
    pub fn add_local_candidate(&self, addr: SocketAddr) -> Result<(), WebRtcError> {
        let mut rtc = self.rtc.write();
        let candidate = Candidate::host(addr, Protocol::Udp)
            .map_err(|e| WebRtcError::IceCandidate(e.to_string()))?;
        rtc.add_local_candidate(candidate);
        Ok(())
    }

    /// Process an incoming SDP offer and generate an answer
    pub fn process_offer(&self, offer_sdp: &str) -> Result<String, WebRtcError> {
        let mut rtc = self.rtc.write();

        // Parse the SDP offer
        let offer = SdpOffer::from_sdp_string(offer_sdp)
            .map_err(|e| WebRtcError::SdpParse(e.to_string()))?;

        // Accept the offer and get an answer
        let answer = rtc
            .sdp_api()
            .accept_offer(offer)
            .map_err(|e| WebRtcError::SdpApply(e.to_string()))?;

        *self.state.write() = SessionState::Negotiating;
        *self.last_activity.write() = Instant::now();

        Ok(answer.to_sdp_string())
    }

    /// Add a remote ICE candidate from SDP string (a]candidate:... format)
    pub fn add_ice_candidate(&self, candidate_str: &str) -> Result<(), WebRtcError> {
        let mut rtc = self.rtc.write();

        // Parse the candidate from SDP format using from_sdp_string
        let candidate = Candidate::from_sdp_string(candidate_str)
            .map_err(|e| WebRtcError::IceCandidate(e.to_string()))?;

        rtc.add_remote_candidate(candidate);
        *self.last_activity.write() = Instant::now();
        Ok(())
    }

    /// Process incoming network data
    pub fn handle_receive(
        &self,
        now: Instant,
        source: SocketAddr,
        destination: SocketAddr,
        data: &[u8],
    ) -> Result<(), WebRtcError> {
        let mut rtc = self.rtc.write();

        let receive = Receive {
            proto: Protocol::Udp,
            source,
            destination,
            contents: data
                .try_into()
                .map_err(|_| WebRtcError::Input("Invalid packet data".into()))?,
        };

        rtc.handle_input(Input::Receive(now, receive))
            .map_err(|e| WebRtcError::Input(e.to_string()))?;

        *self.last_activity.write() = Instant::now();
        Ok(())
    }

    /// Poll the RTC instance and process events
    /// Returns network output if there's data to send
    pub fn poll(&self, now: Instant) -> Result<PollResult, WebRtcError> {
        let mut rtc = self.rtc.write();

        // Drive time forward
        rtc.handle_input(Input::Timeout(now))
            .map_err(|e| WebRtcError::Input(e.to_string()))?;

        // Poll for output
        match rtc
            .poll_output()
            .map_err(|e| WebRtcError::Input(e.to_string()))?
        {
            Output::Timeout(timeout) => Ok(PollResult::Timeout(timeout)),
            Output::Transmit(transmit) => Ok(PollResult::Transmit(NetworkOutput {
                destination: transmit.destination,
                contents: transmit.contents.to_vec(),
            })),
            Output::Event(event) => {
                // Handle state change events
                match &event {
                    Event::IceConnectionStateChange(state) => {
                        info!(session_id = %self.id, ?state, "ICE connection state changed");
                        match state {
                            IceConnectionState::Connected => {
                                *self.state.write() = SessionState::Active;
                            }
                            IceConnectionState::Disconnected => {
                                *self.state.write() = SessionState::Closing;
                            }
                            _ => {}
                        }
                    }
                    Event::MediaAdded(media) => {
                        debug!(session_id = %self.id, mid = ?media.mid, "Media added");
                        // Store the video mid for later use
                        if media.kind == MediaKind::Video {
                            *self.video_mid.write() = Some(media.mid);
                        }
                    }
                    Event::ChannelOpen(channel_id, label) => {
                        info!(session_id = %self.id, ?channel_id, ?label, "Data channel opened");
                        // Store the channel ID for sending messages
                        if label == "control" {
                            *self.data_channel_id.write() = Some(*channel_id);
                        }
                    }
                    Event::ChannelData(channel_data) => {
                        // Process incoming data channel message
                        debug!(
                            session_id = %self.id, 
                            channel_id = ?channel_data.id,
                            len = channel_data.data.len(),
                            "Received data channel message"
                        );
                        // Parse as JSON command
                        drop(rtc); // Release lock before processing
                        if let Err(e) = self.handle_data_channel_message(&channel_data.data) {
                            warn!(session_id = %self.id, error = %e, "Failed to handle data channel message");
                        }
                        // Re-acquire lock to return properly, but we don't need it
                        return Ok(PollResult::Event(Box::new(event)));
                    }
                    Event::ChannelClose(channel_id) => {
                        info!(session_id = %self.id, ?channel_id, "Data channel closed");
                        let mut data_channel = self.data_channel_id.write();
                        if *data_channel == Some(*channel_id) {
                            *data_channel = None;
                        }
                    }
                    Event::EgressBitrateEstimate(bwe_kind) => {
                        // Extract bitrate from BWE event
                        let bitrate_kbps = match &bwe_kind {
                            str0m::bwe::BweKind::Twcc(bitrate) => bitrate.as_u64() / 1000,
                            str0m::bwe::BweKind::Remb(_, bitrate) => bitrate.as_u64() / 1000,
                            _ => return Ok(PollResult::Event(Box::new(event))),
                        };
                        
                        debug!(
                            session_id = %self.id,
                            bitrate_kbps = bitrate_kbps,
                            "Received bandwidth estimate"
                        );
                        
                        // Update layer state with bandwidth info (may trigger layer switch)
                        drop(rtc); // Release lock before calling method
                        if let Some(notification) = self.update_bandwidth_estimate(bitrate_kbps as u32) {
                            // Request stream switch via internal channel (to forwarding loop)
                            self.request_stream_switch(notification.stream_type);
                            // Notify client about layer change
                            let _ = self.notify_layer_change(&notification);
                        }
                        return Ok(PollResult::Event(Box::new(event)));
                    }
                    _ => {}
                }
                Ok(PollResult::Event(Box::new(event)))
            }
        }
    }

    /// Handle incoming data channel message
    fn handle_data_channel_message(&self, data: &[u8]) -> Result<(), WebRtcError> {
        // Parse JSON
        let command: DataChannelCommand = serde_json::from_slice(data)
            .map_err(|e| WebRtcError::DataChannel(format!("Invalid JSON: {}", e)))?;

        debug!(session_id = %self.id, ?command, "Received data channel command");

        // Forward command to the playback handler
        self.command_tx
            .send(command)
            .map_err(|e| WebRtcError::DataChannel(format!("Failed to send command: {}", e)))?;

        Ok(())
    }

    /// Send a message over the data channel
    pub fn send_data_channel_message(&self, message: &DataChannelMessage) -> Result<(), WebRtcError> {
        let channel_id = self.data_channel_id.read();
        let Some(cid) = *channel_id else {
            return Err(WebRtcError::DataChannel("No data channel open".to_string()));
        };
        drop(channel_id);

        let json = serde_json::to_vec(message)
            .map_err(|e| WebRtcError::DataChannel(format!("Failed to serialize: {}", e)))?;

        let mut rtc = self.rtc.write();
        let Some(mut channel) = rtc.channel(cid) else {
            return Err(WebRtcError::DataChannel("Channel not found".to_string()));
        };

        channel
            .write(true, &json)
            .map_err(|e| WebRtcError::DataChannel(format!("Failed to write: {}", e)))?;

        *self.last_activity.write() = Instant::now();
        Ok(())
    }

    /// Send current position update to client
    pub fn send_position_update(&self) -> Result<(), WebRtcError> {
        let state = self.playback_state.read();
        let message = DataChannelMessage::Position {
            timestamp: state.current_timestamp.unwrap_or_else(Utc::now),
            pts: state.current_pts,
            mode: state.mode,
            is_playing: state.is_playing,
            speed: state.speed,
        };
        drop(state);
        self.send_data_channel_message(&message)
    }

    /// Update playback state
    pub fn update_playback_state<F>(&self, f: F)
    where
        F: FnOnce(&mut PlaybackState),
    {
        let mut state = self.playback_state.write();
        f(&mut state);
    }

    /// Get current playback state
    pub fn get_playback_state(&self) -> PlaybackState {
        self.playback_state.read().clone()
    }

    /// Check if data channel is open
    pub fn has_data_channel(&self) -> bool {
        self.data_channel_id.read().is_some()
    }

    /// Send a video frame via the frame-level API
    pub fn send_frame(&self, frame: &VideoFrame) -> Result<(), WebRtcError> {
        let mut rtc = self.rtc.write();

        let Some(mid) = *self.video_mid.read() else {
            return Err(WebRtcError::NoMedia);
        };

        // Get the media writer
        let Some(writer) = rtc.writer(mid) else {
            return Err(WebRtcError::NoMedia);
        };

        // Get the first available payload type for the codec
        let Some(params) = writer.payload_params().next() else {
            return Err(WebRtcError::NoMedia);
        };

        let pt = params.pt();

        // Create MediaTime from RTP timestamp (90kHz clock for video)
        let media_time = MediaTime::from_90khz(frame.rtp_timestamp as u64);

        // Write the frame using the frame-level API
        // wallclock: absolute time the media was captured
        // media_time: RTP timestamp in codec time units (90kHz for video)
        // Pass &[u8] - str0m's write() accepts impl Into<Vec<u8>> which &[u8] implements
        writer
            .write(pt, frame.wallclock, media_time, &*frame.data)
            .map_err(|e| WebRtcError::MediaWrite(e.to_string()))?;

        *self.last_activity.write() = Instant::now();
        Ok(())
    }

    // =========================================================================
    // Layer Control Methods
    // =========================================================================

    /// Get current layer state
    pub fn get_layer_state(&self) -> LayerState {
        self.layer_state.read().clone()
    }

    /// Set video layer directly
    pub fn set_layer(&self, layer: VideoLayer) -> Result<LayerChangeNotification, WebRtcError> {
        let mut state = self.layer_state.write();
        let previous_layer = Some(state.active_layer);
        state.active_layer = layer;
        state.preference.preferred_layer = layer;
        state.preference.mode = LayerSelectionMode::Manual;
        state.last_layer_change = Some(Instant::now());
        
        // Determine new stream type
        let new_stream_type = layer.to_stream_type();
        state.stream_type = new_stream_type;

        let notification = LayerChangeNotification {
            active_layer: layer,
            previous_layer,
            reason: LayerChangeReason::ClientRequest,
            stream_type: new_stream_type,
            estimated_bitrate_kbps: Some(layer.typical_bitrate_kbps()),
            width: None,
            height: None,
        };

        Ok(notification)
    }

    /// Set layer preference (mode + hints)
    pub fn set_layer_preference(&self, preference: LayerPreference) -> Result<Option<LayerChangeNotification>, WebRtcError> {
        let mut state = self.layer_state.write();
        let previous_layer = state.active_layer;
        state.preference = preference.clone();
        
        if let Some(ctx) = preference.ui_context {
            state.ui_context = Some(ctx);
        }

        // Check if we need to switch layers
        if let Some(new_layer) = state.should_switch() {
            state.active_layer = new_layer;
            state.stream_type = new_layer.to_stream_type();
            state.last_layer_change = Some(Instant::now());

            return Ok(Some(LayerChangeNotification {
                active_layer: new_layer,
                previous_layer: Some(previous_layer),
                reason: match preference.mode {
                    LayerSelectionMode::Manual => LayerChangeReason::ClientRequest,
                    LayerSelectionMode::Adaptive => LayerChangeReason::BandwidthImproved,
                    LayerSelectionMode::UiContext => LayerChangeReason::UiContextChange,
                },
                stream_type: new_layer.to_stream_type(),
                estimated_bitrate_kbps: Some(new_layer.typical_bitrate_kbps()),
                width: None,
                height: None,
            }));
        }

        Ok(None)
    }

    /// Set UI context for adaptive layer selection
    pub fn set_ui_context(&self, context: UiContext) -> Result<Option<LayerChangeNotification>, WebRtcError> {
        let mut state = self.layer_state.write();
        let previous_layer = state.active_layer;
        state.ui_context = Some(context);
        state.preference.ui_context = Some(context);

        // Check if we should switch based on context
        if state.preference.mode == LayerSelectionMode::UiContext {
            let recommended = context.recommended_layer();
            if recommended != state.active_layer {
                state.active_layer = recommended;
                state.stream_type = recommended.to_stream_type();
                state.last_layer_change = Some(Instant::now());

                return Ok(Some(LayerChangeNotification {
                    active_layer: recommended,
                    previous_layer: Some(previous_layer),
                    reason: LayerChangeReason::UiContextChange,
                    stream_type: recommended.to_stream_type(),
                    estimated_bitrate_kbps: Some(recommended.typical_bitrate_kbps()),
                    width: None,
                    height: None,
                }));
            }
        }

        Ok(None)
    }

    /// Update estimated bandwidth (from RTCP feedback)
    pub fn update_bandwidth_estimate(&self, bandwidth_kbps: u32) -> Option<LayerChangeNotification> {
        let mut state = self.layer_state.write();
        let previous_bw = state.estimated_bandwidth_kbps;
        state.estimated_bandwidth_kbps = Some(bandwidth_kbps);

        // Only auto-adjust in adaptive mode
        if state.preference.mode != LayerSelectionMode::Adaptive {
            return None;
        }

        let previous_layer = state.active_layer;

        // Determine if we need to switch based on bandwidth
        let new_layer = if bandwidth_kbps < 400 {
            VideoLayer::Low
        } else if bandwidth_kbps < 1200 {
            VideoLayer::Medium
        } else {
            VideoLayer::High
        };

        // Only switch if different and enough time has passed
        if new_layer != state.active_layer {
            if let Some(last_change) = state.last_layer_change {
                if last_change.elapsed() < Duration::from_secs(3) {
                    return None; // Don't switch too fast
                }
            }

            state.active_layer = new_layer;
            state.stream_type = new_layer.to_stream_type();
            state.last_layer_change = Some(Instant::now());

            let reason = if bandwidth_kbps < previous_bw.unwrap_or(u32::MAX) {
                LayerChangeReason::BandwidthLow
            } else {
                LayerChangeReason::BandwidthImproved
            };

            return Some(LayerChangeNotification {
                active_layer: new_layer,
                previous_layer: Some(previous_layer),
                reason,
                stream_type: new_layer.to_stream_type(),
                estimated_bitrate_kbps: Some(bandwidth_kbps),
                width: None,
                height: None,
            });
        }

        None
    }

    /// Send layer change notification to client
    pub fn notify_layer_change(&self, notification: &LayerChangeNotification) -> Result<(), WebRtcError> {
        let message = DataChannelMessage::LayerChanged(notification.clone());
        self.send_data_channel_message(&message)
    }

    /// Send layer info to client
    pub fn send_layer_info(&self) -> Result<(), WebRtcError> {
        let state = self.layer_state.read();
        let message = DataChannelMessage::LayerInfo {
            active_layer: state.active_layer,
            mode: state.preference.mode,
            available_layers: state.available_layers.clone(),
            stream_type: state.stream_type,
            estimated_bitrate_kbps: state.estimated_bandwidth_kbps,
        };
        drop(state);
        self.send_data_channel_message(&message)
    }

    /// Get current stream type based on layer
    pub fn get_effective_stream_type(&self) -> StreamType {
        self.layer_state.read().stream_type
    }

    // =========================================================================
    // Session Management
    // =========================================================================

    /// Check if session has timed out
    pub fn is_timed_out(&self, timeout: Duration) -> bool {
        self.last_activity.read().elapsed() > timeout
    }

    /// Close the session
    pub fn close(&self) {
        *self.state.write() = SessionState::Closed;
        let mut rtc = self.rtc.write();
        rtc.disconnect();
    }
    
    /// Get the local ICE ufrag for this session (used for demuxing)
    pub fn local_ice_ufrag(&self) -> String {
        let mut rtc = self.rtc.write();
        rtc.direct_api().local_ice_credentials().ufrag.clone()
    }
    
    /// Check if this session accepts the given input
    /// Used for demultiplexing packets to the correct session
    pub fn accepts(&self, input: &Input) -> bool {
        let rtc = self.rtc.read();
        rtc.accepts(input)
    }
}

// =============================================================================
// Multi-Replay Session - Single WebRTC session with multiple camera tracks
// =============================================================================

use crate::api::{MultiReplayCommand, MultiReplayMessage};
use crate::replay_coordinator::{ReplayCoordinator, ReplayCoordinatorConfig};

/// Multi-camera replay session with synchronized playback
/// 
/// This creates a SINGLE WebRTC session with up to 9 video tracks (one per camera).
/// The ReplayCoordinator manages a shared playhead for synchronized playback.
pub struct MultiReplaySession {
    /// Session ID
    pub id: Uuid,
    /// Camera IDs in order (index = track index)
    pub camera_ids: Vec<Uuid>,
    /// Session state
    state: RwLock<SessionState>,
    /// str0m RTC instance
    rtc: RwLock<Rtc>,
    /// Video media IDs for each camera track (index = camera index)
    video_mids: RwLock<Vec<Option<Mid>>>,
    /// Data channel ID for commands
    data_channel_id: RwLock<Option<ChannelId>>,
    /// Replay coordinator for synchronized playback
    coordinator: Arc<ReplayCoordinator>,
    /// Created timestamp
    pub created_at: Instant,
    /// Last activity timestamp
    last_activity: RwLock<Instant>,
    /// Channel for receiving commands from data channel
    command_rx: RwLock<Option<mpsc::UnboundedReceiver<MultiReplayCommand>>>,
    /// Channel for sending commands (used by data channel handler)
    command_tx: mpsc::UnboundedSender<MultiReplayCommand>,
}

impl MultiReplaySession {
    /// Create a new multi-replay session
    pub fn new(
        camera_ids: Vec<Uuid>,
        coordinator_config: ReplayCoordinatorConfig,
        webrtc_config: &WebRtcConfig,
    ) -> Result<Self, WebRtcError> {
        // Set up crypto provider
        str0m_rust_crypto::default_provider().install_process_default();

        // Build RTC instance with BWE disabled for replay (we control the rate)
        let rtc = Rtc::builder()
            .set_ice_lite(webrtc_config.ice_lite)
            .build(Instant::now());

        // Create command channel
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        
        // Create channel for coordinator->client messages
        let (client_tx, _client_rx) = mpsc::unbounded_channel();
        
        // Create coordinator
        let coordinator = ReplayCoordinator::new(coordinator_config, client_tx);
        
        info!(
            session_id = %coordinator.session_id,
            camera_count = camera_ids.len(),
            "Created multi-replay WebRTC session"
        );

        Ok(Self {
            id: coordinator.session_id,
            camera_ids: camera_ids.clone(),
            state: RwLock::new(SessionState::New),
            rtc: RwLock::new(rtc),
            video_mids: RwLock::new(vec![None; camera_ids.len()]),
            data_channel_id: RwLock::new(None),
            coordinator,
            created_at: Instant::now(),
            last_activity: RwLock::new(Instant::now()),
            command_rx: RwLock::new(Some(command_rx)),
            command_tx,
        })
    }

    /// Get the replay coordinator
    pub fn coordinator(&self) -> Arc<ReplayCoordinator> {
        Arc::clone(&self.coordinator)
    }

    /// Take command receiver (can only be called once)
    pub fn take_command_rx(&self) -> Option<mpsc::UnboundedReceiver<MultiReplayCommand>> {
        self.command_rx.write().take()
    }

    /// Get current state
    pub fn state(&self) -> SessionState {
        *self.state.read()
    }

    /// Add local ICE candidate
    pub fn add_local_candidate(&self, addr: SocketAddr) -> Result<(), WebRtcError> {
        let mut rtc = self.rtc.write();
        let candidate = Candidate::host(addr, Protocol::Udp)
            .map_err(|e| WebRtcError::IceCandidate(e.to_string()))?;
        rtc.add_local_candidate(candidate);
        Ok(())
    }

    /// Process SDP offer and generate answer
    /// 
    /// The offer should contain N video tracks (one per camera) where each client
    /// has created a receive-only transceiver for each camera.
    pub fn process_offer(&self, offer_sdp: &str) -> Result<String, WebRtcError> {
        let mut rtc = self.rtc.write();

        let offer = SdpOffer::from_sdp_string(offer_sdp)
            .map_err(|e| WebRtcError::SdpParse(e.to_string()))?;

        let answer = rtc
            .sdp_api()
            .accept_offer(offer)
            .map_err(|e| WebRtcError::SdpApply(e.to_string()))?;

        *self.state.write() = SessionState::Negotiating;
        *self.last_activity.write() = Instant::now();

        Ok(answer.to_sdp_string())
    }

    /// Add remote ICE candidate
    pub fn add_ice_candidate(&self, candidate_str: &str) -> Result<(), WebRtcError> {
        let mut rtc = self.rtc.write();
        let candidate = Candidate::from_sdp_string(candidate_str)
            .map_err(|e| WebRtcError::IceCandidate(e.to_string()))?;
        rtc.add_remote_candidate(candidate);
        *self.last_activity.write() = Instant::now();
        Ok(())
    }

    /// Handle receive for incoming packets
    pub fn handle_receive(
        &self,
        now: Instant,
        source: SocketAddr,
        destination: SocketAddr,
        data: &[u8],
    ) -> Result<(), WebRtcError> {
        let mut rtc = self.rtc.write();
        let receive = Receive {
            proto: Protocol::Udp,
            source,
            destination,
            contents: data
                .try_into()
                .map_err(|_| WebRtcError::Input("Invalid packet data".into()))?,
        };
        rtc.handle_input(Input::Receive(now, receive))
            .map_err(|e| WebRtcError::Input(e.to_string()))?;
        *self.last_activity.write() = Instant::now();
        Ok(())
    }

    /// Poll the session
    pub fn poll(&self, now: Instant) -> Result<PollResult, WebRtcError> {
        let mut rtc = self.rtc.write();
        rtc.handle_input(Input::Timeout(now))
            .map_err(|e| WebRtcError::Input(e.to_string()))?;

        match rtc.poll_output().map_err(|e| WebRtcError::Input(e.to_string()))? {
            Output::Timeout(timeout) => Ok(PollResult::Timeout(timeout)),
            Output::Transmit(transmit) => Ok(PollResult::Transmit(NetworkOutput {
                destination: transmit.destination,
                contents: transmit.contents.to_vec(),
            })),
            Output::Event(event) => {
                match &event {
                    Event::IceConnectionStateChange(state) => {
                        info!(session_id = %self.id, ?state, "Multi-replay ICE state changed");
                        match state {
                            IceConnectionState::Connected => {
                                *self.state.write() = SessionState::Active;
                            }
                            IceConnectionState::Disconnected => {
                                *self.state.write() = SessionState::Closing;
                            }
                            _ => {}
                        }
                    }
                    Event::MediaAdded(media) => {
                        if media.kind == MediaKind::Video {
                            // Map media to camera track by order
                            let mut mids = self.video_mids.write();
                            let assigned_count = mids.iter().filter(|m| m.is_some()).count();
                            if assigned_count < mids.len() {
                                mids[assigned_count] = Some(media.mid);
                                debug!(
                                    session_id = %self.id,
                                    track_index = assigned_count,
                                    mid = ?media.mid,
                                    "Assigned video mid to camera track"
                                );
                            }
                        }
                    }
                    Event::ChannelOpen(channel_id, label) => {
                        info!(session_id = %self.id, ?channel_id, ?label, "Data channel opened");
                        if label == "control" {
                            *self.data_channel_id.write() = Some(*channel_id);
                        }
                    }
                    Event::ChannelData(channel_data) => {
                        drop(rtc);
                        if let Err(e) = self.handle_data_channel_message(&channel_data.data) {
                            warn!(session_id = %self.id, error = %e, "Failed to handle data channel message");
                        }
                        return Ok(PollResult::Event(Box::new(event)));
                    }
                    Event::ChannelClose(channel_id) => {
                        let mut dc = self.data_channel_id.write();
                        if *dc == Some(*channel_id) {
                            *dc = None;
                        }
                    }
                    _ => {}
                }
                Ok(PollResult::Event(Box::new(event)))
            }
        }
    }

    /// Handle data channel message (multi-replay commands)
    fn handle_data_channel_message(&self, data: &[u8]) -> Result<(), WebRtcError> {
        let command: MultiReplayCommand = serde_json::from_slice(data)
            .map_err(|e| WebRtcError::DataChannel(format!("Invalid JSON: {}", e)))?;

        debug!(session_id = %self.id, ?command, "Received multi-replay command");

        self.command_tx
            .send(command)
            .map_err(|e| WebRtcError::DataChannel(format!("Failed to send command: {}", e)))?;

        Ok(())
    }

    /// Send message over data channel
    pub fn send_data_channel_message(&self, message: &MultiReplayMessage) -> Result<(), WebRtcError> {
        let channel_id = self.data_channel_id.read();
        let Some(cid) = *channel_id else {
            return Err(WebRtcError::DataChannel("No data channel open".to_string()));
        };
        drop(channel_id);

        let json = serde_json::to_vec(message)
            .map_err(|e| WebRtcError::DataChannel(format!("Failed to serialize: {}", e)))?;

        let mut rtc = self.rtc.write();
        let Some(mut channel) = rtc.channel(cid) else {
            return Err(WebRtcError::DataChannel("Channel not found".to_string()));
        };

        channel
            .write(true, &json)
            .map_err(|e| WebRtcError::DataChannel(format!("Failed to write: {}", e)))?;

        *self.last_activity.write() = Instant::now();
        Ok(())
    }

    /// Send video frame to a specific camera track
    pub fn send_frame_to_track(&self, track_index: usize, frame: &VideoFrame) -> Result<(), WebRtcError> {
        let mids = self.video_mids.read();
        let Some(Some(mid)) = mids.get(track_index) else {
            return Err(WebRtcError::NoMedia);
        };
        let mid = *mid;
        drop(mids);

        let mut rtc = self.rtc.write();
        let Some(writer) = rtc.writer(mid) else {
            return Err(WebRtcError::NoMedia);
        };

        let Some(params) = writer.payload_params().next() else {
            return Err(WebRtcError::NoMedia);
        };

        let pt = params.pt();
        let media_time = MediaTime::from_90khz(frame.rtp_timestamp as u64);

        writer
            .write(pt, frame.wallclock, media_time, &*frame.data)
            .map_err(|e| WebRtcError::MediaWrite(e.to_string()))?;

        *self.last_activity.write() = Instant::now();
        Ok(())
    }

    /// Get track index for camera
    pub fn track_index_for_camera(&self, camera_id: Uuid) -> Option<usize> {
        self.camera_ids.iter().position(|id| *id == camera_id)
    }

    /// Get local ICE ufrag
    pub fn local_ice_ufrag(&self) -> String {
        let mut rtc = self.rtc.write();
        rtc.direct_api().local_ice_credentials().ufrag.clone()
    }

    /// Check if session accepts input
    pub fn accepts(&self, input: &Input) -> bool {
        let rtc = self.rtc.read();
        rtc.accepts(input)
    }

    /// Check if timed out
    pub fn is_timed_out(&self, timeout: Duration) -> bool {
        self.last_activity.read().elapsed() > timeout
    }

    /// Close the session
    pub fn close(&self) {
        *self.state.write() = SessionState::Closed;
        self.coordinator.shutdown();
        let mut rtc = self.rtc.write();
        rtc.disconnect();
    }
}

// =============================================================================
// API Types for Multi-Replay WebRTC
// =============================================================================

/// Request to create a multi-replay WebRTC session
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiReplayWebRtcRequest {
    /// Camera IDs to include
    pub camera_ids: Vec<Uuid>,
    /// SDP offer from client
    pub sdp_offer: String,
    /// Mode (should be "replay")
    pub mode: String,
    /// Start time for replay
    pub start_time: chrono::DateTime<Utc>,
    /// End time for replay
    pub end_time: chrono::DateTime<Utc>,
}

/// Response for multi-replay WebRTC session creation
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiReplayWebRtcResponse {
    /// Whether creation succeeded
    pub success: bool,
    /// Error message if failed
    pub error: Option<String>,
    /// Session ID
    pub session_id: Option<Uuid>,
    /// SDP answer
    pub sdp_answer: Option<String>,
}

/// Result from polling a session
#[derive(Debug)]
pub enum PollResult {
    /// Timeout - poll again at this instant
    Timeout(Instant),
    /// Data to transmit over network
    Transmit(NetworkOutput),
    /// An event occurred (boxed to reduce enum size)
    Event(Box<Event>),
}

/// WebRTC session manager with fast source address routing
pub struct WebRtcManager {
    config: WebRtcConfig,
    sessions: RwLock<HashMap<Uuid, Arc<WebRtcSession>>>,
    /// Sessions by camera for quick lookup
    camera_sessions: RwLock<HashMap<Uuid, Vec<Uuid>>>,
    /// Multi-replay sessions
    multi_replay_sessions: RwLock<HashMap<Uuid, Arc<MultiReplaySession>>>,
    /// Fast cache: source address → session ID
    /// After first STUN packet identifies a session, subsequent packets 
    /// from the same source can be routed directly without ufrag parsing
    source_addr_cache: RwLock<FastHashMap<SocketAddr, Uuid>>,
}

impl WebRtcManager {
    /// Create a new WebRTC manager
    pub fn new(config: WebRtcConfig) -> Self {
        Self {
            config,
            sessions: RwLock::new(HashMap::new()),
            camera_sessions: RwLock::new(HashMap::new()),
            multi_replay_sessions: RwLock::new(HashMap::new()),
            source_addr_cache: RwLock::new(FastHashMap::new()),
        }
    }

    /// Create a new session for a camera
    pub fn create_session(&self, camera_id: Uuid, stream_type: &str) -> Result<Uuid, WebRtcError> {
        // Check session limit
        let camera_sessions = self.camera_sessions.read();
        if let Some(sessions) = camera_sessions.get(&camera_id)
            && sessions.len() >= self.config.max_sessions_per_camera
        {
            return Err(WebRtcError::TooManySessions);
        }
        drop(camera_sessions);

        // Create session
        let session = WebRtcSession::new(camera_id, stream_type.to_string(), &self.config)?;

        let session_id = session.id;
        let session = Arc::new(session);

        // Store session
        self.sessions.write().insert(session_id, session.clone());
        self.camera_sessions
            .write()
            .entry(camera_id)
            .or_default()
            .push(session_id);

        info!(
            session_id = %session_id,
            camera_id = %camera_id,
            stream_type = %stream_type,
            "Created WebRTC session"
        );

        Ok(session_id)
    }

    /// Get a session by ID
    pub fn get_session(&self, session_id: Uuid) -> Option<Arc<WebRtcSession>> {
        self.sessions.read().get(&session_id).cloned()
    }

    /// Remove a session
    pub fn remove_session(&self, session_id: Uuid) {
        if let Some(session) = self.sessions.write().remove(&session_id) {
            session.close();

            // Remove from camera sessions
            let mut camera_sessions = self.camera_sessions.write();
            if let Some(sessions) = camera_sessions.get_mut(&session.camera_id) {
                sessions.retain(|id| *id != session_id);
            }
            
            // Remove from source address cache
            self.source_addr_cache
                .write()
                .retain(|_, sid| *sid != session_id);

            info!(session_id = %session_id, "Removed WebRTC session");
        }
    }

    /// Get all sessions for a camera
    pub fn get_camera_sessions(&self, camera_id: Uuid) -> Vec<Arc<WebRtcSession>> {
        let sessions = self.sessions.read();
        let camera_sessions = self.camera_sessions.read();

        camera_sessions
            .get(&camera_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| sessions.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Broadcast a frame to all sessions for a camera
    pub fn broadcast_frame(&self, camera_id: Uuid, frame: &VideoFrame) {
        for session in self.get_camera_sessions(camera_id) {
            if session.state() == SessionState::Active
                && let Err(e) = session.send_frame(frame)
            {
                warn!(
                    session_id = %session.id,
                    error = %e,
                    "Failed to send frame to WebRTC session"
                );
            }
        }
    }

    /// Clean up timed out sessions
    pub fn cleanup_timed_out(&self) {
        let timeout = Duration::from_secs(self.config.session_timeout_secs);
        let sessions: Vec<Uuid> = self
            .sessions
            .read()
            .iter()
            .filter(|(_, s)| s.is_timed_out(timeout))
            .map(|(id, _)| *id)
            .collect();

        for session_id in sessions {
            warn!(session_id = %session_id, "Removing timed out WebRTC session");
            self.remove_session(session_id);
        }
    }

    /// Get session count
    pub fn session_count(&self) -> usize {
        self.sessions.read().len()
    }

    /// List all sessions
    pub fn list_sessions(&self) -> Vec<(Uuid, Uuid, String, SessionState)> {
        self.sessions
            .read()
            .values()
            .map(|s| (s.id, s.camera_id, s.stream_type.clone(), s.state()))
            .collect()
    }
    
    /// Find a session by its local ICE ufrag
    /// Used for demultiplexing STUN packets to the correct session
    pub fn find_session_by_ufrag(&self, ufrag: &str) -> Option<Arc<WebRtcSession>> {
        self.sessions
            .read()
            .values()
            .find(|s| s.local_ice_ufrag() == ufrag)
            .cloned()
    }
    
    /// Find the session that accepts this input
    /// Tries each session's accepts() method (which checks ufrag for STUN, source addr for RTP/RTCP)
    pub fn find_accepting_session(&self, input: &Input) -> Option<Arc<WebRtcSession>> {
        self.sessions
            .read()
            .values()
            .find(|s| s.accepts(input))
            .cloned()
    }
    
    /// Route an incoming packet to the correct session
    /// 
    /// Demuxing strategy (fast path first):
    /// 1. Check source address cache (O(1) lookup via halfbrown)
    /// 2. If STUN packet: extract local ufrag, find session, cache source addr
    /// 3. Fallback: iterate sessions with Rtc::accepts()
    pub fn route_packet(&self, source: SocketAddr, destination: SocketAddr, data: &[u8]) -> Option<Arc<WebRtcSession>> {
        // Fast path: check source address cache first
        // This avoids STUN parsing for subsequent packets from known sources
        if let Some(&session_id) = self.source_addr_cache.read().get(&source) {
            if let Some(session) = self.sessions.read().get(&session_id).cloned() {
                trace!("Cache hit for {} -> session {}", source, session_id);
                return Some(session);
            }
            // Session was removed, clean up stale cache entry
            self.source_addr_cache.write().remove(&source);
        }
        
        // Slow path: STUN-based demuxing for initial packets
        if is_stun_packet(data)
            && let Some(local_ufrag) = extract_local_ufrag_from_stun(data)
        {
            trace!("STUN packet from {} with ufrag: {}", source, local_ufrag);
            if let Some(session) = self.find_session_by_ufrag(&local_ufrag) {
                // Cache this source address for fast routing of subsequent packets
                self.source_addr_cache.write().insert(source, session.id);
                debug!("Cached source {} -> session {}", source, session.id);
                return Some(session);
            }
        }
        
        // Fallback: Input::Receive based demuxing
        // This works for RTP/RTCP after ICE has established the connection
        let recv = match DatagramRecv::try_from(data) {
            Ok(r) => r,
            Err(e) => {
                trace!("Failed to parse datagram from {}: {}", source, e);
                return None;
            }
        };
        
        let receive = Receive {
            proto: Protocol::Udp,
            source,
            destination,
            contents: recv,
        };
        
        let input = Input::Receive(Instant::now(), receive);
        if let Some(session) = self.find_accepting_session(&input) {
            // Cache this source address too
            self.source_addr_cache.write().insert(source, session.id);
            debug!("Cached source {} -> session {} (via accepts)", source, session.id);
            return Some(session);
        }
        
        None
    }

    // =========================================================================
    // Multi-Replay Session Management  
    // =========================================================================
    
    /// Create a multi-replay session for multiple cameras
    pub fn create_multi_replay_session(
        &self,
        camera_ids: Vec<Uuid>,
        start_time: chrono::DateTime<Utc>,
        end_time: chrono::DateTime<Utc>,
    ) -> Result<Arc<MultiReplaySession>, WebRtcError> {
        let config = ReplayCoordinatorConfig {
            camera_ids: camera_ids.clone(),
            start_time,
            end_time,
            initial_position: start_time,
            auto_play: false,
            initial_speed: 1.0,
        };
        
        let session = MultiReplaySession::new(camera_ids, config, &self.config)?;
        let session = Arc::new(session);
        let session_id = session.id;
        
        self.multi_replay_sessions.write().insert(session_id, session.clone());
        
        info!(
            session_id = %session_id,
            "Created multi-replay session"
        );
        
        Ok(session)
    }
    
    /// Get a multi-replay session by ID
    pub fn get_multi_replay_session(&self, session_id: Uuid) -> Option<Arc<MultiReplaySession>> {
        self.multi_replay_sessions.read().get(&session_id).cloned()
    }
    
    /// Remove a multi-replay session
    pub fn remove_multi_replay_session(&self, session_id: Uuid) {
        if let Some(session) = self.multi_replay_sessions.write().remove(&session_id) {
            session.close();
            
            // Clean up cache entries
            self.source_addr_cache
                .write()
                .retain(|_, sid| *sid != session_id);
            
            info!(session_id = %session_id, "Removed multi-replay session");
        }
    }
    
    /// Find multi-replay session by ufrag
    pub fn find_multi_replay_session_by_ufrag(&self, ufrag: &str) -> Option<Arc<MultiReplaySession>> {
        self.multi_replay_sessions
            .read()
            .values()
            .find(|s| s.local_ice_ufrag() == ufrag)
            .cloned()
    }
    
    /// Route packet to multi-replay session
    pub fn route_to_multi_replay(&self, source: SocketAddr, data: &[u8]) -> Option<Arc<MultiReplaySession>> {
        // Check cache first
        if let Some(&session_id) = self.source_addr_cache.read().get(&source) {
            if let Some(session) = self.multi_replay_sessions.read().get(&session_id).cloned() {
                return Some(session);
            }
        }
        
        // Try STUN-based lookup
        if is_stun_packet(data)
            && let Some(local_ufrag) = extract_local_ufrag_from_stun(data)
        {
            if let Some(session) = self.find_multi_replay_session_by_ufrag(&local_ufrag) {
                self.source_addr_cache.write().insert(source, session.id);
                return Some(session);
            }
        }
        
        // Try accepts-based lookup
        for session in self.multi_replay_sessions.read().values() {
            let recv = match DatagramRecv::try_from(data) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let input = Input::Receive(
                Instant::now(),
                Receive {
                    proto: Protocol::Udp,
                    source,
                    destination: "0.0.0.0:0".parse().unwrap(), // Ignored for accepts
                    contents: recv,
                },
            );
            if session.accepts(&input) {
                self.source_addr_cache.write().insert(source, session.id);
                return Some(session.clone());
            }
        }
        
        None
    }
    
    /// Get the number of cached source addresses
    pub fn cache_size(&self) -> usize {
        self.source_addr_cache.read().len()
    }
    
    /// Clear the source address cache
    pub fn clear_cache(&self) {
        self.source_addr_cache.write().clear();
    }
    
    /// Get all sessions as a list (for iteration in the receive loop)
    pub fn all_sessions(&self) -> Vec<Arc<WebRtcSession>> {
        self.sessions.read().values().cloned().collect()
    }
}

impl Default for WebRtcManager {
    fn default() -> Self {
        Self::new(WebRtcConfig::default())
    }
}

// ============================================================================
// UDP Receive Task
// ============================================================================

/// Runs the UDP receive loop, demultiplexing packets to the correct session
/// 
/// This task:
/// 1. Receives UDP packets from the shared socket
/// 2. Parses STUN to extract ufrag for demuxing
/// 3. Routes packets to the correct session
/// 4. Sessions handle the packet and generate outgoing transmits
pub async fn run_udp_receive_loop(
    transport: Arc<UdpTransport>,
    manager: Arc<WebRtcManager>,
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
) {
    let mut buf = vec![0u8; 65536]; // Max UDP packet size
    let local_addr = transport.local_addr();
    
    info!("Starting WebRTC UDP receive loop on {}", local_addr);
    
    loop {
        tokio::select! {
            // Check for shutdown signal
            _ = shutdown.recv() => {
                info!("WebRTC UDP receive loop shutting down");
                break;
            }
            
            // Receive UDP packet
            result = transport.recv_from(&mut buf) => {
                match result {
                    Ok((len, source)) => {
                        let data = &buf[..len];
                        let now = Instant::now();
                        
                        trace!("Received {} bytes from {}", len, source);
                        
                        // Route packet to correct session
                        if let Some(session) = manager.route_packet(source, local_addr, data) {
                            // Handle the packet in the session
                            if let Err(e) = session.handle_receive(now, source, local_addr, data) {
                                warn!(
                                    session_id = %session.id,
                                    error = %e,
                                    "Error handling packet"
                                );
                            }
                            
                            // Poll the session for outgoing packets
                            loop {
                                match session.poll(now) {
                                    Ok(PollResult::Transmit(output)) => {
                                        if let Err(e) = transport.send_to(&output.contents, output.destination).await {
                                            warn!(
                                                session_id = %session.id,
                                                dest = %output.destination,
                                                error = %e,
                                                "Failed to send UDP packet"
                                            );
                                        }
                                    }
                                    Ok(PollResult::Timeout(_)) => break,
                                    Ok(PollResult::Event(event)) => {
                                        debug!(session_id = %session.id, ?event, "Session event");
                                    }
                                    Err(e) => {
                                        warn!(
                                            session_id = %session.id,
                                            error = %e,
                                            "Error polling session"
                                        );
                                        break;
                                    }
                                }
                            }
                        } else {
                            trace!("No session found for packet from {}", source);
                        }
                    }
                    Err(e) => {
                        error!("UDP receive error: {}", e);
                    }
                }
            }
        }
    }
}

/// Configuration for the WebRTC server
#[derive(Debug, Clone)]
pub struct WebRtcServerConfig {
    /// Address to bind the UDP socket to
    pub bind_addr: SocketAddr,
    /// WebRTC session configuration
    pub session_config: WebRtcConfig,
}

impl Default for WebRtcServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:0".parse().unwrap(),
            session_config: WebRtcConfig::default(),
        }
    }
}

/// WebRTC server with shared UDP transport
pub struct WebRtcServer {
    /// Shared UDP transport
    pub transport: Arc<UdpTransport>,
    /// Session manager
    pub manager: Arc<WebRtcManager>,
    /// Shutdown signal sender
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
}

impl WebRtcServer {
    /// Create and start a new WebRTC server
    pub async fn start(config: WebRtcServerConfig) -> Result<Self, WebRtcError> {
        // Initialize crypto provider
        str0m_rust_crypto::default_provider().install_process_default();
        
        // Create UDP transport
        let transport = Arc::new(UdpTransport::bind(config.bind_addr).await?);
        
        // Create session manager
        let manager = Arc::new(WebRtcManager::new(config.session_config));
        
        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
        
        // Spawn the receive loop
        let transport_clone = Arc::clone(&transport);
        let manager_clone = Arc::clone(&manager);
        tokio::spawn(async move {
            run_udp_receive_loop(transport_clone, manager_clone, shutdown_rx).await;
        });
        
        info!("WebRTC server started on {}", transport.local_addr());
        
        Ok(Self {
            transport,
            manager,
            shutdown_tx,
        })
    }
    
    /// Get the local address of the UDP socket
    pub fn local_addr(&self) -> SocketAddr {
        self.transport.local_addr()
    }
    
    /// Create a new session for a camera
    /// Automatically adds the local candidate
    pub fn create_session(&self, camera_id: Uuid, stream_type: &str) -> Result<Uuid, WebRtcError> {
        let session_id = self.manager.create_session(camera_id, stream_type)?;
        
        // Add local candidate (our UDP socket address)
        if let Some(session) = self.manager.get_session(session_id) {
            session.add_local_candidate(self.transport.local_addr())?;
        }
        
        Ok(session_id)
    }
    
    /// Shutdown the server
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}

/// WebRTC errors
#[derive(Debug, thiserror::Error)]
pub enum WebRtcError {
    #[error("SDP parse error: {0}")]
    SdpParse(String),

    #[error("SDP apply error: {0}")]
    SdpApply(String),

    #[error("SDP create error: {0}")]
    SdpCreate(String),

    #[error("ICE candidate error: {0}")]
    IceCandidate(String),

    #[error("Input error: {0}")]
    Input(String),

    #[error("Media write error: {0}")]
    MediaWrite(String),

    #[error("No media track configured")]
    NoMedia,

    #[error("Too many sessions for this camera")]
    TooManySessions,

    #[error("Session not found")]
    SessionNotFound,

    #[error("RTC error: {0}")]
    Rtc(#[from] RtcError),
    
    #[error("IO error: {0}")]
    Io(String),

    #[error("Data channel error: {0}")]
    DataChannel(String),
}

// =============================================================================
// Forwarding Pipeline - Stream Manager → WebRTC
// =============================================================================

/// Convert stream_manager::VideoCodec to webrtc::VideoCodec
impl From<StreamCodec> for VideoCodec {
    fn from(codec: StreamCodec) -> Self {
        match codec {
            StreamCodec::H264 => VideoCodec::H264,
            StreamCodec::H265 => VideoCodec::H265,
            StreamCodec::Unknown => VideoCodec::H264, // Default to H264 for WebRTC compatibility
        }
    }
}

/// Convert StreamChunk to VideoFrame for WebRTC transmission
impl From<StreamChunk> for VideoFrame {
    fn from(chunk: StreamChunk) -> Self {
        VideoFrame {
            data: chunk.data,
            rtp_timestamp: chunk.timestamp,
            wallclock: chunk.received_at,
            is_keyframe: chunk.is_keyframe,
            codec: VideoCodec::H264, // Will be set by forwarder based on stream metadata
        }
    }
}

/// Convert StreamChunk to VideoFrame with explicit codec
pub fn chunk_to_frame(chunk: StreamChunk, codec: StreamCodec) -> VideoFrame {
    VideoFrame {
        data: chunk.data,
        rtp_timestamp: chunk.timestamp,
        wallclock: chunk.received_at,
        is_keyframe: chunk.is_keyframe,
        codec: codec.into(),
    }
}

/// Forwarding task that bridges Stream Manager to WebRTC sessions
/// 
/// This runs as a background task per WebRTC session, consuming frames
/// from the stream manager and forwarding them to the WebRTC peer.
pub async fn run_forwarding_task(
    session: Arc<WebRtcSession>,
    stream_session: Arc<StreamSession>,
    shutdown: tokio::sync::broadcast::Receiver<()>,
) {
    // Subscribe to the stream - get prebuffered frames + live receiver
    let (prebuffered, mut receiver) = stream_session.subscribe();
    let codec = stream_session.metadata().codec;
    
    info!(
        session_id = %session.id,
        camera_id = %session.camera_id,
        prebuffered_frames = prebuffered.len(),
        codec = ?codec,
        "Starting WebRTC forwarding task"
    );
    
    // Send prebuffered frames first for instant playback
    // Only send from the most recent keyframe onwards
    let mut started_sending = false;
    for chunk in prebuffered {
        // Wait for keyframe to start (clean decoder state)
        if !started_sending && !chunk.is_keyframe {
            continue;
        }
        started_sending = true;
        
        let frame = chunk_to_frame(chunk, codec);
        if let Err(e) = session.send_frame(&frame) {
            warn!(
                session_id = %session.id,
                error = %e,
                "Failed to send prebuffered frame"
            );
            // Don't fail completely - continue with live frames
            break;
        }
    }
    
    // Forward live frames
    let mut shutdown = shutdown;
    loop {
        tokio::select! {
            // Shutdown signal
            _ = shutdown.recv() => {
                info!(session_id = %session.id, "Forwarding task shutting down");
                break;
            }
            
            // Receive frame from stream manager
            chunk = receiver.recv() => {
                match chunk {
                    Some(chunk) => {
                        let frame = chunk_to_frame(chunk, codec);
                        if let Err(e) = session.send_frame(&frame) {
                            // Log but don't fail - session might be in negotiating state
                            debug!(
                                session_id = %session.id,
                                error = %e,
                                "Failed to send frame to WebRTC"
                            );
                        }
                    }
                    None => {
                        // Channel closed - stream ended
                        info!(session_id = %session.id, "Stream ended, stopping forwarding");
                        break;
                    }
                }
            }
            
            // Check if session is still active (poll periodically)
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                if session.state() == SessionState::Closing {
                    info!(session_id = %session.id, "Session closing, stopping forwarding");
                    break;
                }
            }
        }
    }
}

/// Start forwarding for a WebRTC session
/// 
/// This creates the forwarding task and returns a handle to stop it.
pub fn start_forwarding(
    session: Arc<WebRtcSession>,
    stream_session: Arc<StreamSession>,
) -> (tokio::task::JoinHandle<()>, tokio::sync::broadcast::Sender<()>) {
    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    
    let handle = tokio::spawn(run_forwarding_task(session, stream_session, shutdown_rx));
    
    (handle, shutdown_tx)
}

/// Enhanced forwarding context for stream switching
pub struct ForwardingContext {
    /// Main stream session (if available)
    pub main_stream: Option<Arc<StreamSession>>,
    /// Sub stream session (if available)
    pub sub_stream: Option<Arc<StreamSession>>,
}

impl ForwardingContext {
    pub fn new() -> Self {
        Self {
            main_stream: None,
            sub_stream: None,
        }
    }
    
    pub fn with_main(mut self, stream: Arc<StreamSession>) -> Self {
        self.main_stream = Some(stream);
        self
    }
    
    pub fn with_sub(mut self, stream: Arc<StreamSession>) -> Self {
        self.sub_stream = Some(stream);
        self
    }
    
    pub fn get_stream(&self, stream_type: StreamType) -> Option<Arc<StreamSession>> {
        match stream_type {
            StreamType::Main => self.main_stream.clone(),
            StreamType::Sub => self.sub_stream.clone(),
        }
    }
}

impl Default for ForwardingContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Start enhanced forwarding with command handling and stream switching
/// 
/// This version:
/// 1. Handles data channel commands (setLayer, setUiContext, etc.)
/// 2. Sends responses back to the client
/// 3. Supports switching between main/sub streams
pub fn start_forwarding_with_commands(
    session: Arc<WebRtcSession>,
    context: ForwardingContext,
) -> (tokio::task::JoinHandle<()>, tokio::sync::broadcast::Sender<()>) {
    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    
    let handle = tokio::spawn(run_forwarding_with_commands(session, context, shutdown_rx));
    
    (handle, shutdown_tx)
}

/// Enhanced forwarding task with command handling
async fn run_forwarding_with_commands(
    session: Arc<WebRtcSession>,
    context: ForwardingContext,
    shutdown: tokio::sync::broadcast::Receiver<()>,
) {
    // Take the command receiver from the session
    let command_rx = session.take_command_rx();
    
    // Take the stream switch receiver (for BWE-triggered switches)
    let stream_switch_rx = session.take_stream_switch_rx();
    
    // Determine initial stream based on layer state
    let layer_state = session.get_layer_state();
    let initial_stream_type = layer_state.stream_type;
    
    // Get initial stream session
    let initial_stream = context.get_stream(initial_stream_type);
    
    if initial_stream.is_none() && context.main_stream.is_none() && context.sub_stream.is_none() {
        warn!(
            session_id = %session.id,
            "No streams available for forwarding"
        );
        return;
    }
    
    // Use available stream, preferring the one matching layer state
    let stream_session = initial_stream
        .or_else(|| context.main_stream.clone())
        .or_else(|| context.sub_stream.clone());
    
    let Some(stream_session) = stream_session else {
        warn!(session_id = %session.id, "No stream session available");
        return;
    };
    
    info!(
        session_id = %session.id,
        camera_id = %session.camera_id,
        initial_stream_type = %initial_stream_type,
        "Starting enhanced forwarding with command handling"
    );
    
    // Run the combined forwarding + command handling loop
    run_combined_forwarding_loop(
        session,
        context,
        stream_session,
        command_rx,
        stream_switch_rx,
        shutdown,
    ).await;
}

/// Combined loop that handles both frame forwarding and commands
async fn run_combined_forwarding_loop(
    session: Arc<WebRtcSession>,
    context: ForwardingContext,
    mut current_stream: Arc<StreamSession>,
    command_rx: Option<mpsc::UnboundedReceiver<DataChannelCommand>>,
    mut stream_switch_rx: Option<mpsc::UnboundedReceiver<StreamType>>,
    shutdown: tokio::sync::broadcast::Receiver<()>,
) {
    // Subscribe to the initial stream
    let (prebuffered, mut receiver) = current_stream.subscribe();
    let mut codec = current_stream.metadata().codec;
    
    info!(
        session_id = %session.id,
        prebuffered_frames = prebuffered.len(),
        codec = ?codec,
        "Subscribed to stream"
    );
    
    // Send prebuffered frames first for instant playback
    let mut started_sending = false;
    for chunk in prebuffered {
        if !started_sending && !chunk.is_keyframe {
            continue;
        }
        started_sending = true;
        
        let frame = chunk_to_frame(chunk, codec);
        if let Err(e) = session.send_frame(&frame) {
            warn!(session_id = %session.id, error = %e, "Failed to send prebuffered frame");
            break;
        }
    }
    
    // Send initial status to client
    if session.has_data_channel() {
        let _ = session.send_layer_info();
    }
    
    // Command receiver (if available)
    let mut command_rx = command_rx;
    let mut shutdown = shutdown;
    
    // Position update interval
    let mut position_interval = tokio::time::interval(Duration::from_millis(250));
    
    // Track if we need to switch streams
    let mut pending_stream_switch: Option<StreamType> = None;
    let mut waiting_for_keyframe = false;
    
    loop {
        tokio::select! {
            // Shutdown signal
            _ = shutdown.recv() => {
                info!(session_id = %session.id, "Forwarding task shutting down");
                break;
            }
            
            // Handle commands from data channel
            cmd = async {
                if let Some(ref mut rx) = command_rx {
                    rx.recv().await
                } else {
                    std::future::pending().await
                }
            } => {
                if let Some(cmd) = cmd {
                    handle_command(&session, &context, cmd, &mut pending_stream_switch).await;
                }
            }
            
            // Handle BWE-triggered stream switches
            stream_switch = async {
                if let Some(ref mut rx) = stream_switch_rx {
                    rx.recv().await
                } else {
                    std::future::pending().await
                }
            } => {
                if let Some(new_type) = stream_switch {
                    info!(
                        session_id = %session.id,
                        new_type = %new_type,
                        "BWE triggered stream switch request"
                    );
                    pending_stream_switch = Some(new_type);
                }
            }
            
            // Receive frame from stream manager
            chunk = receiver.recv() => {
                match chunk {
                    Some(chunk) => {
                        // Check if we need to switch streams (on keyframe)
                        if let Some(new_type) = pending_stream_switch {
                            if chunk.is_keyframe || !waiting_for_keyframe {
                                // Try to switch
                                if let Some(new_stream) = context.get_stream(new_type) {
                                    info!(
                                        session_id = %session.id,
                                        new_type = %new_type,
                                        "Switching streams"
                                    );
                                    
                                    // Update current stream
                                    current_stream = new_stream;
                                    let (new_prebuf, new_rx) = current_stream.subscribe();
                                    receiver = new_rx;
                                    codec = current_stream.metadata().codec;
                                    
                                    // Send prebuffered frames from new stream
                                    for prebuf_chunk in new_prebuf {
                                        if prebuf_chunk.is_keyframe || !waiting_for_keyframe {
                                            let frame = chunk_to_frame(prebuf_chunk, codec);
                                            let _ = session.send_frame(&frame);
                                            waiting_for_keyframe = false;
                                        }
                                    }
                                    
                                    pending_stream_switch = None;
                                    
                                    // Notify client
                                    let _ = session.send_layer_info();
                                    continue;
                                }
                            }
                        }
                        
                        // Forward frame
                        let frame = chunk_to_frame(chunk, codec);
                        if let Err(e) = session.send_frame(&frame) {
                            debug!(session_id = %session.id, error = %e, "Failed to send frame");
                        }
                    }
                    None => {
                        info!(session_id = %session.id, "Stream ended");
                        break;
                    }
                }
            }
            
            // Periodic position update
            _ = position_interval.tick() => {
                if session.has_data_channel() {
                    let _ = session.send_position_update();
                }
            }
            
            // Session health check
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                if session.state() == SessionState::Closing {
                    info!(session_id = %session.id, "Session closing");
                    break;
                }
            }
        }
    }
}

/// Handle a data channel command
async fn handle_command(
    session: &Arc<WebRtcSession>,
    context: &ForwardingContext,
    cmd: DataChannelCommand,
    pending_stream_switch: &mut Option<StreamType>,
) {
    debug!(session_id = %session.id, ?cmd, "Processing command");
    
    match cmd {
        DataChannelCommand::SetLayer { layer } => {
            match session.set_layer(layer) {
                Ok(notification) => {
                    // Schedule stream switch if needed
                    let new_stream_type = layer.to_stream_type();
                    if context.get_stream(new_stream_type).is_some() {
                        *pending_stream_switch = Some(new_stream_type);
                    }
                    
                    // Notify client
                    let _ = session.notify_layer_change(&notification);
                }
                Err(e) => {
                    let _ = session.send_data_channel_message(&DataChannelMessage::Error {
                        code: "LAYER_ERROR".to_string(),
                        message: e.to_string(),
                        command: Some("setLayer".to_string()),
                    });
                }
            }
        }
        
        DataChannelCommand::SetLayerPreference { preference } => {
            match session.set_layer_preference(preference) {
                Ok(Some(notification)) => {
                    // Schedule stream switch if layer changed
                    let new_stream_type = notification.stream_type;
                    if context.get_stream(new_stream_type).is_some() {
                        *pending_stream_switch = Some(new_stream_type);
                    }
                    let _ = session.notify_layer_change(&notification);
                }
                Ok(None) => {
                    // No change needed, just send current info
                    let _ = session.send_layer_info();
                }
                Err(e) => {
                    let _ = session.send_data_channel_message(&DataChannelMessage::Error {
                        code: "PREFERENCE_ERROR".to_string(),
                        message: e.to_string(),
                        command: Some("setLayerPreference".to_string()),
                    });
                }
            }
        }
        
        DataChannelCommand::SetUiContext { context: ui_ctx } => {
            match session.set_ui_context(ui_ctx) {
                Ok(Some(notification)) => {
                    let new_stream_type = notification.stream_type;
                    if context.get_stream(new_stream_type).is_some() {
                        *pending_stream_switch = Some(new_stream_type);
                    }
                    let _ = session.notify_layer_change(&notification);
                }
                Ok(None) => {
                    let _ = session.send_layer_info();
                }
                Err(e) => {
                    let _ = session.send_data_channel_message(&DataChannelMessage::Error {
                        code: "CONTEXT_ERROR".to_string(),
                        message: e.to_string(),
                        command: Some("setUiContext".to_string()),
                    });
                }
            }
        }
        
        DataChannelCommand::SetStream { stream_type } => {
            // Direct stream switch
            if context.get_stream(stream_type).is_some() {
                *pending_stream_switch = Some(stream_type);
                
                // Update layer state to match
                let layer = match stream_type {
                    StreamType::Main => VideoLayer::High,
                    StreamType::Sub => VideoLayer::Low,
                };
                if let Ok(notification) = session.set_layer(layer) {
                    let _ = session.notify_layer_change(&notification);
                }
            } else {
                let _ = session.send_data_channel_message(&DataChannelMessage::Error {
                    code: "STREAM_NOT_AVAILABLE".to_string(),
                    message: format!("{} stream not available", stream_type),
                    command: Some("setStream".to_string()),
                });
            }
        }
        
        DataChannelCommand::GetStatus => {
            let layer_state = session.get_layer_state();
            let playback_state = session.get_playback_state();
            
            let status = crate::api::PlaybackStatus {
                camera_id: session.camera_id,
                stream_type: layer_state.stream_type,
                mode: playback_state.mode,
                is_playing: playback_state.is_playing,
                speed: playback_state.speed,
                current_timestamp: playback_state.current_timestamp,
                codec: None, // Could be filled from stream metadata
                width: None,
                height: None,
                connection_state: format!("{:?}", session.state()),
                buffered_ranges: vec![],
                active_layer: layer_state.active_layer,
                layer_mode: layer_state.preference.mode,
                available_layers: layer_state.available_layers,
            };
            
            let _ = session.send_data_channel_message(&DataChannelMessage::Status(status));
        }
        
        DataChannelCommand::Ping { client_time } => {
            let _ = session.send_data_channel_message(&DataChannelMessage::Pong {
                client_time,
                server_time: Utc::now(),
            });
        }
        
        DataChannelCommand::Play { speed } => {
            session.update_playback_state(|state| {
                state.is_playing = true;
                state.speed = speed;
            });
            // For live mode, playback is always "playing"
            // Could send stateChange notification here
        }
        
        DataChannelCommand::Pause => {
            session.update_playback_state(|state| {
                state.is_playing = false;
            });
        }
        
        DataChannelCommand::SetSpeed { speed } => {
            session.update_playback_state(|state| {
                state.speed = speed;
            });
        }
        
        DataChannelCommand::SetMode { mode } => {
            session.update_playback_state(|state| {
                state.mode = mode;
            });
            let _ = session.send_data_channel_message(&DataChannelMessage::ModeChanged {
                mode,
                previous_mode: None,
            });
        }
        
        // Timeline commands - these would need access to the database
        DataChannelCommand::Seek { timestamp: _, direction: _ } => {
            // For live mode, seek is not supported
            // For replay mode, this would need database access
            let _ = session.send_data_channel_message(&DataChannelMessage::Error {
                code: "NOT_IMPLEMENTED".to_string(),
                message: "Seek not implemented for live mode".to_string(),
                command: Some("seek".to_string()),
            });
        }
        
        DataChannelCommand::Skip { seconds: _ } => {
            let _ = session.send_data_channel_message(&DataChannelMessage::Error {
                code: "NOT_IMPLEMENTED".to_string(),
                message: "Skip not implemented for live mode".to_string(),
                command: Some("skip".to_string()),
            });
        }
        
        DataChannelCommand::GetTimeline { start_time, end_time } => {
            // Would need database access
            let _ = session.send_data_channel_message(&DataChannelMessage::Timeline {
                start_time,
                end_time,
                segments: vec![],
                earliest_available: None,
                latest_available: None,
            });
        }
        
        DataChannelCommand::GetKeyframes { start_time, end_time } => {
            let _ = session.send_data_channel_message(&DataChannelMessage::Keyframes {
                start_time,
                end_time,
                keyframes: vec![],
            });
        }
        
        DataChannelCommand::GetEvents { start_time, end_time, event_types: _ } => {
            let _ = session.send_data_channel_message(&DataChannelMessage::Events {
                start_time,
                end_time,
                events: vec![],
            });
        }
        
        // Focus commands - these need the bandwidth manager
        DataChannelCommand::RequestFocus => {
            // Note: In a full implementation, we'd pass the bandwidth manager here.
            // For now, we just upgrade locally and notify.
            let layer_state = session.get_layer_state();
            
            if layer_state.active_layer == VideoLayer::High {
                // Already focused
                let _ = session.send_data_channel_message(&DataChannelMessage::FocusGranted {
                    camera_id: session.camera_id,
                    layer: VideoLayer::High,
                    bitrate_kbps: VideoLayer::High.typical_bitrate_kbps(),
                });
            } else {
                // Try to upgrade via layer control
                match session.set_layer(VideoLayer::High) {
                    Ok(notification) => {
                        // Schedule stream switch
                        let new_stream_type = notification.stream_type;
                        if context.get_stream(new_stream_type).is_some() {
                            *pending_stream_switch = Some(new_stream_type);
                        }
                        
                        let _ = session.send_data_channel_message(&DataChannelMessage::FocusGranted {
                            camera_id: session.camera_id,
                            layer: VideoLayer::High,
                            bitrate_kbps: notification.estimated_bitrate_kbps.unwrap_or(4000),
                        });
                        let _ = session.notify_layer_change(&notification);
                    }
                    Err(e) => {
                        let _ = session.send_data_channel_message(&DataChannelMessage::FocusDenied {
                            camera_id: session.camera_id,
                            reason: e.to_string(),
                            available_kbps: 0,
                            required_kbps: 4000,
                        });
                    }
                }
            }
        }
        
        DataChannelCommand::ReleaseFocus => {
            // Downgrade to low
            match session.set_layer(VideoLayer::Low) {
                Ok(notification) => {
                    let new_stream_type = notification.stream_type;
                    if context.get_stream(new_stream_type).is_some() {
                        *pending_stream_switch = Some(new_stream_type);
                    }
                    
                    let _ = session.send_data_channel_message(&DataChannelMessage::FocusRevoked {
                        camera_id: session.camera_id,
                        reason: "Released by client".to_string(),
                        new_layer: VideoLayer::Low,
                    });
                    let _ = session.notify_layer_change(&notification);
                }
                Err(e) => {
                    let _ = session.send_data_channel_message(&DataChannelMessage::Error {
                        code: "FOCUS_ERROR".to_string(),
                        message: e.to_string(),
                        command: Some("releaseFocus".to_string()),
                    });
                }
            }
        }
        
        DataChannelCommand::GetBandwidthBudget => {
            // Return a simulated budget based on current state
            let layer_state = session.get_layer_state();
            let is_focused = layer_state.active_layer == VideoLayer::High;
            let used_kbps = layer_state.active_layer.typical_bitrate_kbps();
            let estimated_total = layer_state.estimated_bandwidth_kbps.unwrap_or(5000);
            
            let _ = session.send_data_channel_message(&DataChannelMessage::BandwidthBudget {
                total_kbps: estimated_total,
                used_kbps,
                available_kbps: estimated_total.saturating_sub(used_kbps + 500),
                can_upgrade: !is_focused && estimated_total > 4500,
                recommendation: if is_focused {
                    "focused".to_string()
                } else if estimated_total > 4500 {
                    "upgrade_available".to_string()
                } else {
                    "bandwidth_limited".to_string()
                },
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webrtc_config_default() {
        let config = WebRtcConfig::default();
        assert!(config.ice_lite);
        assert_eq!(config.max_sessions_per_camera, 10);
    }

    #[test]
    fn test_webrtc_manager_creation() {
        let manager = WebRtcManager::default();
        assert_eq!(manager.session_count(), 0);
    }

    #[test]
    fn test_session_creation() {
        let manager = WebRtcManager::default();
        let camera_id = Uuid::new_v4();

        let result = manager.create_session(camera_id, "main");
        assert!(result.is_ok());

        let session_id = result.unwrap();
        assert!(manager.get_session(session_id).is_some());
        assert_eq!(manager.session_count(), 1);
    }

    #[test]
    fn test_session_removal() {
        let manager = WebRtcManager::default();
        let camera_id = Uuid::new_v4();

        let session_id = manager.create_session(camera_id, "main").unwrap();
        assert_eq!(manager.session_count(), 1);

        manager.remove_session(session_id);
        assert_eq!(manager.session_count(), 0);
        assert!(manager.get_session(session_id).is_none());
    }
    
    #[test]
    fn test_is_stun_packet() {
        // STUN binding request magic cookie: 0x2112A442
        // First two bytes are type, STUN messages have first byte < 2
        let stun_like = [0x00, 0x01, 0x00, 0x00, 0x21, 0x12, 0xa4, 0x42,
                        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                        0x00, 0x00, 0x00, 0x00]; // 20 bytes minimum
        assert!(is_stun_packet(&stun_like));
        
        // RTP packets start with byte >= 128
        let rtp_like = [0x80, 0x00]; // First byte >= 128
        assert!(!is_stun_packet(&rtp_like));
        
        // Too short
        let short = [0x00, 0x01];
        assert!(!is_stun_packet(&short));
    }
    
    #[test]
    fn test_session_ufrag() {
        let manager = WebRtcManager::default();
        let camera_id = Uuid::new_v4();
        
        let session_id = manager.create_session(camera_id, "main").unwrap();
        let session = manager.get_session(session_id).unwrap();
        
        // Session should have a non-empty ufrag
        let ufrag = session.local_ice_ufrag();
        assert!(!ufrag.is_empty());
        
        // Find by ufrag should work
        let found = manager.find_session_by_ufrag(&ufrag);
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, session_id);
        
        // Unknown ufrag should not find anything
        let not_found = manager.find_session_by_ufrag("unknown-ufrag");
        assert!(not_found.is_none());
    }
    
    #[tokio::test]
    async fn test_udp_transport_creation() {
        let transport = UdpTransport::bind("127.0.0.1:0".parse().unwrap()).await;
        assert!(transport.is_ok());
        
        let transport = transport.unwrap();
        assert!(transport.local_addr().port() > 0);
    }
    
    #[tokio::test]
    async fn test_webrtc_server_creation() {
        let config = WebRtcServerConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            ..Default::default()
        };
        
        let server = WebRtcServer::start(config).await;
        assert!(server.is_ok());
        
        let server = server.unwrap();
        assert!(server.local_addr().port() > 0);
        assert_eq!(server.manager.session_count(), 0);
        
        // Create a session
        let camera_id = Uuid::new_v4();
        let session_id = server.create_session(camera_id, "main");
        assert!(session_id.is_ok());
        assert_eq!(server.manager.session_count(), 1);
        
        server.shutdown();
    }
    
    #[test]
    fn test_source_addr_cache() {
        let manager = WebRtcManager::default();
        let camera_id = Uuid::new_v4();
        
        let session_id = manager.create_session(camera_id, "main").unwrap();
        
        // Cache should be empty initially
        assert_eq!(manager.cache_size(), 0);
        
        // Manually populate cache (simulating what route_packet does)
        let source_addr: SocketAddr = "192.168.1.100:50000".parse().unwrap();
        manager.source_addr_cache.write().insert(source_addr, session_id);
        
        assert_eq!(manager.cache_size(), 1);
        
        // Cache lookup should work
        let cached_id = manager.source_addr_cache.read().get(&source_addr).copied();
        assert_eq!(cached_id, Some(session_id));
        
        // Removing session should clean cache
        manager.remove_session(session_id);
        assert_eq!(manager.cache_size(), 0);
        assert_eq!(manager.session_count(), 0);
    }
    
    #[test]
    fn test_cache_clear() {
        let manager = WebRtcManager::default();
        let camera_id = Uuid::new_v4();
        
        let session_id = manager.create_session(camera_id, "main").unwrap();
        
        // Add some cache entries
        let addr1: SocketAddr = "192.168.1.100:50000".parse().unwrap();
        let addr2: SocketAddr = "192.168.1.101:50001".parse().unwrap();
        manager.source_addr_cache.write().insert(addr1, session_id);
        manager.source_addr_cache.write().insert(addr2, session_id);
        
        assert_eq!(manager.cache_size(), 2);
        
        // Clear cache
        manager.clear_cache();
        assert_eq!(manager.cache_size(), 0);
    }
}
