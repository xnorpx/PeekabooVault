//! Frame Storage - Stores and retrieves video frames for playback
//!
//! This module handles:
//! - Storing video frames with metadata (timestamp, keyframe flag, codec info)
//! - Time-based frame queries with keyframe seeking
//! - Efficient playback retrieval starting from keyframes

use anyhow::{Context, Result};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio_rusqlite::Connection as AsyncConnection;
use uuid::Uuid;

/// Frame storage schema SQL
const FRAME_SCHEMA_SQL: &str = r#"
-- Frame index table - metadata about each stored frame
CREATE TABLE IF NOT EXISTS frame_index (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    camera_id TEXT NOT NULL,
    stream_type TEXT NOT NULL,  -- 'main' or 'sub'
    timestamp_ms INTEGER NOT NULL,  -- Unix timestamp in milliseconds
    sequence_num INTEGER NOT NULL,  -- Frame sequence within stream
    is_keyframe INTEGER NOT NULL,   -- 1 if keyframe (IDR), 0 otherwise
    frame_size INTEGER NOT NULL,    -- Size in bytes
    codec TEXT NOT NULL,            -- 'h264' or 'h265'
    file_path TEXT NOT NULL,        -- Relative path to frame data file
    file_offset INTEGER NOT NULL,   -- Offset within file
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Indexes for efficient queries
CREATE INDEX IF NOT EXISTS idx_frame_camera_time 
    ON frame_index(camera_id, stream_type, timestamp_ms);
CREATE INDEX IF NOT EXISTS idx_frame_keyframe 
    ON frame_index(camera_id, stream_type, is_keyframe, timestamp_ms);
CREATE INDEX IF NOT EXISTS idx_frame_sequence 
    ON frame_index(camera_id, stream_type, sequence_num);

-- Recording segments table - groups frames into segments for retention
CREATE TABLE IF NOT EXISTS recording_segment (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    camera_id TEXT NOT NULL,
    stream_type TEXT NOT NULL,
    start_time_ms INTEGER NOT NULL,
    end_time_ms INTEGER,  -- NULL if segment is still being written
    frame_count INTEGER NOT NULL DEFAULT 0,
    keyframe_count INTEGER NOT NULL DEFAULT 0,
    total_bytes INTEGER NOT NULL DEFAULT 0,
    file_path TEXT NOT NULL,
    is_complete INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_segment_camera_time 
    ON recording_segment(camera_id, stream_type, start_time_ms);
"#;

/// A stored video frame with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredFrame {
    pub id: i64,
    pub camera_id: Uuid,
    pub stream_type: String,
    pub timestamp: DateTime<Utc>,
    pub sequence_num: i64,
    pub is_keyframe: bool,
    pub frame_size: u64,
    pub codec: String,
}

/// Frame data with its metadata
#[derive(Debug, Clone)]
pub struct FrameData {
    pub metadata: StoredFrame,
    pub data: Bytes,
}

/// A recording segment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingSegment {
    pub id: i64,
    pub camera_id: Uuid,
    pub stream_type: String,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub frame_count: u64,
    pub keyframe_count: u64,
    pub total_bytes: u64,
    pub file_path: String,
    pub is_complete: bool,
}

/// Seek direction for keyframe finding
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekDirection {
    /// Find keyframe at or before the target time
    Backward,
    /// Find keyframe at or after the target time
    Forward,
    /// Find the closest keyframe (either direction)
    Nearest,
}

/// Frame store for recording and playback
pub struct FrameStore {
    conn: AsyncConnection,
    storage_path: PathBuf,
}

impl FrameStore {
    /// Open or create a frame store
    pub async fn open(db_path: &Path, storage_path: &Path) -> Result<Self> {
        let conn = AsyncConnection::open(db_path)
            .await
            .context("Failed to open frame store database")?;

        let store = Self {
            conn,
            storage_path: storage_path.to_path_buf(),
        };

        store.initialize().await?;
        Ok(store)
    }

    /// Initialize the database schema
    async fn initialize(&self) -> Result<()> {
        self.conn
            .call(|conn| {
                conn.execute_batch("PRAGMA journal_mode=WAL;")?;
                conn.execute_batch("PRAGMA foreign_keys=ON;")?;
                conn.execute_batch(FRAME_SCHEMA_SQL)?;
                Ok(())
            })
            .await
            .context("Failed to initialize frame store schema")
    }

