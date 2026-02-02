//! ONVIF discovery service
//!
//! Wraps the onvif crate's discovery functionality with our API types.

use crate::api::DiscoveredDevice;
use chrono::Utc;
use onvif_rs::discovery::DiscoveryBuilder;
use onvif_rs::soap::client::{ClientBuilder, Credentials};
use std::time::Duration;
use tokio_stream::StreamExt;
use tracing::{debug, info, warn};
use url::Url;
use uuid::Uuid;

/// Discover ONVIF devices on the network
pub async fn discover_devices(duration_secs: u64) -> anyhow::Result<Vec<DiscoveredDevice>> {
    info!(duration_secs, "Starting ONVIF discovery scan");

    let duration = Duration::from_secs(duration_secs);

    let mut discovered = Vec::new();

    // Use the onvif crate's discovery - need to bind the builder to extend its lifetime
    let mut builder = DiscoveryBuilder::default();
    builder.duration(duration);

    let stream = builder.run().await?;

    tokio::pin!(stream);

    while let Some(device) = stream.next().await {
        debug!(?device, "Discovered ONVIF device");

        let discovered_device = DiscoveredDevice {
            id: Uuid::new_v4(),
            address: device.address.clone(),
            name: device.name.clone(),
            hardware: device.hardware.clone(),
            types: device.types.clone(),
            urls: device.urls.clone(),
            discovered_at: Utc::now(),
            is_onboarded: false,
        };

        discovered.push(discovered_device);
    }

    info!(count = discovered.len(), "Discovery scan complete");

    Ok(discovered)
}

/// Create an ONVIF client for a device
fn create_client(
    url: &Url,
    username: Option<&str>,
    password: Option<&str>,
) -> onvif_rs::soap::client::Client {
    let mut builder = ClientBuilder::new(url);

    if let (Some(u), Some(p)) = (username, password) {
        builder = builder.credentials(Some(Credentials {
            username: u.to_string(),
            password: p.to_string(),
        }));
    }

    builder.build()
}

/// Probe a specific ONVIF device to get device information
pub async fn probe_device(
    url: &Url,
    username: Option<&str>,
    password: Option<&str>,
) -> anyhow::Result<DeviceInfo> {
    use schema::devicemgmt::{self, GetDeviceInformation};

    info!(%url, "Probing ONVIF device");

    let client = create_client(url, username, password);

    // Get device information
    let response = devicemgmt::get_device_information(&client, &GetDeviceInformation {}).await?;

    let info = DeviceInfo {
        manufacturer: Some(response.manufacturer),
        model: Some(response.model),
        firmware_version: Some(response.firmware_version),
        serial_number: Some(response.serial_number),
        hardware_id: Some(response.hardware_id),
    };

    debug!(?info, "Got device information");

    Ok(info)
}

/// Get stream URIs from an ONVIF device
pub async fn get_stream_uris(
    url: &Url,
    username: Option<&str>,
    password: Option<&str>,
) -> anyhow::Result<Vec<StreamInfo>> {
    use schema::media::{self, GetProfiles, GetStreamUri};
    use schema::onvif::{StreamSetup, StreamType, Transport as OnvifTransport, TransportProtocol};

    info!(%url, "Getting stream URIs from ONVIF device");

    let client = create_client(url, username, password);

    // Get media profiles
    let profiles_response = media::get_profiles(&client, &GetProfiles {}).await?;

    let mut streams = Vec::new();

    for profile in profiles_response.profiles {
        let stream_setup = StreamSetup {
            stream: StreamType::RtpUnicast,
            transport: OnvifTransport {
                protocol: TransportProtocol::Rtsp,
                tunnel: vec![],
            },
        };

        let uri_request = GetStreamUri {
            profile_token: schema::onvif::ReferenceToken(profile.token.0.clone()),
            stream_setup,
        };

        match media::get_stream_uri(&client, &uri_request).await {
            Ok(uri_response) => {
                let video_config = profile.video_encoder_configuration.as_ref();
                let rate_control = video_config.and_then(|v| v.rate_control.as_ref());
                let h264_config = video_config.and_then(|v| v.h264.as_ref());

                streams.push(StreamInfo {
                    profile_token: profile.token.0.clone(),
                    profile_name: profile.name.0.clone(),
                    stream_uri: uri_response.media_uri.uri,
                    encoding: video_config.map(|v| format!("{:?}", v.encoding)),
                    width: video_config.map(|v| v.resolution.width as u32),
                    height: video_config.map(|v| v.resolution.height as u32),
                    frame_rate: rate_control.map(|r| r.frame_rate_limit as f32),
                    bitrate_limit: rate_control.map(|r| r.bitrate_limit),
                    quality: video_config.map(|v| v.quality),
                    gov_length: h264_config.map(|h| h.gov_length),
                    h264_profile: h264_config.map(|h| format!("{:?}", h.h264_profile)),
                    encoding_interval: rate_control.map(|r| r.encoding_interval),
                    guaranteed_frame_rate: video_config.and_then(|v| v.guaranteed_frame_rate),
                });
            }
            Err(e) => {
                warn!(profile = %profile.token.0, error = %e, "Failed to get stream URI for profile");
            }
        }
    }

    info!(count = streams.len(), "Got stream URIs");

    Ok(streams)
}

/// Device information from ONVIF GetDeviceInformation
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub firmware_version: Option<String>,
    pub serial_number: Option<String>,
    pub hardware_id: Option<String>,
}

/// Stream information from ONVIF GetStreamUri
#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub profile_token: String,
    pub profile_name: String,
    pub stream_uri: String,
    /// Video codec (H264, JPEG, MPEG4)
    pub encoding: Option<String>,
    /// Video width in pixels
    pub width: Option<u32>,
    /// Video height in pixels  
    pub height: Option<u32>,
    /// Maximum frame rate in fps
    pub frame_rate: Option<f32>,
    /// Maximum bitrate in kbps
    pub bitrate_limit: Option<i32>,
    /// Quality setting (0-100)
    pub quality: Option<f64>,
    /// GOP length (I-frame interval)
    pub gov_length: Option<i32>,
    /// H.264 profile (Baseline, Main, High, Extended)
    pub h264_profile: Option<String>,
    /// Encoding interval (1 = every frame, 2 = every other frame, etc.)
    pub encoding_interval: Option<i32>,
    /// Whether frame rate is guaranteed
    pub guaranteed_frame_rate: Option<bool>,
}
