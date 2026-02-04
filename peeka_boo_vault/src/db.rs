//! Database module for PeekabooVault
//!
//! Moonfire-inspired SQLite schema for recording metadata and indexing.
//! Uses 90kHz ticks for time representation (matching Moonfire's approach).

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio_rusqlite::Connection as AsyncConnection;
use uuid::Uuid;

/// Time unit: 90kHz ticks (same as Moonfire NVR)
/// This matches the MPEG-TS/RTP timestamp frequency.
pub const TICKS_PER_SEC: i64 = 90_000;

/// Convert a timestamp to 90kHz ticks
pub fn datetime_to_ticks(dt: DateTime<Utc>) -> i64 {
    dt.timestamp() * TICKS_PER_SEC + (dt.timestamp_subsec_nanos() as i64 * TICKS_PER_SEC / 1_000_000_000)
}

/// Convert 90kHz ticks to a timestamp
pub fn ticks_to_datetime(ticks: i64) -> DateTime<Utc> {
    let secs = ticks / TICKS_PER_SEC;
    let subsec_ticks = ticks % TICKS_PER_SEC;
    let nanos = (subsec_ticks * 1_000_000_000 / TICKS_PER_SEC) as u32;
    DateTime::from_timestamp(secs, nanos).unwrap_or_default()
}

/// Database schema version
const SCHEMA_VERSION: i32 = 1;

/// SQL schema for the recording database (Moonfire-inspired)
const SCHEMA_SQL: &str = r#"
-- Schema version tracking
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Sample file directories (storage locations)
-- Multiple directories allow spreading across disks
CREATE TABLE IF NOT EXISTS sample_file_dir (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    -- UUID for this directory (used in filenames)
    uuid TEXT NOT NULL UNIQUE,
    -- Last filesystem check time (ticks)
    last_complete_open_id INTEGER
);

-- Camera definitions
CREATE TABLE IF NOT EXISTS camera (
    id INTEGER PRIMARY KEY,
    -- External UUID for API use
    uuid TEXT NOT NULL UNIQUE,
    -- Human-readable name
    name TEXT NOT NULL,
    -- ONVIF device address
    onvif_host TEXT,
    -- Short description
    description TEXT,
    -- Manufacturer from ONVIF
    manufacturer TEXT,
    -- Model from ONVIF
    model TEXT,
    -- Serial number
    serial_number TEXT,
    -- Created timestamp (ticks)
    created_ticks INTEGER NOT NULL,
    -- Config JSON blob
    config TEXT
);

-- Stream definitions (main, sub, etc.)
CREATE TABLE IF NOT EXISTS stream (
    id INTEGER PRIMARY KEY,
    camera_id INTEGER NOT NULL REFERENCES camera(id),
    -- Stream type: 'main', 'sub', 'ext'
    type TEXT NOT NULL,
    -- RTSP URL
    rtsp_url TEXT,
    -- Codec (h264, h265, etc.)
    codec TEXT,
    -- Resolution
    width INTEGER,
    height INTEGER,
    -- Sample file directory for recordings
    sample_file_dir_id INTEGER REFERENCES sample_file_dir(id),
    -- Target segment duration in ticks
    target_segment_duration_ticks INTEGER NOT NULL DEFAULT 5400000, -- 60 seconds
    -- Recording enabled
    recording_enabled INTEGER NOT NULL DEFAULT 0,
    -- Cumulative stats
    total_duration_ticks INTEGER NOT NULL DEFAULT 0,
    total_sample_file_bytes INTEGER NOT NULL DEFAULT 0,
    -- Retention policy (bytes)
    retain_bytes INTEGER NOT NULL DEFAULT 0,
    
    UNIQUE(camera_id, type)
);

