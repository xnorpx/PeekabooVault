pub mod api;
pub mod bandwidth_manager;
pub mod clip_export;
pub mod config;
pub mod db;
pub mod discovery;
pub mod frame_store;
pub mod health_monitor;
pub mod hot_cold_db;
pub mod migration_job;
pub mod mp4_muxer;
pub mod paths;
pub mod recorder;
pub mod replay_coordinator;
pub mod rtsp_client;
pub mod server;
pub mod server_task;
pub mod state;
pub mod stream_manager;
pub mod stream_router;
pub mod ui;
pub mod webrtc;

pub use config::Config;
pub use db::Database;
pub use frame_store::FrameStore;
pub use health_monitor::{HealthConfig, HealthMonitor, HealthState, EventNormalizer};
pub use hot_cold_db::HotColdDb;
pub use migration_job::MigrationJob;
pub use mp4_muxer::{Mp4Muxer, H264Frame, NalUnit, NalUnitType, H264Config};
pub use paths::PathInfo;
pub use recorder::{RecorderConfig, RecorderHandle, start_recording, decode_video_index, DecodedFrameIndex};
pub use replay_coordinator::{ReplayCoordinator, ReplayCoordinatorConfig, SimulcastLayer, MAX_CAMERAS};
pub use rtsp_client::{RtspClient, RtspConfig};
pub use server::run_server;
pub use server_task::{ServerTask, ServerMessage};
pub use stream_manager::StreamManager;
pub use stream_router::{StreamRouterConfig, StreamRouterManager, CameraStreamRouter};
pub use bandwidth_manager::{BandwidthBudgetManager, BandwidthManagerConfig, BudgetSummary, FocusResult};
pub use ui::{generate_live_viewer, generate_replay_page, generate_dashboard, generate_multi_replay_page};
pub use webrtc::{
    WebRtcConfig, WebRtcManager, WebRtcSession, PlaybackState as WebRtcPlaybackState, LayerState,
    ForwardingContext, start_forwarding, start_forwarding_with_commands,
};
pub use api::{
    DataChannelCommand, DataChannelMessage, PlaybackMode, VideoLayer, LayerPreference, UiContext,
    MultiReplayRequest, MultiReplayResponse, MultiReplayCommand, MultiReplayMessage, MultiReplayState,
};