    /// Create a new recording segment
    pub async fn create_segment(
        &self,
        camera_id: Uuid,
        stream_type: &str,
        start_time: DateTime<Utc>,
    ) -> Result<RecordingSegment> {
        let camera_id_str = camera_id.to_string();
        let stream_type = stream_type.to_string();
        let start_time_ms = start_time.timestamp_millis();
        
        // Generate file path for this segment
        let file_name = format!(
            "{}_{}_{}_{}.frames",
            camera_id_str,
            stream_type,
            start_time.format("%Y%m%d_%H%M%S"),
            start_time_ms
        );
        let file_path = format!("{}/{}", camera_id_str, file_name);

        let segment = self
            .conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO recording_segment (camera_id, stream_type, start_time_ms, file_path)
                     VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![camera_id_str, stream_type, start_time_ms, file_path],
                )?;
                let id = conn.last_insert_rowid();
                Ok(RecordingSegment {
                    id,
                    camera_id,
                    stream_type,
                    start_time,
                    end_time: None,
                    frame_count: 0,
                    keyframe_count: 0,
                    total_bytes: 0,
                    file_path,
                    is_complete: false,
                })
            })
            .await
            .context("Failed to create recording segment")?;

        Ok(segment)
    }

    /// Store a frame and update segment
    #[allow(clippy::too_many_arguments)]
    pub async fn store_frame(
        &self,
        segment_id: i64,
        camera_id: Uuid,
        stream_type: &str,
        timestamp: DateTime<Utc>,
        sequence_num: i64,
        is_keyframe: bool,
        codec: &str,
        file_offset: i64,
        frame_size: u64,
    ) -> Result<i64> {
        let camera_id_str = camera_id.to_string();
        let stream_type = stream_type.to_string();
        let timestamp_ms = timestamp.timestamp_millis();
        let codec = codec.to_string();

        let frame_id = self
            .conn
            .call(move |conn| {
                // Get the file path from the segment
                let file_path: String = conn.query_row(
                    "SELECT file_path FROM recording_segment WHERE id = ?1",
                    [segment_id],
                    |row| row.get(0),
                )?;

                // Insert frame index
                conn.execute(
                    "INSERT INTO frame_index 
                     (camera_id, stream_type, timestamp_ms, sequence_num, is_keyframe, 
                      frame_size, codec, file_path, file_offset)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    rusqlite::params![
                        camera_id_str,
                        stream_type,
                        timestamp_ms,
                        sequence_num,
                        is_keyframe as i32,
                        frame_size as i64,
                        codec,
                        file_path,
                        file_offset
                    ],
                )?;
                let frame_id = conn.last_insert_rowid();

                // Update segment statistics
                conn.execute(
                    "UPDATE recording_segment 
                     SET frame_count = frame_count + 1,
                         keyframe_count = keyframe_count + ?1,
                         total_bytes = total_bytes + ?2,
                         end_time_ms = ?3
                     WHERE id = ?4",
                    rusqlite::params![
                        is_keyframe as i32,
                        frame_size as i64,
                        timestamp_ms,
                        segment_id
                    ],
                )?;

                Ok(frame_id)
            })
            .await
            .context("Failed to store frame")?;

        Ok(frame_id)
    }

    /// Complete a recording segment
    pub async fn complete_segment(&self, segment_id: i64) -> Result<()> {
        self.conn
            .call(move |conn| {
                conn.execute(
                    "UPDATE recording_segment SET is_complete = 1 WHERE id = ?1",
                    [segment_id],
                )?;
                Ok(())
            })
            .await
            .context("Failed to complete segment")
    }

    /// Find the nearest keyframe to a given timestamp
    pub async fn find_keyframe(
        &self,
        camera_id: Uuid,
        stream_type: &str,
        target_time: DateTime<Utc>,
        direction: SeekDirection,
    ) -> Result<Option<StoredFrame>> {
        let camera_id_str = camera_id.to_string();
        let stream_type = stream_type.to_string();
        let target_ms = target_time.timestamp_millis();

        let frame = self
            .conn
            .call(move |conn| {
                let frame = match direction {
                    SeekDirection::Backward => {
                        // Find keyframe at or before target
                        conn.query_row(
                            "SELECT id, camera_id, stream_type, timestamp_ms, sequence_num, 
                                    is_keyframe, frame_size, codec
                             FROM frame_index
                             WHERE camera_id = ?1 AND stream_type = ?2 
                                   AND is_keyframe = 1 AND timestamp_ms <= ?3
                             ORDER BY timestamp_ms DESC
                             LIMIT 1",
                            rusqlite::params![camera_id_str, stream_type, target_ms],
                            |row| Ok(row_to_stored_frame(row)),
                        )
                        .optional()?
                    }
                    SeekDirection::Forward => {
                        // Find keyframe at or after target
                        conn.query_row(
                            "SELECT id, camera_id, stream_type, timestamp_ms, sequence_num, 
                                    is_keyframe, frame_size, codec
                             FROM frame_index
                             WHERE camera_id = ?1 AND stream_type = ?2 
                                   AND is_keyframe = 1 AND timestamp_ms >= ?3
                             ORDER BY timestamp_ms ASC
                             LIMIT 1",
                            rusqlite::params![camera_id_str, stream_type, target_ms],
                            |row| Ok(row_to_stored_frame(row)),
                        )
                        .optional()?
                    }
                    SeekDirection::Nearest => {
                        // Find both and pick closest
                        let before: Option<(i64, i64)> = conn
                            .query_row(
                                "SELECT id, timestamp_ms FROM frame_index
                                 WHERE camera_id = ?1 AND stream_type = ?2 
                                       AND is_keyframe = 1 AND timestamp_ms <= ?3
                                 ORDER BY timestamp_ms DESC LIMIT 1",
                                rusqlite::params![camera_id_str, stream_type, target_ms],
                                |row| Ok((row.get(0)?, row.get(1)?)),
                            )
                            .optional()?;

                        let after: Option<(i64, i64)> = conn
                            .query_row(
                                "SELECT id, timestamp_ms FROM frame_index
                                 WHERE camera_id = ?1 AND stream_type = ?2 
                                       AND is_keyframe = 1 AND timestamp_ms >= ?3
                                 ORDER BY timestamp_ms ASC LIMIT 1",
                                rusqlite::params![camera_id_str, stream_type, target_ms],
                                |row| Ok((row.get(0)?, row.get(1)?)),
                            )
                            .optional()?;

                        // Pick the closest one
                        let chosen_id = match (before, after) {
                            (Some((bid, bts)), Some((aid, ats))) => {
                                if (target_ms - bts).abs() <= (ats - target_ms).abs() {
                                    Some(bid)
                                } else {
                                    Some(aid)
                                }
                            }
                            (Some((id, _)), None) => Some(id),
                            (None, Some((id, _))) => Some(id),
                            (None, None) => None,
                        };

                        if let Some(id) = chosen_id {
                            conn.query_row(
                                "SELECT id, camera_id, stream_type, timestamp_ms, sequence_num, 
                                        is_keyframe, frame_size, codec
                                 FROM frame_index WHERE id = ?1",
                                [id],
                                |row| Ok(row_to_stored_frame(row)),
                            )
                            .optional()?
                        } else {
                            None
                        }
                    }
                };
                Ok(frame)
            })
            .await
            .context("Failed to find keyframe")?;

        Ok(frame)
    }

    /// Query frames in a time range, starting from nearest keyframe
    pub async fn query_frames(
        &self,
        camera_id: Uuid,
        stream_type: &str,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        start_from_keyframe: bool,
    ) -> Result<Vec<StoredFrame>> {
        let camera_id_str = camera_id.to_string();
        let stream_type = stream_type.to_string();
        let mut start_ms = start_time.timestamp_millis();
        let end_ms = end_time.timestamp_millis();

        // If starting from keyframe, find the keyframe before start_time
        if start_from_keyframe
            && let Some(keyframe) = self
                .find_keyframe(camera_id, &stream_type, start_time, SeekDirection::Backward)
                .await?
        {
            start_ms = keyframe.timestamp.timestamp_millis();
        }

        let frames = self
            .conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, camera_id, stream_type, timestamp_ms, sequence_num, 
                            is_keyframe, frame_size, codec
                     FROM frame_index
                     WHERE camera_id = ?1 AND stream_type = ?2 
                           AND timestamp_ms >= ?3 AND timestamp_ms <= ?4
                     ORDER BY timestamp_ms ASC, sequence_num ASC",
                )?;

                let frames: Vec<StoredFrame> = stmt
                    .query_map(
                        rusqlite::params![camera_id_str, stream_type, start_ms, end_ms],
                        |row| Ok(row_to_stored_frame(row)),
                    )?
                    .filter_map(|r| r.ok())
                    .collect();

                Ok(frames)
            })
            .await
            .context("Failed to query frames")?;

        Ok(frames)
    }

    /// Get frame metadata by ID
    pub async fn get_frame(&self, frame_id: i64) -> Result<Option<StoredFrame>> {
        self.conn
            .call(move |conn| {
                let result = conn
                    .query_row(
                        "SELECT id, camera_id, stream_type, timestamp_ms, sequence_num, 
                                is_keyframe, frame_size, codec
                         FROM frame_index WHERE id = ?1",
                        [frame_id],
                        |row| Ok(row_to_stored_frame(row)),
                    )
                    .optional()?;
                Ok(result)
            })
            .await
            .context("Failed to get frame")
    }

    /// Get the storage path for a frame's data file
    pub fn get_frame_file_path(&self, file_path: &str) -> PathBuf {
        self.storage_path.join(file_path)
    }

    /// Query segments for a camera
    pub async fn query_segments(
        &self,
        camera_id: Uuid,
        stream_type: &str,
        start_time: Option<DateTime<Utc>>,
        end_time: Option<DateTime<Utc>>,
    ) -> Result<Vec<RecordingSegment>> {
        let camera_id_str = camera_id.to_string();
        let stream_type = stream_type.to_string();
        let start_ms = start_time.map(|t| t.timestamp_millis());
        let end_ms = end_time.map(|t| t.timestamp_millis());

        let segments = self
            .conn
            .call(move |conn| {
                let mut query = String::from(
                    "SELECT id, camera_id, stream_type, start_time_ms, end_time_ms,
                            frame_count, keyframe_count, total_bytes, file_path, is_complete
                     FROM recording_segment
                     WHERE camera_id = ?1 AND stream_type = ?2",
                );

                if start_ms.is_some() {
                    query.push_str(" AND (end_time_ms IS NULL OR end_time_ms >= ?3)");
                }
                if end_ms.is_some() {
                    query.push_str(" AND start_time_ms <= ?4");
                }
                query.push_str(" ORDER BY start_time_ms ASC");

                let mut stmt = conn.prepare(&query)?;

                let segments: Vec<RecordingSegment> = match (start_ms, end_ms) {
                    (Some(s), Some(e)) => stmt
                        .query_map(
                            rusqlite::params![camera_id_str, stream_type, s, e],
                            row_to_segment,
                        )?
                        .filter_map(|r| r.ok())
                        .collect(),
                    (Some(s), None) => stmt
                        .query_map(
                            rusqlite::params![camera_id_str, stream_type, s],
                            row_to_segment,
                        )?
                        .filter_map(|r| r.ok())
                        .collect(),
                    (None, Some(e)) => stmt
                        .query_map(
                            rusqlite::params![camera_id_str, stream_type, e],
                            row_to_segment,
                        )?
                        .filter_map(|r| r.ok())
                        .collect(),
                    (None, None) => stmt
                        .query_map(rusqlite::params![camera_id_str, stream_type], row_to_segment)?
                        .filter_map(|r| r.ok())
                        .collect(),
                };

                Ok(segments)
            })
            .await
            .context("Failed to query segments")?;

        Ok(segments)
    }

    /// Get timeline summary (keyframe timestamps) for efficient seeking
    pub async fn get_timeline(
        &self,
        camera_id: Uuid,
        stream_type: &str,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> Result<Vec<DateTime<Utc>>> {
        let camera_id_str = camera_id.to_string();
        let stream_type = stream_type.to_string();
        let start_ms = start_time.timestamp_millis();
        let end_ms = end_time.timestamp_millis();

        let keyframe_times = self
            .conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT timestamp_ms FROM frame_index
                     WHERE camera_id = ?1 AND stream_type = ?2 
                           AND is_keyframe = 1
                           AND timestamp_ms >= ?3 AND timestamp_ms <= ?4
                     ORDER BY timestamp_ms ASC",
                )?;

                let times: Vec<DateTime<Utc>> = stmt
                    .query_map(
                        rusqlite::params![camera_id_str, stream_type, start_ms, end_ms],
                        |row| {
                            let ms: i64 = row.get(0)?;
                            Ok(DateTime::from_timestamp_millis(ms).unwrap_or_default())
                        },
                    )?
                    .filter_map(|r| r.ok())
                    .collect();

                Ok(times)
            })
            .await
            .context("Failed to get timeline")?;

        Ok(keyframe_times)
    }
    
    /// Read a frame at or near a specific timestamp
    /// 
    /// For replay, this finds the frame closest to the target time.
    /// Returns the frame data with its metadata.
    pub async fn read_frame_at_time(
        &self,
        camera_id: Uuid,
        stream_type: &str,
        target_time: DateTime<Utc>,
    ) -> Result<Option<ReplayFrame>> {
        let camera_id_str = camera_id.to_string();
        let stream_type = stream_type.to_string();
        let target_ms = target_time.timestamp_millis();
        let storage_path = self.storage_path.clone();
        
        // Find the frame closest to target_time
        // First look for a frame at or just before the target time
        let result = self
            .conn
            .call(move |conn| {
                // Get frame metadata - look for frame closest to target time
                let frame_opt: Option<(i64, String, i64, bool, i64)> = conn
                    .query_row(
                        "SELECT id, file_path, file_offset, is_keyframe, timestamp_ms
                         FROM frame_index
                         WHERE camera_id = ?1 AND stream_type = ?2 
                               AND timestamp_ms <= ?3
                         ORDER BY timestamp_ms DESC
                         LIMIT 1",
                        rusqlite::params![camera_id_str, stream_type, target_ms],
                        |row| {
                            let is_kf: i32 = row.get(3)?;
                            Ok((
                                row.get(0)?,          // id
                                row.get(1)?,          // file_path
                                row.get(2)?,          // file_offset
                                is_kf == 1,           // is_keyframe
                                row.get(4)?,          // timestamp_ms
                            ))
                        },
                    )
                    .optional()?;
                
                let Some((_frame_id, file_path, file_offset, is_keyframe, timestamp_ms)) = frame_opt else {
                    return Ok(None);
                };
                
                // Read frame data from file
                let full_path = storage_path.join(&file_path);
                let data = match std::fs::read(&full_path) {
                    Ok(d) => {
                        // If file_offset is 0 and file contains full frame, use it
                        // Otherwise, we'd need to extract the specific frame from offset
                        // For simplicity, assume one frame per file or read from offset
                        if file_offset > 0 && file_offset < d.len() as i64 {
                            // This would need frame size info to slice properly
                            // For now, just return full file contents
                            d
                        } else {
                            d
                        }
                    }
                    Err(_) => return Ok(None),
                };
                
                let timestamp = DateTime::from_timestamp_millis(timestamp_ms).unwrap_or_default();
                
                Ok(Some(ReplayFrame {
                    data,
                    pts: timestamp_ms * 90, // Convert ms to 90kHz ticks
                    is_keyframe,
                    timestamp,
                }))
            })
            .await
            .context("Failed to read frame at time")?;
        
        Ok(result)
    }
}