-- Recording segments
-- Each row represents one ~60 second segment of recorded video
CREATE TABLE IF NOT EXISTS recording (
    id INTEGER PRIMARY KEY,
    stream_id INTEGER NOT NULL REFERENCES stream(id),
    -- Open ID when this recording was created (for crash recovery)
    open_id INTEGER NOT NULL,
    -- Sample file directory
    sample_file_dir_id INTEGER NOT NULL REFERENCES sample_file_dir(id),
    -- Start time in 90kHz ticks
    start_ticks INTEGER NOT NULL,
    -- Duration in 90kHz ticks
    duration_ticks INTEGER NOT NULL,
    -- File info
    sample_file_bytes INTEGER NOT NULL,
    -- Video sample count
    video_samples INTEGER NOT NULL,
    -- Keyframe count
    video_sync_samples INTEGER NOT NULL,
    -- Video sample index blob (for seeking)
    video_index BLOB,
    -- Run index (for continuous playback detection)
    run_id INTEGER NOT NULL,
    -- Flags (e.g., has audio, incomplete)
    flags INTEGER NOT NULL DEFAULT 0
);

-- Index for time-based queries on recordings
CREATE INDEX IF NOT EXISTS recording_start_idx ON recording(stream_id, start_ticks);

-- Recording playback metadata (sample tables for MP4 assembly)
CREATE TABLE IF NOT EXISTS recording_playback (
    recording_id INTEGER PRIMARY KEY REFERENCES recording(id),
    -- Video sample descriptions (for MP4 stsd box)
    video_sample_entry BLOB
);

-- Garbage collection queue
-- Files scheduled for deletion
CREATE TABLE IF NOT EXISTS garbage (
    id INTEGER PRIMARY KEY,
    sample_file_dir_id INTEGER NOT NULL REFERENCES sample_file_dir(id),
    -- Filename (UUID-based)
    filename TEXT NOT NULL,
    -- When scheduled for deletion (ticks)
    scheduled_ticks INTEGER NOT NULL
);

-- ONVIF events log
CREATE TABLE IF NOT EXISTS event (
    id INTEGER PRIMARY KEY,
    camera_id INTEGER NOT NULL REFERENCES camera(id),
    -- Event time in ticks
    time_ticks INTEGER NOT NULL,
    -- Event type (motion, audio, digital_input, etc.)
    type TEXT NOT NULL,
    -- Event data as JSON
    data TEXT
);

-- Index for event queries
CREATE INDEX IF NOT EXISTS event_time_idx ON event(camera_id, time_ticks);

-- Health check history
CREATE TABLE IF NOT EXISTS health_check (
    id INTEGER PRIMARY KEY,
    camera_id INTEGER NOT NULL REFERENCES camera(id),
    -- Check time in ticks
    time_ticks INTEGER NOT NULL,
    -- Success flag
    success INTEGER NOT NULL,
    -- Latency in milliseconds
    latency_ms INTEGER,
    -- Error message if failed
    error TEXT
);

-- Index for health queries
CREATE INDEX IF NOT EXISTS health_check_time_idx ON health_check(camera_id, time_ticks);
"#;

/// Apply database migrations from one version to another
fn apply_migrations(conn: &rusqlite::Connection, from_version: i32, to_version: i32) -> rusqlite::Result<()> {
    tracing::info!(
        from = from_version,
        to = to_version,
        "Applying database migrations"
    );

    // No migrations defined yet - schema is at version 1
    // When schema changes are needed, add migration functions here:
    //
    // Example:
    // if from_version < 2 && to_version >= 2 {
    //     migration_1_to_2(conn)?;
    // }
    // if from_version < 3 && to_version >= 3 {
    //     migration_2_to_3(conn)?;
    // }

    // Currently at version 1 with no migrations defined
    // If versions differ, just log (error handling is done by caller)
    if from_version != to_version {
        tracing::warn!(
            from = from_version,
            to = to_version,
            "No migrations defined yet, schema versions differ"
        );
    }

    Ok(())
}

// Example migration function (commented out for reference):
// fn migration_1_to_2(conn: &rusqlite::Connection) -> Result<()> {
//     tracing::info!("Running migration 1 -> 2");
//     conn.execute_batch(
//         "ALTER TABLE camera ADD COLUMN new_field TEXT;
//          CREATE INDEX IF NOT EXISTS new_idx ON camera(new_field);"
//     )?;
//     Ok(())
// }

/// Database wrapper for async operations
pub struct Database {
    conn: AsyncConnection,
}

