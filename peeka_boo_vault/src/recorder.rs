//! Recording Pipeline - Writes video segments to disk
//!
//! This module implements Gap 2 from DESIGN.md:
//! - Subscribes to StreamManager sessions
//! - Accumulates video chunks into ~60 second segments
//! - Writes segments to disk with Moonfire-style schema
//! - Inserts recording metadata into HotColdDb
//!
//! The recorder is the "middle link" between StreamManager and HotColdDb.

use crate::db::{datetime_to_ticks, DbRecording, TICKS_PER_SEC};
use crate::hot_cold_db::HotColdDb;
use crate::stream_manager::{StreamChunk, StreamSession};

use anyhow::{Context, Result};
use bytes::{BufMut, BytesMut};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tracing::{debug, error, info};
use uuid::Uuid;

/// Decoded frame index entry (public for use by MP4 muxer/clip export)
#[derive(Debug, Clone)]
pub struct DecodedFrameIndex {
    /// Byte offset in the segment file
    pub offset: u64,
    /// Size in bytes
    pub size: u32,
    /// Whether this is a keyframe
    pub is_keyframe: bool,
    /// Duration since previous frame in ticks
    pub duration_ticks: i32,
    /// Absolute timestamp in ticks from segment start
    pub timestamp_ticks: i64,
}

/// Decode a video index blob from the database
///
/// Returns a list of frame entries with offsets, sizes, and keyframe flags.
/// This enables efficient seeking and MP4 reconstruction.
pub fn decode_video_index(index_blob: &[u8]) -> Result<Vec<DecodedFrameIndex>> {
    let mut frames = Vec::new();
    let mut offset = 0u64;
    let mut timestamp_ticks = 0i64;
    let mut pos = 0usize;

    while pos + 9 <= index_blob.len() {
        // Read flags (1 byte)
        let flags = index_blob[pos];
        let is_keyframe = (flags & 0x01) != 0;
        pos += 1;

        // Read duration_ticks (4 bytes, i32 LE)
        let duration_ticks = i32::from_le_bytes([
            index_blob[pos],
            index_blob[pos + 1],
            index_blob[pos + 2],
            index_blob[pos + 3],
        ]);
        pos += 4;

        // Read size (4 bytes, u32 LE)
        let size = u32::from_le_bytes([
            index_blob[pos],
            index_blob[pos + 1],
            index_blob[pos + 2],
            index_blob[pos + 3],
        ]);
        pos += 4;

        // Calculate absolute timestamp for this frame
        timestamp_ticks += duration_ticks as i64;

        frames.push(DecodedFrameIndex {
            offset,
            size,
            is_keyframe,
            duration_ticks,
            timestamp_ticks,
        });

        // Update offset for next frame
        offset += size as u64;
    }

    if pos != index_blob.len() {
        anyhow::bail!(
            "Invalid video index: {} bytes remaining after parsing {} frames",
            index_blob.len() - pos,
            frames.len()
        );
    }

    Ok(frames)
}

/// Default segment duration (60 seconds)
const DEFAULT_SEGMENT_DURATION: Duration = Duration::from_secs(60);

/// Maximum segment size (100MB)
const MAX_SEGMENT_BYTES: usize = 100 * 1024 * 1024;

/// Recorder configuration
#[derive(Debug, Clone)]
pub struct RecorderConfig {
    /// Target segment duration
    pub segment_duration: Duration,
    /// Maximum segment size in bytes
    pub max_segment_bytes: usize,
    /// Storage directory for video files
    pub storage_path: PathBuf,
    /// Sample file directory ID in database
    pub sample_file_dir_id: i64,
    /// Stream ID in database
    pub stream_id: i64,
    /// Run ID for continuous recording detection
    pub run_id: i64,
}

impl Default for RecorderConfig {
    fn default() -> Self {
        Self {
            segment_duration: DEFAULT_SEGMENT_DURATION,
            max_segment_bytes: MAX_SEGMENT_BYTES,
            storage_path: PathBuf::from("./recordings"),
            sample_file_dir_id: 1,
            stream_id: 1,
            run_id: 1,
        }
    }
}