/// Frame data for replay playback
#[derive(Debug, Clone)]
pub struct ReplayFrame {
    /// Raw frame data (NAL units)
    pub data: Vec<u8>,
    /// Presentation timestamp in 90kHz ticks
    pub pts: i64,
    /// Whether this is a keyframe
    pub is_keyframe: bool,
    /// Wall clock timestamp
    pub timestamp: DateTime<Utc>,
}

/// Convert a database row to StoredFrame
fn row_to_stored_frame(row: &rusqlite::Row) -> StoredFrame {
    let camera_id_str: String = row.get(1).unwrap_or_default();
    let timestamp_ms: i64 = row.get(3).unwrap_or(0);

    StoredFrame {
        id: row.get(0).unwrap_or(0),
        camera_id: Uuid::parse_str(&camera_id_str).unwrap_or_default(),
        stream_type: row.get(2).unwrap_or_default(),
        timestamp: DateTime::from_timestamp_millis(timestamp_ms).unwrap_or_default(),
        sequence_num: row.get(4).unwrap_or(0),
        is_keyframe: row.get::<_, i32>(5).unwrap_or(0) == 1,
        frame_size: row.get::<_, i64>(6).unwrap_or(0) as u64,
        codec: row.get(7).unwrap_or_default(),
    }
}