impl Database {
    /// Open or create a database at the given path
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let conn = AsyncConnection::open(&path)
            .await
            .context("Failed to open database")?;

        let db = Self { conn };
        db.initialize().await?;
        Ok(db)
    }

    /// Open an in-memory database (for testing)
    pub async fn open_in_memory() -> Result<Self> {
        let conn = AsyncConnection::open_in_memory()
            .await
            .context("Failed to open in-memory database")?;

        let db = Self { conn };
        db.initialize().await?;
        Ok(db)
    }

    /// Initialize database schema
    async fn initialize(&self) -> Result<()> {
        self.conn
            .call(|conn| {
                // Enable WAL mode for better concurrent performance
                conn.execute_batch("PRAGMA journal_mode=WAL;")?;
                conn.execute_batch("PRAGMA foreign_keys=ON;")?;
                
                // Check schema version
                let version: Option<i32> = conn
                    .query_row(
                        "SELECT value FROM meta WHERE key = 'schema_version'",
                        [],
                        |row| row.get::<_, String>(0),
                    )
                    .ok()
                    .and_then(|s| s.parse().ok());

                if version.is_none() {
                    // Fresh database - create schema
                    conn.execute_batch(SCHEMA_SQL)?;
                    conn.execute(
                        "INSERT INTO meta (key, value) VALUES ('schema_version', ?)",
                        [SCHEMA_VERSION.to_string()],
                    )?;
                    tracing::info!("Created fresh database schema v{}", SCHEMA_VERSION);
                } else if version != Some(SCHEMA_VERSION) {
                    // Run migrations to bring database up to date
                    let current_version = version.unwrap_or(0);

                    // Check if database is newer than we support
                    if current_version > SCHEMA_VERSION {
                        tracing::error!(
                            current = current_version,
                            supported = SCHEMA_VERSION,
                            "Database schema version is newer than supported version"
                        );
                        // Return an error
                        return Err(tokio_rusqlite::Error::Rusqlite(rusqlite::Error::InvalidQuery));
                    }

                    tracing::info!(
                        current_version = current_version,
                        target_version = SCHEMA_VERSION,
                        "Running database migrations"
                    );

                    // Apply migrations
                    apply_migrations(conn, current_version, SCHEMA_VERSION)?;

                    // Update version
                    conn.execute(
                        "UPDATE meta SET value = ? WHERE key = 'schema_version'",
                        [SCHEMA_VERSION.to_string()],
                    )?;

                    tracing::info!("Database migrations completed successfully");
                }

                Ok(())
            })
            .await
            .context("Failed to initialize database schema")
    }

    /// Add or get a sample file directory
    pub async fn ensure_sample_file_dir(&self, path: &str) -> Result<i64> {
        let path = path.to_string();
        self.conn
            .call(move |conn| {
                // Check if exists
                let existing: Option<i64> = conn
                    .query_row(
                        "SELECT id FROM sample_file_dir WHERE path = ?",
                        [&path],
                        |row| row.get(0),
                    )
                    .ok();

                if let Some(id) = existing {
                    return Ok(id);
                }

                // Create new
                let uuid = Uuid::new_v4().to_string();
                conn.execute(
                    "INSERT INTO sample_file_dir (path, uuid) VALUES (?, ?)",
                    [&path, &uuid],
                )?;
                Ok(conn.last_insert_rowid())
            })
            .await
            .context("Failed to ensure sample file directory")
    }

    /// Add a camera to the database
    pub async fn add_camera(&self, camera: &DbCamera) -> Result<i64> {
        let camera = camera.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    r#"
                    INSERT INTO camera (uuid, name, onvif_host, description, manufacturer, model, serial_number, created_ticks, config)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                    "#,
                    rusqlite::params![
                        camera.uuid.to_string(),
                        camera.name,
                        camera.onvif_host,
                        camera.description,
                        camera.manufacturer,
                        camera.model,
                        camera.serial_number,
                        camera.created_ticks,
                        camera.config,
                    ],
                )?;
                Ok(conn.last_insert_rowid())
            })
            .await
            .context("Failed to add camera")
    }

    /// Get a camera by UUID
    pub async fn get_camera_by_uuid(&self, uuid: Uuid) -> Result<Option<DbCamera>> {
        let uuid_str = uuid.to_string();
        self.conn
            .call(move |conn| {
                let result = conn.query_row(
                    r#"
                    SELECT id, uuid, name, onvif_host, description, manufacturer, model, serial_number, created_ticks, config
                    FROM camera WHERE uuid = ?
                    "#,
                    [&uuid_str],
                    |row| {
                        Ok(DbCamera {
                            id: Some(row.get(0)?),
                            uuid: row.get::<_, String>(1)?.parse().unwrap_or_default(),
                            name: row.get(2)?,
                            onvif_host: row.get(3)?,
                            description: row.get(4)?,
                            manufacturer: row.get(5)?,
                            model: row.get(6)?,
                            serial_number: row.get(7)?,
                            created_ticks: row.get(8)?,
                            config: row.get(9)?,
                        })
                    },
                );
                match result {
                    Ok(camera) => Ok(Some(camera)),
                    Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                    Err(e) => Err(e.into()),
                }
            })
            .await
            .context("Failed to get camera")
    }

    /// Add a stream for a camera
    pub async fn add_stream(&self, stream: &DbStream) -> Result<i64> {
        let stream = stream.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    r#"
                    INSERT INTO stream (camera_id, type, rtsp_url, codec, width, height, sample_file_dir_id, recording_enabled)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                    "#,
                    rusqlite::params![
                        stream.camera_id,
                        stream.stream_type,
                        stream.rtsp_url,
                        stream.codec,
                        stream.width,
                        stream.height,
                        stream.sample_file_dir_id,
                        stream.recording_enabled as i32,
                    ],
                )?;
                Ok(conn.last_insert_rowid())
            })
            .await
            .context("Failed to add stream")
    }

    /// Get streams for a camera
    pub async fn get_streams_for_camera(&self, camera_id: i64) -> Result<Vec<DbStream>> {
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT id, camera_id, type, rtsp_url, codec, width, height, sample_file_dir_id, 
                           target_segment_duration_ticks, recording_enabled, total_duration_ticks,
                           total_sample_file_bytes, retain_bytes
                    FROM stream WHERE camera_id = ?
                    "#,
                )?;
                let streams = stmt
                    .query_map([camera_id], |row| {
                        Ok(DbStream {
                            id: Some(row.get(0)?),
                            camera_id: row.get(1)?,
                            stream_type: row.get(2)?,
                            rtsp_url: row.get(3)?,
                            codec: row.get(4)?,
                            width: row.get(5)?,
                            height: row.get(6)?,
                            sample_file_dir_id: row.get(7)?,
                            target_segment_duration_ticks: row.get(8)?,
                            recording_enabled: row.get::<_, i32>(9)? != 0,
                            total_duration_ticks: row.get(10)?,
                            total_sample_file_bytes: row.get(11)?,
                            retain_bytes: row.get(12)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(streams)
            })
            .await
            .context("Failed to get streams")
    }

    /// Insert a recording segment
    pub async fn insert_recording(&self, recording: &DbRecording) -> Result<i64> {
        let recording = recording.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    r#"
                    INSERT INTO recording (stream_id, open_id, sample_file_dir_id, start_ticks, 
                                          duration_ticks, sample_file_bytes, video_samples,
                                          video_sync_samples, video_index, run_id, flags)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    "#,
                    rusqlite::params![
                        recording.stream_id,
                        recording.open_id,
                        recording.sample_file_dir_id,
                        recording.start_ticks,
                        recording.duration_ticks,
                        recording.sample_file_bytes,
                        recording.video_samples,
                        recording.video_sync_samples,
                        recording.video_index,
                        recording.run_id,
                        recording.flags,
                    ],
                )?;
                Ok(conn.last_insert_rowid())
            })
            .await
            .context("Failed to insert recording")
    }

    /// Query recordings by time range
    pub async fn query_recordings(
        &self,
        stream_id: i64,
        start_ticks: i64,
        end_ticks: i64,
    ) -> Result<Vec<DbRecording>> {
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT id, stream_id, open_id, sample_file_dir_id, start_ticks, duration_ticks,
                           sample_file_bytes, video_samples, video_sync_samples, video_index, run_id, flags
                    FROM recording 
                    WHERE stream_id = ? 
                      AND start_ticks < ?
                      AND start_ticks + duration_ticks > ?
                    ORDER BY start_ticks
                    "#,
                )?;
                let recordings = stmt
                    .query_map([stream_id, end_ticks, start_ticks], |row| {
                        Ok(DbRecording {
                            id: Some(row.get(0)?),
                            stream_id: row.get(1)?,
                            open_id: row.get(2)?,
                            sample_file_dir_id: row.get(3)?,
                            start_ticks: row.get(4)?,
                            duration_ticks: row.get(5)?,
                            sample_file_bytes: row.get(6)?,
                            video_samples: row.get(7)?,
                            video_sync_samples: row.get(8)?,
                            video_index: row.get(9)?,
                            run_id: row.get(10)?,
                            flags: row.get(11)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(recordings)
            })
            .await
            .context("Failed to query recordings")
    }
}

