//! PeekabooVault NVR - Camera Discovery and Recording
//!
//! This is the main library for PeekabooVault, a Rust-based Network Video Recorder.
//! It handles ONVIF camera discovery, RTSP streaming, and video recording.

pub mod api;
pub mod config;
pub mod discovery;
pub mod server;
pub mod state;

pub use config::Config;
pub use server::run_server;