/// Recording task handle
pub struct RecorderHandle {
    /// Channel to signal stop
    stop_tx: mpsc::Sender<()>,
    /// Task join handle
    task_handle: tokio::task::JoinHandle<()>,
    /// Channel to send frames to the recorder (for ServerTask message-passing architecture)
    pub frame_tx: Option<mpsc::UnboundedSender<crate::server_task::VideoFrame>>,
}

impl RecorderHandle {
    /// Stop the recorder
    pub async fn stop(self) {
        let _ = self.stop_tx.send(()).await;
        let _ = self.task_handle.await;
    }
}

/// Statistics for a recording segment
#[derive(Debug, Clone, Default)]
pub struct SegmentStats {
    /// Total bytes written
    pub bytes_written: u64,
    /// Number of video samples
    pub video_samples: u64,
    /// Number of keyframes (sync samples)
    pub video_sync_samples: u64,
    /// Start timestamp (RTP)
    pub start_timestamp: Option<u32>,
    /// Last timestamp (RTP)
    pub last_timestamp: Option<u32>,
    /// Start wall clock time
    pub start_time: Option<Instant>,
}

/// Information about a single frame for index building
#[derive(Debug, Clone)]
struct FrameIndexEntry {
    /// Byte offset in the segment file
    offset: u64,
    /// Size in bytes
    size: u32,
    /// Whether this is a keyframe
    is_keyframe: bool,
    /// Duration since previous frame in ticks (for seeking)
    duration_ticks: i32,
}

/// Start a recording task for a stream session
///
/// This implements the disk write pipeline:
/// StreamSession -> subscribe() -> RecordingTask -> HotColdDb + filesystem
pub async fn start_recording(
    session: Arc<StreamSession>,
    db: Arc<tokio::sync::RwLock<HotColdDb>>,
    config: RecorderConfig,
) -> Result<RecorderHandle> {
    // Subscribe to the session
    let (prebuffered, rx) = session.subscribe();
    
    let (stop_tx, stop_rx) = mpsc::channel(1);
    
    let task_handle = tokio::spawn(async move {
        if let Err(e) = recording_loop(rx, stop_rx, db, config, prebuffered).await {
            error!("Recording task failed: {}", e);
        }
    });
    
    Ok(RecorderHandle {
        stop_tx,
        task_handle,
        frame_tx: None, // For now, will be populated when using ServerTask architecture
    })
}

/// Main recording loop
async fn recording_loop(
    mut rx: mpsc::Receiver<StreamChunk>,
    mut stop_rx: mpsc::Receiver<()>,
    db: Arc<tokio::sync::RwLock<HotColdDb>>,
    config: RecorderConfig,
    prebuffered: Vec<StreamChunk>,
) -> Result<()> {
    // Ensure storage directory exists
    tokio::fs::create_dir_all(&config.storage_path)
        .await
        .context("Failed to create storage directory")?;
    
    let mut segment = SegmentBuilder::new(&config);
    
    // Process prebuffered data first
    for chunk in prebuffered {
        if chunk.is_keyframe {
            // Start segment on keyframe
            if !segment.is_empty() {
                // Finish current segment
                if let Err(e) = segment.finalize(&db).await {
                    error!("Failed to finalize segment: {}", e);
                }
                segment = SegmentBuilder::new(&config);
            }
        }
        segment.add_chunk(chunk)?;
    }
    
    info!("Recording started");
    
    loop {
        tokio::select! {
            _ = stop_rx.recv() => {
                info!("Stop signal received");
                break;
            }
            chunk = rx.recv() => {
                match chunk {
                    Some(chunk) => {
                        // Check if we should start a new segment
                        let should_rotate = segment.should_rotate(&config);
                        
                        if should_rotate && chunk.is_keyframe {
                            // Finalize current segment
                            if !segment.is_empty() {
                                if let Err(e) = segment.finalize(&db).await {
                                    error!("Failed to finalize segment: {}", e);
                                }
                            }
                            
                            // Start new segment
                            segment = SegmentBuilder::new(&config);
                        }
                        
                        segment.add_chunk(chunk)?;
                    }
                    None => {
                        info!("Stream ended");
                        break;
                    }
                }
            }
        }
    }
    
    // Finalize any remaining segment
    if !segment.is_empty() {
        if let Err(e) = segment.finalize(&db).await {
            error!("Failed to finalize final segment: {}", e);
        }
    }
    
    info!("Recording stopped");
    Ok(())
}