/// Camera record for database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbCamera {
    pub id: Option<i64>,
    pub uuid: Uuid,
    pub name: String,
    pub onvif_host: Option<String>,
    pub description: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub serial_number: Option<String>,
    pub created_ticks: i64,
    pub config: Option<String>,
}

/// Stream record for database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbStream {
    pub id: Option<i64>,
    pub camera_id: i64,
    pub stream_type: String,
    pub rtsp_url: Option<String>,
    pub codec: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub sample_file_dir_id: Option<i64>,
    pub target_segment_duration_ticks: i64,
    pub recording_enabled: bool,
    pub total_duration_ticks: i64,
    pub total_sample_file_bytes: i64,
    pub retain_bytes: i64,
}

impl Default for DbStream {
    fn default() -> Self {
        Self {
            id: None,
            camera_id: 0,
            stream_type: "main".to_string(),
            rtsp_url: None,
            codec: None,
            width: None,
            height: None,
            sample_file_dir_id: None,
            target_segment_duration_ticks: 60 * TICKS_PER_SEC, // 60 seconds
            recording_enabled: false,
            total_duration_ticks: 0,
            total_sample_file_bytes: 0,
            retain_bytes: 0,
        }
    }
}

/// Recording segment record for database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbRecording {
    pub id: Option<i64>,
    pub stream_id: i64,
    pub open_id: i64,
    pub sample_file_dir_id: i64,
    pub start_ticks: i64,
    pub duration_ticks: i64,
    pub sample_file_bytes: i64,
    pub video_samples: i64,
    pub video_sync_samples: i64,
    pub video_index: Option<Vec<u8>>,
    pub run_id: i64,
    pub flags: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_database_creation() {
        let db = Database::open_in_memory().await.unwrap();
        let dir_id = db.ensure_sample_file_dir("/tmp/recordings").await.unwrap();
        assert!(dir_id > 0);
    }

    #[tokio::test]
    async fn test_camera_operations() {
        let db = Database::open_in_memory().await.unwrap();
        
        let camera = DbCamera {
            id: None,
            uuid: Uuid::new_v4(),
            name: "Test Camera".to_string(),
            onvif_host: Some("192.168.1.100".to_string()),
            description: None,
            manufacturer: Some("Test".to_string()),
            model: Some("Model 1".to_string()),
            serial_number: None,
            created_ticks: datetime_to_ticks(Utc::now()),
            config: None,
        };
        
        let camera_id = db.add_camera(&camera).await.unwrap();
        assert!(camera_id > 0);
        
        let retrieved = db.get_camera_by_uuid(camera.uuid).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "Test Camera");
    }

    #[test]
    fn test_tick_conversions() {
        let now = Utc::now();
        let ticks = datetime_to_ticks(now);
        let back = ticks_to_datetime(ticks);
        
        // Should be within 1 tick precision
        let diff = (now.timestamp_nanos_opt().unwrap() - back.timestamp_nanos_opt().unwrap()).abs();
        assert!(diff < 1_000_000_000 / TICKS_PER_SEC + 1);
    }
}