/// Convert a database row to RecordingSegment
fn row_to_segment(row: &rusqlite::Row) -> rusqlite::Result<RecordingSegment> {
    let camera_id_str: String = row.get(1)?;
    let start_ms: i64 = row.get(3)?;
    let end_ms: Option<i64> = row.get(4)?;

    Ok(RecordingSegment {
        id: row.get(0)?,
        camera_id: Uuid::parse_str(&camera_id_str).unwrap_or_default(),
        stream_type: row.get(2)?,
        start_time: DateTime::from_timestamp_millis(start_ms).unwrap_or_default(),
        end_time: end_ms.and_then(DateTime::from_timestamp_millis),
        frame_count: row.get::<_, i64>(5)? as u64,
        keyframe_count: row.get::<_, i64>(6)? as u64,
        total_bytes: row.get::<_, i64>(7)? as u64,
        file_path: row.get(8)?,
        is_complete: row.get::<_, i32>(9)? == 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_frame_store_creation() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("frames.db");
        let storage_path = temp_dir.path().join("storage");
        std::fs::create_dir_all(&storage_path).unwrap();

        let store = FrameStore::open(&db_path, &storage_path).await.unwrap();

        // Create a segment
        let camera_id = Uuid::new_v4();
        let segment = store
            .create_segment(camera_id, "main", Utc::now())
            .await
            .unwrap();

        assert_eq!(segment.camera_id, camera_id);
        assert_eq!(segment.stream_type, "main");
        assert!(!segment.is_complete);
    }

    #[tokio::test]
    async fn test_store_and_query_frames() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("frames.db");
        let storage_path = temp_dir.path().join("storage");
        std::fs::create_dir_all(&storage_path).unwrap();

        let store = FrameStore::open(&db_path, &storage_path).await.unwrap();
        let camera_id = Uuid::new_v4();
        let now = Utc::now();

        // Create segment
        let segment = store.create_segment(camera_id, "main", now).await.unwrap();

        // Store a keyframe
        let keyframe_time = now;
        store
            .store_frame(
                segment.id,
                camera_id,
                "main",
                keyframe_time,
                1,
                true,
                "h264",
                0,
                1000,
            )
            .await
            .unwrap();

        // Store regular frames
        for i in 2..=5 {
            let frame_time = now + chrono::Duration::milliseconds(i * 33);
            store
                .store_frame(
                    segment.id,
                    camera_id,
                    "main",
                    frame_time,
                    i,
                    false,
                    "h264",
                    i * 1000,
                    500,
                )
                .await
                .unwrap();
        }

        // Query frames
        let end_time = now + chrono::Duration::seconds(1);
        let frames = store
            .query_frames(camera_id, "main", now, end_time, false)
            .await
            .unwrap();

        assert_eq!(frames.len(), 5);
        assert!(frames[0].is_keyframe);
    }

    #[tokio::test]
    async fn test_keyframe_seeking() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("frames.db");
        let storage_path = temp_dir.path().join("storage");
        std::fs::create_dir_all(&storage_path).unwrap();

        let store = FrameStore::open(&db_path, &storage_path).await.unwrap();
        let camera_id = Uuid::new_v4();
        let base_time = Utc::now();

        let segment = store
            .create_segment(camera_id, "main", base_time)
            .await
            .unwrap();

        // Store keyframes at 0s, 2s, 4s
        for i in 0..3 {
            let kf_time = base_time + chrono::Duration::seconds(i * 2);
            store
                .store_frame(
                    segment.id,
                    camera_id,
                    "main",
                    kf_time,
                    i * 60,
                    true,
                    "h264",
                    0,
                    1000,
                )
                .await
                .unwrap();
        }

        // Seek backward from 3s - should find keyframe at 2s
        let target = base_time + chrono::Duration::seconds(3);
        let kf = store
            .find_keyframe(camera_id, "main", target, SeekDirection::Backward)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            kf.timestamp.timestamp(),
            (base_time + chrono::Duration::seconds(2)).timestamp()
        );

        // Seek forward from 3s - should find keyframe at 4s
        let kf = store
            .find_keyframe(camera_id, "main", target, SeekDirection::Forward)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            kf.timestamp.timestamp(),
            (base_time + chrono::Duration::seconds(4)).timestamp()
        );

        // Seek nearest from 2.5s - should find keyframe at 2s (closer)
        let target = base_time + chrono::Duration::milliseconds(2500);
        let kf = store
            .find_keyframe(camera_id, "main", target, SeekDirection::Nearest)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            kf.timestamp.timestamp(),
            (base_time + chrono::Duration::seconds(2)).timestamp()
        );
    }
}