/// Builds a recording segment
struct SegmentBuilder {
    /// Unique ID for this segment's file
    file_id: Uuid,
    /// Storage path
    storage_path: PathBuf,
    /// Accumulated video data
    data: BytesMut,
    /// Statistics
    stats: SegmentStats,
    /// Config reference
    sample_file_dir_id: i64,
    stream_id: i64,
    run_id: i64,
    /// Frame index entries for seeking
    frame_index: Vec<FrameIndexEntry>,
    /// Last frame timestamp for computing deltas
    last_frame_timestamp: Option<u32>,
}

impl SegmentBuilder {
    fn new(config: &RecorderConfig) -> Self {
        Self {
            file_id: Uuid::new_v4(),
            storage_path: config.storage_path.clone(),
            data: BytesMut::with_capacity(config.max_segment_bytes),
            stats: SegmentStats::default(),
            sample_file_dir_id: config.sample_file_dir_id,
            stream_id: config.stream_id,
            run_id: config.run_id,
            frame_index: Vec::new(),
            last_frame_timestamp: None,
        }
    }
    
    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
    
    fn should_rotate(&self, config: &RecorderConfig) -> bool {
        // Rotate if we've exceeded duration or size limits
        if let Some(start_time) = self.stats.start_time {
            if start_time.elapsed() >= config.segment_duration {
                return true;
            }
        }
        
        if self.data.len() >= config.max_segment_bytes {
            return true;
        }
        
        false
    }
    
    fn add_chunk(&mut self, chunk: StreamChunk) -> Result<()> {
        // Track stats
        if self.stats.start_time.is_none() {
            self.stats.start_time = Some(chunk.received_at);
            self.stats.start_timestamp = Some(chunk.timestamp);
        }
        self.stats.last_timestamp = Some(chunk.timestamp);
        self.stats.video_samples += 1;
        if chunk.is_keyframe {
            self.stats.video_sync_samples += 1;
        }

        // Build frame index entry for seeking
        let current_offset = self.data.len() as u64;
        let frame_size = chunk.data.len() as u32;

        // Calculate duration since last frame in ticks
        // RTP timestamps are already in 90kHz (same as TICKS_PER_SEC)
        let duration_ticks = if let Some(last_ts) = self.last_frame_timestamp {
            chunk.timestamp.wrapping_sub(last_ts) as i32
        } else {
            0 // First frame, no delta
        };

        self.frame_index.push(FrameIndexEntry {
            offset: current_offset,
            size: frame_size,
            is_keyframe: chunk.is_keyframe,
            duration_ticks,
        });

        self.last_frame_timestamp = Some(chunk.timestamp);

        // Add data
        self.data.put_slice(&chunk.data);
        self.stats.bytes_written += chunk.data.len() as u64;

        Ok(())
    }

    /// Encode frame index into binary format for database storage
    ///
    /// Format: Sequence of variable-length records
    /// Each record:
    ///   - flags: u8 (bit 0 = is_keyframe, bits 1-7 reserved)
    ///   - duration_ticks: i32 LE (time since last frame)
    ///   - size: u32 LE (frame size in bytes)
    ///
    /// This format allows efficient seeking by skipping to keyframes
    /// and calculating positions without decompressing video.
    fn encode_video_index(&self) -> Vec<u8> {
        let mut index = Vec::with_capacity(self.frame_index.len() * 9); // Estimate: 1 + 4 + 4 per frame

        for entry in &self.frame_index {
            // Flags byte
            let flags: u8 = if entry.is_keyframe { 0x01 } else { 0x00 };
            index.push(flags);

            // Duration since last frame (i32 LE)
            index.extend_from_slice(&entry.duration_ticks.to_le_bytes());

            // Frame size (u32 LE)
            index.extend_from_slice(&entry.size.to_le_bytes());
        }

        index
    }

    async fn finalize(self, db: &Arc<tokio::sync::RwLock<HotColdDb>>) -> Result<()> {
        if self.is_empty() {
            return Ok(());
        }
        
        // Generate filename
        let filename = format!("{}.mp4", self.file_id);
        let file_path = self.storage_path.join(&filename);
        
        debug!(
            file = %file_path.display(),
            bytes = self.stats.bytes_written,
            samples = self.stats.video_samples,
            keyframes = self.stats.video_sync_samples,
            "Writing segment"
        );
        
        // Write data to file
        let mut file = File::create(&file_path)
            .await
            .context("Failed to create segment file")?;
        
        file.write_all(&self.data)
            .await
            .context("Failed to write segment data")?;
        
        file.sync_all()
            .await
            .context("Failed to sync segment file")?;
        
        // Calculate duration in ticks
        let duration_ticks = if let (Some(start), Some(end)) = (self.stats.start_timestamp, self.stats.last_timestamp) {
            // RTP timestamps are in 90kHz for video (standard)
            (end.wrapping_sub(start)) as i64
        } else {
            // Fallback: estimate from wall clock time
            self.stats.start_time
                .map(|t| t.elapsed().as_secs() as i64 * TICKS_PER_SEC)
                .unwrap_or(0)
        };
        
        // Calculate start time in ticks
        let start_ticks = self.stats.start_time
            .map(|t| {
                let now = chrono::Utc::now();
                let duration_since_start = t.elapsed();
                let start_dt = now - chrono::Duration::from_std(duration_since_start).unwrap_or_default();
                datetime_to_ticks(start_dt)
            })
            .unwrap_or(0);
        
        // Build video index for seeking
        let video_index = if !self.frame_index.is_empty() {
            Some(self.encode_video_index())
        } else {
            None
        };

        debug!(
            frames = self.frame_index.len(),
            index_bytes = video_index.as_ref().map(|v| v.len()).unwrap_or(0),
            "Built video index"
        );

        // Insert recording into database
        let recording = DbRecording {
            id: None, // Will be set by database
            stream_id: self.stream_id,
            open_id: 0, // Will be set by HotColdDb
            sample_file_dir_id: self.sample_file_dir_id,
            start_ticks,
            duration_ticks,
            sample_file_bytes: self.stats.bytes_written as i64,
            video_samples: self.stats.video_samples as i64,
            video_sync_samples: self.stats.video_sync_samples as i64,
            video_index,
            run_id: self.run_id,
            flags: 0, // Complete
        };
        
        let recording_id = db.write().await
            .insert_recording(&recording)
            .await
            .context("Failed to insert recording into database")?;
        
        info!(
            recording_id = recording_id,
            file = %file_path.display(),
            duration_ms = duration_ticks * 1000 / TICKS_PER_SEC,
            bytes = self.stats.bytes_written,
            "Segment saved"
        );
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use std::time::Instant;

    #[test]
    fn test_recorder_config_default() {
        let config = RecorderConfig::default();
        assert_eq!(config.segment_duration, Duration::from_secs(60));
        assert_eq!(config.max_segment_bytes, 100 * 1024 * 1024);
    }

    #[test]
    fn test_segment_builder_empty() {
        let config = RecorderConfig::default();
        let segment = SegmentBuilder::new(&config);
        assert!(segment.is_empty());
        assert!(!segment.should_rotate(&config));
    }

    #[test]
    fn test_segment_builder_add_chunk() {
        let config = RecorderConfig::default();
        let mut segment = SegmentBuilder::new(&config);
        
        let chunk = StreamChunk {
            data: Bytes::from(vec![0u8; 1000]),
            timestamp: 0,
            is_keyframe: true,
            received_at: Instant::now(),
            codec_extra: None,
        };
        
        segment.add_chunk(chunk).unwrap();
        
        assert!(!segment.is_empty());
        assert_eq!(segment.stats.bytes_written, 1000);
        assert_eq!(segment.stats.video_samples, 1);
        assert_eq!(segment.stats.video_sync_samples, 1);
    }

    #[test]
    fn test_segment_rotation_by_size() {
        let mut config = RecorderConfig::default();
        config.max_segment_bytes = 2000; // Small limit for testing
        
        let mut segment = SegmentBuilder::new(&config);
        
        // Add chunk that exceeds size limit
        let chunk = StreamChunk {
            data: Bytes::from(vec![0u8; 2500]),
            timestamp: 0,
            is_keyframe: true,
            received_at: Instant::now(),
            codec_extra: None,
        };
        
        segment.add_chunk(chunk).unwrap();
        
        // Should rotate due to size
        assert!(segment.should_rotate(&config));
    }

    #[test]
    fn test_segment_stats_tracking() {
        let config = RecorderConfig::default();
        let mut segment = SegmentBuilder::new(&config);
        
        // Add keyframe
        let chunk1 = StreamChunk {
            data: Bytes::from(vec![0u8; 100]),
            timestamp: 1000,
            is_keyframe: true,
            received_at: Instant::now(),
            codec_extra: None,
        };
        segment.add_chunk(chunk1).unwrap();
        
        // Add non-keyframe
        let chunk2 = StreamChunk {
            data: Bytes::from(vec![0u8; 50]),
            timestamp: 2000,
            is_keyframe: false,
            received_at: Instant::now(),
            codec_extra: None,
        };
        segment.add_chunk(chunk2).unwrap();
        
        assert_eq!(segment.stats.video_samples, 2);
        assert_eq!(segment.stats.video_sync_samples, 1);
        assert_eq!(segment.stats.bytes_written, 150);
        assert_eq!(segment.stats.start_timestamp, Some(1000));
        assert_eq!(segment.stats.last_timestamp, Some(2000));
    }

    #[test]
    fn test_video_index_building() {
        let config = RecorderConfig::default();
        let mut segment = SegmentBuilder::new(&config);

        // Add a keyframe
        let chunk1 = StreamChunk {
            data: Bytes::from(vec![0u8; 1000]),
            timestamp: 90000, // 1 second at 90kHz
            is_keyframe: true,
            received_at: Instant::now(),
            codec_extra: None,
        };
        segment.add_chunk(chunk1).unwrap();

        // Add P-frame
        let chunk2 = StreamChunk {
            data: Bytes::from(vec![0u8; 300]),
            timestamp: 93000, // +33ms (30fps)
            is_keyframe: false,
            received_at: Instant::now(),
            codec_extra: None,
        };
        segment.add_chunk(chunk2).unwrap();

        // Add another P-frame
        let chunk3 = StreamChunk {
            data: Bytes::from(vec![0u8; 250]),
            timestamp: 96000, // +33ms
            is_keyframe: false,
            received_at: Instant::now(),
            codec_extra: None,
        };
        segment.add_chunk(chunk3).unwrap();

        // Check frame index was built
        assert_eq!(segment.frame_index.len(), 3);

        // Check first frame (keyframe)
        assert_eq!(segment.frame_index[0].offset, 0);
        assert_eq!(segment.frame_index[0].size, 1000);
        assert!(segment.frame_index[0].is_keyframe);
        assert_eq!(segment.frame_index[0].duration_ticks, 0); // First frame

        // Check second frame
        assert_eq!(segment.frame_index[1].offset, 1000);
        assert_eq!(segment.frame_index[1].size, 300);
        assert!(!segment.frame_index[1].is_keyframe);
        assert_eq!(segment.frame_index[1].duration_ticks, 3000); // 93000 - 90000

        // Check third frame
        assert_eq!(segment.frame_index[2].offset, 1300);
        assert_eq!(segment.frame_index[2].size, 250);
        assert!(!segment.frame_index[2].is_keyframe);
        assert_eq!(segment.frame_index[2].duration_ticks, 3000); // 96000 - 93000
    }

    #[test]
    fn test_video_index_encode_decode() {
        let config = RecorderConfig::default();
        let mut segment = SegmentBuilder::new(&config);

        // Add several frames with varying properties
        let frames = vec![
            (5000, 90000, true),   // Keyframe
            (1500, 93000, false),  // P-frame
            (1200, 96000, false),  // P-frame
            (4500, 99000, true),   // Keyframe
            (1800, 102000, false), // P-frame
        ];

        for (size, timestamp, is_keyframe) in frames {
            let chunk = StreamChunk {
                data: Bytes::from(vec![0u8; size]),
                timestamp,
                is_keyframe,
                received_at: Instant::now(),
                codec_extra: None,
            };
            segment.add_chunk(chunk).unwrap();
        }

        // Encode the index
        let encoded = segment.encode_video_index();
        assert!(!encoded.is_empty());
        assert_eq!(encoded.len(), 5 * 9); // 5 frames * 9 bytes each

        // Decode the index
        let decoded = decode_video_index(&encoded).unwrap();
        assert_eq!(decoded.len(), 5);

        // Verify first frame (keyframe)
        assert_eq!(decoded[0].offset, 0);
        assert_eq!(decoded[0].size, 5000);
        assert!(decoded[0].is_keyframe);
        assert_eq!(decoded[0].duration_ticks, 0);
        assert_eq!(decoded[0].timestamp_ticks, 0);

        // Verify second frame
        assert_eq!(decoded[1].offset, 5000);
        assert_eq!(decoded[1].size, 1500);
        assert!(!decoded[1].is_keyframe);
        assert_eq!(decoded[1].duration_ticks, 3000); // 93000 - 90000
        assert_eq!(decoded[1].timestamp_ticks, 3000);

        // Verify last frame
        assert_eq!(decoded[4].offset, 5000 + 1500 + 1200 + 4500);
        assert_eq!(decoded[4].size, 1800);
        assert!(!decoded[4].is_keyframe);
        assert_eq!(decoded[4].duration_ticks, 3000); // 102000 - 99000
        assert_eq!(decoded[4].timestamp_ticks, 12000); // Sum of all durations
    }

    #[test]
    fn test_video_index_keyframe_finding() {
        // Test that keyframes can be easily identified in the index
        let config = RecorderConfig::default();
        let mut segment = SegmentBuilder::new(&config);

        // Add I-frame followed by several P-frames, then another I-frame
        for i in 0..10 {
            let is_keyframe = i == 0 || i == 5; // Keyframes at 0 and 5
            let chunk = StreamChunk {
                data: Bytes::from(vec![0u8; if is_keyframe { 5000 } else { 1000 }]),
                timestamp: 90000 + (i * 3000),
                is_keyframe,
                received_at: Instant::now(),
                codec_extra: None,
            };
            segment.add_chunk(chunk).unwrap();
        }

        let encoded = segment.encode_video_index();
        let decoded = decode_video_index(&encoded).unwrap();

        // Find all keyframes
        let keyframes: Vec<_> = decoded
            .iter()
            .enumerate()
            .filter(|(_, f)| f.is_keyframe)
            .collect();

        assert_eq!(keyframes.len(), 2);
        assert_eq!(keyframes[0].0, 0); // First frame
        assert_eq!(keyframes[1].0, 5); // Sixth frame
    }

    #[test]
    fn test_video_index_empty() {
        // Test encoding empty index
        let config = RecorderConfig::default();
        let segment = SegmentBuilder::new(&config);

        let encoded = segment.encode_video_index();
        assert!(encoded.is_empty());

        // Test decoding empty index
        let decoded = decode_video_index(&[]).unwrap();
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_video_index_invalid_data() {
        // Test decoding invalid data (incomplete frame record)
        // 5 bytes when we need 9 for a complete frame
        let invalid_data = vec![0x01, 0x00, 0x00, 0x00, 0x00];
        let result = decode_video_index(&invalid_data);

        // Should error because of remaining bytes that don't form a complete frame
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("remaining"));
    }
}
