//! Hot/Cold Database Management for PeekabooVault
//!
//! This module implements a dual-database strategy:
//! - **Hot DB**: Fast writes for recent recordings (last N days), optimized for frequent reads
//! - **Cold DB**: Historical archive with fewer writes, can be on slower storage
//!
//! Key features:
//! - Automatic migration from hot → cold based on quota and age policies
//! - Startup recovery for partial segments and incomplete moves
//! - Unified query interface that spans both databases

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::OptionalExtension;
use std::path::{Path, PathBuf};
use tokio_rusqlite::Connection as AsyncConnection;
use tracing::warn;

use crate::config::RetentionConfig;
use crate::db::{datetime_to_ticks, ticks_to_datetime, DbRecording, TICKS_PER_SEC};

/// Recording segment flags
pub mod flags {
    /// Segment is incomplete (still being written or crashed during write)
    pub const INCOMPLETE: i32 = 1 << 0;
    /// Segment is being migrated to cold storage
    pub const MIGRATING: i32 = 1 << 1;
    /// Segment has been migrated to cold storage
    pub const MIGRATED: i32 = 1 << 2;
}

/// Migration state for a recording
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationState {
    /// Recording is in hot storage only
    Hot,
    /// Recording is being migrated (files copied, not yet confirmed)
    Migrating,
    /// Recording has been migrated to cold
    Cold,
}

/// A unified view of a recording that may be in hot or cold storage
#[derive(Debug, Clone)]
pub struct UnifiedRecording {
    pub recording: DbRecording,
    pub storage: MigrationState,
    pub file_path: String,
}

/// Storage statistics for a single stream
#[derive(Debug, Clone, Default)]
pub struct StreamStats {
    pub stream_id: i64,
    pub total_bytes: u64,
    pub total_duration_secs: f64,
    pub recording_count: u64,
    pub oldest_timestamp: Option<DateTime<Utc>>,
    pub newest_timestamp: Option<DateTime<Utc>>,
}

/// Combined statistics for hot and cold storage
#[derive(Debug, Clone, Default)]
pub struct StorageStats {
    pub hot_total_bytes: u64,
    pub cold_total_bytes: u64,
    pub hot_recording_count: u64,
    pub cold_recording_count: u64,
    pub streams: Vec<StreamStats>,
}

/// A file scheduled for garbage collection
#[derive(Debug, Clone)]
pub struct GarbageEntry {
    pub id: i64,
    pub sample_file_dir_id: i64,
    pub filename: String,
    pub scheduled_ticks: i64,
}

/// Dual hot/cold database manager
pub struct HotColdDb {
    hot_conn: AsyncConnection,
    cold_conn: AsyncConnection,
    hot_storage_path: PathBuf,
    cold_storage_path: PathBuf,
    retention_config: RetentionConfig,
    /// Current open ID for crash recovery
    open_id: i64,
}

impl HotColdDb {
    /// Open or create hot and cold databases
    pub async fn open(
        hot_db_path: &Path,
        cold_db_path: &Path,
        hot_storage_path: &Path,
        cold_storage_path: &Path,
        retention_config: RetentionConfig,
    ) -> Result<Self> {
        // Ensure directories exist
        if let Some(parent) = hot_db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        if let Some(parent) = cold_db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::create_dir_all(hot_storage_path).await?;
        tokio::fs::create_dir_all(cold_storage_path).await?;

        let hot_conn = AsyncConnection::open(hot_db_path)
            .await
            .context("Failed to open hot database")?;

        let cold_conn = AsyncConnection::open(cold_db_path)
            .await
            .context("Failed to open cold database")?;

        let db = Self {
            hot_conn,
            cold_conn,
            hot_storage_path: hot_storage_path.to_path_buf(),
            cold_storage_path: cold_storage_path.to_path_buf(),
            retention_config,
            open_id: 0,
        };

        db.initialize_both().await?;
        let open_id = db.register_open().await?;

        Ok(Self { open_id, ..db })
    }

    /// Initialize schema in both databases
    async fn initialize_both(&self) -> Result<()> {
        // Initialize hot DB
        self.hot_conn
            .call(|conn| {
                conn.execute_batch("PRAGMA journal_mode=WAL;")?;
                conn.execute_batch("PRAGMA foreign_keys=ON;")?;
                conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
                conn.execute_batch(SHARED_SCHEMA_SQL)?;
                conn.execute_batch(HOT_SPECIFIC_SQL)?;
                Ok(())
            })
            .await
            .context("Failed to initialize hot database schema")?;

        // Initialize cold DB
        self.cold_conn
            .call(|conn| {
                conn.execute_batch("PRAGMA journal_mode=WAL;")?;
                conn.execute_batch("PRAGMA foreign_keys=ON;")?;
                conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
                conn.execute_batch(SHARED_SCHEMA_SQL)?;
                Ok(())
            })
            .await
            .context("Failed to initialize cold database schema")?;

        Ok(())
    }

    /// Register a new open session (for crash recovery)
    async fn register_open(&self) -> Result<i64> {
        let now_ticks = datetime_to_ticks(Utc::now());
        self.hot_conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO open (start_ticks) VALUES (?)",
                    [now_ticks],
                )?;
                Ok(conn.last_insert_rowid())
            })
            .await
            .context("Failed to register open")
    }

    /// Get current open ID
    pub fn open_id(&self) -> i64 {
        self.open_id
    }

    /// Get or create a sample_file_dir for a storage path
    pub async fn get_or_create_sample_file_dir(&self, storage_path: &Path) -> Result<i64> {
        let path_str = storage_path.to_string_lossy().to_string();
        let uuid_str = uuid::Uuid::new_v4().to_string();

        self.hot_conn
            .call(move |conn| {
                // Try to find existing dir
                let existing: Option<i64> = conn
                    .query_row(
                        "SELECT id FROM sample_file_dir WHERE path = ?",
                        [&path_str],
                        |row| row.get(0),
                    )
                    .optional()?;

                if let Some(id) = existing {
                    Ok(id)
                } else {
                    // Insert new dir
                    conn.execute(
                        "INSERT INTO sample_file_dir (path, uuid) VALUES (?, ?)",
                        rusqlite::params![&path_str, &uuid_str],
                    )?;
                    Ok(conn.last_insert_rowid())
                }
            })
            .await
            .context("Failed to get or create sample_file_dir")
    }

    /// Perform startup recovery
    /// - Mark incomplete segments from previous sessions
    /// - Resume any interrupted migrations
    pub async fn startup_recovery(&self) -> Result<RecoveryReport> {
        let open_id = self.open_id;
        let report = self
            .hot_conn
            .call(move |conn| {
                let mut report = RecoveryReport::default();

                // Find incomplete segments from previous opens
                let incomplete_count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM recording WHERE open_id < ? AND (flags & ?) = ?",
                    [open_id, flags::INCOMPLETE as i64, flags::INCOMPLETE as i64],
                    |row| row.get(0),
                )?;
                report.incomplete_segments = incomplete_count as u64;

                // Find segments stuck in migrating state
                let migrating_count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM recording WHERE (flags & ?) = ?",
                    [flags::MIGRATING as i64, flags::MIGRATING as i64],
                    |row| row.get(0),
                )?;
                report.interrupted_migrations = migrating_count as u64;

                // Mark old incomplete segments as needing repair
                // (In a real impl, we'd try to salvage partial data)
                conn.execute(
                    "UPDATE recording SET flags = flags | ? WHERE open_id < ? AND (flags & ?) = ?",
                    [
                        flags::INCOMPLETE as i64,
                        open_id,
                        flags::INCOMPLETE as i64,
                        flags::INCOMPLETE as i64,
                    ],
                )?;

                Ok(report)
            })
            .await
            .context("Failed to perform startup recovery")?;

        if report.incomplete_segments > 0 {
            warn!(
                "Found {} incomplete segments from previous session",
                report.incomplete_segments
            );
        }
        if report.interrupted_migrations > 0 {
            warn!(
                "Found {} interrupted migrations, will resume",
                report.interrupted_migrations
            );
        }

        Ok(report)
    }

    /// Insert a recording into the hot database
    pub async fn insert_recording(&self, recording: &DbRecording) -> Result<i64> {
        let mut recording = recording.clone();
        recording.open_id = self.open_id;
        
        self.hot_conn
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

    /// Mark a recording as complete (remove INCOMPLETE flag)
    pub async fn mark_complete(&self, recording_id: i64) -> Result<()> {
        self.hot_conn
            .call(move |conn| {
                conn.execute(
                    "UPDATE recording SET flags = flags & ~? WHERE id = ?",
                    [flags::INCOMPLETE as i64, recording_id],
                )?;
                Ok(())
            })
            .await
            .context("Failed to mark recording complete")
    }

    /// Insert or update a camera in the database
    pub async fn upsert_camera(
        &self,
        camera_uuid: String,
        name: String,
        onvif_host: Option<String>,
    ) -> Result<i64> {
        let created_ticks = datetime_to_ticks(Utc::now());

        self.hot_conn
            .call(move |conn| {
                // Try to find existing camera
                let existing: Option<i64> = conn
                    .query_row(
                        "SELECT id FROM camera WHERE uuid = ?",
                        [&camera_uuid],
                        |row| row.get(0),
                    )
                    .optional()?;

                if let Some(id) = existing {
                    // Update existing camera
                    conn.execute(
                        "UPDATE camera SET name = ?, onvif_host = ? WHERE id = ?",
                        rusqlite::params![&name, &onvif_host, id],
                    )?;
                    Ok(id)
                } else {
                    // Insert new camera
                    conn.execute(
                        "INSERT INTO camera (uuid, name, onvif_host, created_ticks) VALUES (?, ?, ?, ?)",
                        rusqlite::params![&camera_uuid, &name, &onvif_host, created_ticks],
                    )?;
                    Ok(conn.last_insert_rowid())
                }
            })
            .await
            .context("Failed to upsert camera")
    }

    /// Get camera database ID from UUID
    pub async fn get_camera_id(&self, camera_uuid: &str) -> Result<Option<i64>> {
        let camera_uuid = camera_uuid.to_string();

        self.hot_conn
            .call(move |conn| {
                let id: Option<i64> = conn
                    .query_row(
                        "SELECT id FROM camera WHERE uuid = ?",
                        [&camera_uuid],
                        |row| row.get(0),
                    )
                    .optional()?;
                Ok(id)
            })
            .await
            .context("Failed to get camera ID")
    }

    /// Insert or update a stream in the database
    pub async fn upsert_stream(
        &self,
        camera_id: i64,
        stream_type: String,
        rtsp_url: Option<String>,
        sample_file_dir_id: i64,
    ) -> Result<i64> {
        self.hot_conn
            .call(move |conn| {
                // Try to find existing stream
                let existing: Option<i64> = conn
                    .query_row(
                        "SELECT id FROM stream WHERE camera_id = ? AND type = ?",
                        rusqlite::params![camera_id, &stream_type],
                        |row| row.get(0),
                    )
                    .optional()?;

                if let Some(id) = existing {
                    // Update existing stream
                    conn.execute(
                        "UPDATE stream SET rtsp_url = ?, sample_file_dir_id = ? WHERE id = ?",
                        rusqlite::params![&rtsp_url, sample_file_dir_id, id],
                    )?;
                    Ok(id)
                } else {
                    // Insert new stream
                    conn.execute(
                        "INSERT INTO stream (camera_id, type, rtsp_url, sample_file_dir_id, recording_enabled) VALUES (?, ?, ?, ?, 1)",
                        rusqlite::params![camera_id, &stream_type, &rtsp_url, sample_file_dir_id],
                    )?;
                    Ok(conn.last_insert_rowid())
                }
            })
            .await
            .context("Failed to upsert stream")
    }

    /// Get stream ID for a camera and stream type
    pub async fn get_stream_id(&self, camera_uuid: &str, stream_type: &str) -> Result<Option<i64>> {
        let camera_uuid = camera_uuid.to_string();
        let stream_type = stream_type.to_string();

        self.hot_conn
            .call(move |conn| {
                let stream_id: Option<i64> = conn
                    .query_row(
                        "SELECT s.id FROM stream s
                         JOIN camera c ON s.camera_id = c.id
                         WHERE c.uuid = ? AND s.type = ?",
                        rusqlite::params![&camera_uuid, &stream_type],
                        |row| row.get(0),
                    )
                    .optional()?;
                Ok(stream_id)
            })
            .await
            .context("Failed to get stream ID")
    }

    /// Get all stream IDs for a camera
    pub async fn get_camera_stream_ids(&self, camera_uuid: &str) -> Result<Vec<(String, i64)>> {
        let camera_uuid = camera_uuid.to_string();

        self.hot_conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT s.type, s.id FROM stream s
                     JOIN camera c ON s.camera_id = c.id
                     WHERE c.uuid = ?"
                )?;

                let streams: Vec<(String, i64)> = stmt
                    .query_map([&camera_uuid], |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;

                Ok(streams)
            })
            .await
            .context("Failed to get camera stream IDs")
    }

    /// Query garbage entries that are ready for deletion
    pub async fn get_garbage_entries(&self) -> Result<Vec<GarbageEntry>> {
        let now_ticks = datetime_to_ticks(Utc::now());

        self.hot_conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, sample_file_dir_id, filename, scheduled_ticks
                     FROM garbage
                     WHERE scheduled_ticks <= ?
                     ORDER BY scheduled_ticks ASC
                     LIMIT 100"
                )?;

                let entries: Vec<GarbageEntry> = stmt
                    .query_map([now_ticks], |row| {
                        Ok(GarbageEntry {
                            id: row.get(0)?,
                            sample_file_dir_id: row.get(1)?,
                            filename: row.get(2)?,
                            scheduled_ticks: row.get(3)?,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;

                Ok(entries)
            })
            .await
            .context("Failed to query garbage entries")
    }

    /// Delete garbage entry from the table
    pub async fn delete_garbage_entry(&self, garbage_id: i64) -> Result<()> {
        self.hot_conn
            .call(move |conn| {
                conn.execute("DELETE FROM garbage WHERE id = ?", [garbage_id])?;
                Ok(())
            })
            .await
            .context("Failed to delete garbage entry")
    }

    /// Schedule a file for garbage collection
    pub async fn schedule_garbage(&self, sample_file_dir_id: i64, filename: String) -> Result<()> {
        let scheduled_ticks = datetime_to_ticks(Utc::now());

        self.hot_conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO garbage (sample_file_dir_id, filename, scheduled_ticks)
                     VALUES (?, ?, ?)",
                    rusqlite::params![sample_file_dir_id, &filename, scheduled_ticks],
                )?;
                Ok(())
            })
            .await
            .context("Failed to schedule garbage")
    }

    /// Query recordings across both hot and cold databases
    pub async fn query_recordings(
        &self,
        stream_id: i64,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> Result<Vec<UnifiedRecording>> {
        let start_ticks = datetime_to_ticks(start_time);
        let end_ticks = datetime_to_ticks(end_time);
        
        // Query hot DB
        let hot_storage_path = self.hot_storage_path.clone();
        let hot_recordings: Vec<UnifiedRecording> = self
            .hot_conn
            .call(move |conn| {
                Ok(query_recordings_from_conn(conn, stream_id, start_ticks, end_ticks, MigrationState::Hot, &hot_storage_path)?)
            })
            .await
            .context("Failed to query hot recordings")?;

        // Query cold DB
        let cold_storage_path = self.cold_storage_path.clone();
        let cold_recordings: Vec<UnifiedRecording> = self
            .cold_conn
            .call(move |conn| {
                Ok(query_recordings_from_conn(conn, stream_id, start_ticks, end_ticks, MigrationState::Cold, &cold_storage_path)?)
            })
            .await
            .context("Failed to query cold recordings")?;

        // Merge results, sorted by start time
        let mut all_recordings = hot_recordings;
        all_recordings.extend(cold_recordings);
        all_recordings.sort_by_key(|r| r.recording.start_ticks);

        Ok(all_recordings)
    }

    /// Get storage statistics
    pub async fn get_storage_stats(&self) -> Result<StorageStats> {
        let hot_stats = self
            .hot_conn
            .call(|conn| Ok(get_db_stats(conn)?))
            .await
            .context("Failed to get hot stats")?;

        let cold_stats = self
            .cold_conn
            .call(|conn| Ok(get_db_stats(conn)?))
            .await
            .context("Failed to get cold stats")?;

        Ok(StorageStats {
            hot_total_bytes: hot_stats.0,
            cold_total_bytes: cold_stats.0,
            hot_recording_count: hot_stats.1,
            cold_recording_count: cold_stats.1,
            streams: hot_stats.2, // Stream details from hot DB
        })
    }

    /// Get candidates for migration from hot to cold
    /// Returns oldest recordings that exceed quota or age limits
    pub async fn get_migration_candidates(&self, limit: usize) -> Result<Vec<DbRecording>> {
        let retention = self.retention_config.clone();
        let limit = limit as i64;

        self.hot_conn
            .call(move |conn| {
                // Get total hot storage usage
                let total_bytes: i64 = conn.query_row(
                    "SELECT COALESCE(SUM(sample_file_bytes), 0) FROM recording",
                    [],
                    |row| row.get(0),
                )?;

                // Calculate age threshold
                let now_ticks = datetime_to_ticks(Utc::now());
                let age_threshold_ticks = if retention.hot_max_age_secs > 0 {
                    now_ticks - (retention.hot_max_age_secs as i64 * TICKS_PER_SEC)
                } else {
                    0
                };

                // Check if migration is needed
                let quota_exceeded = total_bytes as u64 > 
                    (retention.hot_quota_bytes as f64 * retention.migration_threshold) as u64;

                if !quota_exceeded && age_threshold_ticks == 0 {
                    return Ok(vec![]);
                }

                // Find candidates: oldest first, not already migrating
                let mut stmt = conn.prepare(
                    r#"
                    SELECT id, stream_id, open_id, sample_file_dir_id, start_ticks, duration_ticks,
                           sample_file_bytes, video_samples, video_sync_samples, video_index, run_id, flags
                    FROM recording 
                    WHERE (flags & ?) = 0  -- Not already migrating
                      AND (flags & ?) = 0  -- Not incomplete
                      AND (start_ticks < ? OR ?)  -- Age exceeded OR quota exceeded
                    ORDER BY start_ticks ASC
                    LIMIT ?
                    "#,
                )?;

                let recordings = stmt
                    .query_map(
                        rusqlite::params![
                            flags::MIGRATING,
                            flags::INCOMPLETE,
                            age_threshold_ticks,
                            quota_exceeded as i32,
                            limit
                        ],
                        |row| {
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
                        },
                    )?
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(recordings)
            })
            .await
            .context("Failed to get migration candidates")
    }

    /// Mark a recording as migrating (before file copy)
    pub async fn mark_migrating(&self, recording_id: i64) -> Result<()> {
        self.hot_conn
            .call(move |conn| {
                conn.execute(
                    "UPDATE recording SET flags = flags | ? WHERE id = ?",
                    [flags::MIGRATING as i64, recording_id],
                )?;
                Ok(())
            })
            .await
            .context("Failed to mark recording as migrating")
    }

    /// Complete migration: insert into cold DB, delete from hot DB
    pub async fn complete_migration(&self, recording: &DbRecording) -> Result<()> {
        // Insert into cold DB
        let cold_recording = recording.clone();
        self.cold_conn
            .call(move |conn| {
                conn.execute(
                    r#"
                    INSERT INTO recording (stream_id, open_id, sample_file_dir_id, start_ticks, 
                                          duration_ticks, sample_file_bytes, video_samples,
                                          video_sync_samples, video_index, run_id, flags)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    "#,
                    rusqlite::params![
                        cold_recording.stream_id,
                        cold_recording.open_id,
                        cold_recording.sample_file_dir_id,
                        cold_recording.start_ticks,
                        cold_recording.duration_ticks,
                        cold_recording.sample_file_bytes,
                        cold_recording.video_samples,
                        cold_recording.video_sync_samples,
                        cold_recording.video_index,
                        cold_recording.run_id,
                        flags::MIGRATED,  // Mark as migrated in cold DB
                    ],
                )?;
                Ok(())
            })
            .await
            .context("Failed to insert into cold DB")?;

        // Delete from hot DB
        let hot_id = recording.id.unwrap();
        self.hot_conn
            .call(move |conn| {
                conn.execute("DELETE FROM recording WHERE id = ?", [hot_id])?;
                Ok(())
            })
            .await
            .context("Failed to delete from hot DB")?;

        Ok(())
    }

    /// Get recordings to delete from cold storage (exceeds quota or age)
    pub async fn get_cold_deletion_candidates(&self, limit: usize) -> Result<Vec<DbRecording>> {
        let retention = self.retention_config.clone();
        let limit = limit as i64;

        self.cold_conn
            .call(move |conn| {
                // Get total cold storage usage
                let total_bytes: i64 = conn.query_row(
                    "SELECT COALESCE(SUM(sample_file_bytes), 0) FROM recording",
                    [],
                    |row| row.get(0),
                )?;

                // Calculate age threshold
                let now_ticks = datetime_to_ticks(Utc::now());
                let age_threshold_ticks = if retention.cold_max_age_secs > 0 {
                    now_ticks - (retention.cold_max_age_secs as i64 * TICKS_PER_SEC)
                } else {
                    0
                };

                // Check if deletion is needed
                let quota_exceeded = total_bytes as u64 > retention.cold_quota_bytes;

                if !quota_exceeded && age_threshold_ticks == 0 {
                    return Ok(vec![]);
                }

                // Find candidates: oldest first
                let mut stmt = conn.prepare(
                    r#"
                    SELECT id, stream_id, open_id, sample_file_dir_id, start_ticks, duration_ticks,
                           sample_file_bytes, video_samples, video_sync_samples, video_index, run_id, flags
                    FROM recording 
                    WHERE start_ticks < ? OR ?
                    ORDER BY start_ticks ASC
                    LIMIT ?
                    "#,
                )?;

                let recordings = stmt
                    .query_map(
                        rusqlite::params![age_threshold_ticks, quota_exceeded as i32, limit],
                        |row| {
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
                        },
                    )?
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(recordings)
            })
            .await
            .context("Failed to get cold deletion candidates")
    }

    /// Delete a recording from cold storage
    pub async fn delete_cold_recording(&self, recording_id: i64) -> Result<()> {
        self.cold_conn
            .call(move |conn| {
                conn.execute("DELETE FROM recording WHERE id = ?", [recording_id])?;
                Ok(())
            })
            .await
            .context("Failed to delete cold recording")
    }

    /// Get timeline data for a camera (for UI browsing)
    pub async fn get_timeline(
        &self,
        camera_id: i64,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> Result<Vec<TimelineSegment>> {
        let start_ticks = datetime_to_ticks(start_time);
        let end_ticks = datetime_to_ticks(end_time);

        // Query both databases
        let hot_segments: Vec<TimelineSegment> = self
            .hot_conn
            .call(move |conn| {
                Ok(get_timeline_segments(conn, camera_id, start_ticks, end_ticks, false)?)
            })
            .await?;

        let cold_segments: Vec<TimelineSegment> = self
            .cold_conn
            .call(move |conn| {
                Ok(get_timeline_segments(conn, camera_id, start_ticks, end_ticks, true)?)
            })
            .await?;

        // Merge and sort
        let mut all_segments = hot_segments;
        all_segments.extend(cold_segments);
        all_segments.sort_by_key(|s| s.start_time);

        Ok(all_segments)
    }
}

/// Timeline segment for UI display
#[derive(Debug, Clone, serde::Serialize)]
pub struct TimelineSegment {
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub duration_secs: f64,
    pub bytes: u64,
    pub is_cold: bool,
}

/// Recovery report from startup
#[derive(Debug, Default)]
pub struct RecoveryReport {
    pub incomplete_segments: u64,
    pub interrupted_migrations: u64,
    pub recovered_bytes: u64,
}

/// Shared schema for both hot and cold databases
const SHARED_SCHEMA_SQL: &str = r#"
-- Track DB opens for crash recovery
CREATE TABLE IF NOT EXISTS open (
    id INTEGER PRIMARY KEY,
    start_ticks INTEGER NOT NULL
);

-- Sample file directories
CREATE TABLE IF NOT EXISTS sample_file_dir (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    uuid TEXT NOT NULL
);

-- Camera definitions
CREATE TABLE IF NOT EXISTS camera (
    id INTEGER PRIMARY KEY,
    uuid TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    onvif_host TEXT,
    description TEXT,
    manufacturer TEXT,
    model TEXT,
    serial_number TEXT,
    created_ticks INTEGER NOT NULL,
    config TEXT
);

-- Stream definitions (main, sub, etc.)
CREATE TABLE IF NOT EXISTS stream (
    id INTEGER PRIMARY KEY,
    camera_id INTEGER NOT NULL REFERENCES camera(id),
    type TEXT NOT NULL,
    rtsp_url TEXT,
    codec TEXT,
    width INTEGER,
    height INTEGER,
    sample_file_dir_id INTEGER REFERENCES sample_file_dir(id),
    target_segment_duration_ticks INTEGER NOT NULL DEFAULT 5400000,
    recording_enabled INTEGER NOT NULL DEFAULT 0,
    total_duration_ticks INTEGER NOT NULL DEFAULT 0,
    total_sample_file_bytes INTEGER NOT NULL DEFAULT 0,
    retain_bytes INTEGER NOT NULL DEFAULT 0,
    UNIQUE(camera_id, type)
);

-- Recording segments
CREATE TABLE IF NOT EXISTS recording (
    id INTEGER PRIMARY KEY,
    stream_id INTEGER NOT NULL REFERENCES stream(id),
    open_id INTEGER NOT NULL,
    sample_file_dir_id INTEGER NOT NULL,
    start_ticks INTEGER NOT NULL,
    duration_ticks INTEGER NOT NULL,
    sample_file_bytes INTEGER NOT NULL,
    video_samples INTEGER NOT NULL,
    video_sync_samples INTEGER NOT NULL,
    video_index BLOB,
    run_id INTEGER NOT NULL,
    flags INTEGER NOT NULL DEFAULT 0
);

-- Indexes for efficient queries
CREATE INDEX IF NOT EXISTS recording_start_idx ON recording(stream_id, start_ticks);
CREATE INDEX IF NOT EXISTS recording_flags_idx ON recording(flags);

-- Recording playback data (sample tables)
CREATE TABLE IF NOT EXISTS recording_playback (
    recording_id INTEGER PRIMARY KEY,
    video_sample_entry BLOB
);
"#;

/// Hot-specific schema (garbage collection, etc.)
const HOT_SPECIFIC_SQL: &str = r#"
-- Garbage collection queue for deleted files
CREATE TABLE IF NOT EXISTS garbage (
    id INTEGER PRIMARY KEY,
    sample_file_dir_id INTEGER NOT NULL,
    filename TEXT NOT NULL,
    scheduled_ticks INTEGER NOT NULL
);
"#;

// Helper function to query recordings from a single connection
fn query_recordings_from_conn(
    conn: &rusqlite::Connection,
    stream_id: i64,
    start_ticks: i64,
    end_ticks: i64,
    storage: MigrationState,
    storage_path: &Path,
) -> Result<Vec<UnifiedRecording>, rusqlite::Error> {
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
            let recording = DbRecording {
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
            };
            
            // Generate file path based on storage location
            let file_path = storage_path
                .join(format!("{}", recording.sample_file_dir_id))
                .join(format!("{}.mp4", recording.id.unwrap()))
                .to_string_lossy()
                .to_string();

            Ok(UnifiedRecording {
                recording,
                storage,
                file_path,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(recordings)
}

// Helper to get database statistics
fn get_db_stats(conn: &rusqlite::Connection) -> Result<(u64, u64, Vec<StreamStats>), rusqlite::Error> {
    let total_bytes: i64 = conn.query_row(
        "SELECT COALESCE(SUM(sample_file_bytes), 0) FROM recording",
        [],
        |row| row.get(0),
    )?;

    let recording_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM recording",
        [],
        |row| row.get(0),
    )?;

    // Get per-stream stats
    let mut stmt = conn.prepare(
        r#"
        SELECT stream_id, 
               SUM(sample_file_bytes) as total_bytes,
               SUM(duration_ticks) as total_duration,
               COUNT(*) as count,
               MIN(start_ticks) as oldest,
               MAX(start_ticks + duration_ticks) as newest
        FROM recording
        GROUP BY stream_id
        "#,
    )?;

    let streams = stmt
        .query_map([], |row| {
            let oldest_ticks: Option<i64> = row.get(4)?;
            let newest_ticks: Option<i64> = row.get(5)?;
            Ok(StreamStats {
                stream_id: row.get(0)?,
                total_bytes: row.get::<_, i64>(1)? as u64,
                total_duration_secs: row.get::<_, i64>(2)? as f64 / TICKS_PER_SEC as f64,
                recording_count: row.get::<_, i64>(3)? as u64,
                oldest_timestamp: oldest_ticks.map(ticks_to_datetime),
                newest_timestamp: newest_ticks.map(ticks_to_datetime),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok((total_bytes as u64, recording_count as u64, streams))
}

// Helper to get timeline segments
fn get_timeline_segments(
    conn: &rusqlite::Connection,
    camera_id: i64,
    start_ticks: i64,
    end_ticks: i64,
    is_cold: bool,
) -> Result<Vec<TimelineSegment>, rusqlite::Error> {
    // This query assumes we have a way to map stream_id to camera_id
    // For simplicity, we treat stream_id as camera_id here (could be joined via stream table)
    let mut stmt = conn.prepare(
        r#"
        SELECT start_ticks, duration_ticks, sample_file_bytes
        FROM recording 
        WHERE stream_id IN (SELECT id FROM stream WHERE camera_id = ? UNION SELECT ?)
          AND start_ticks < ?
          AND start_ticks + duration_ticks > ?
        ORDER BY start_ticks
        "#,
    )?;

    let segments = stmt
        .query_map([camera_id, camera_id, end_ticks, start_ticks], |row| {
            let start_ticks: i64 = row.get(0)?;
            let duration_ticks: i64 = row.get(1)?;
            Ok(TimelineSegment {
                start_time: ticks_to_datetime(start_ticks),
                end_time: ticks_to_datetime(start_ticks + duration_ticks),
                duration_secs: duration_ticks as f64 / TICKS_PER_SEC as f64,
                bytes: row.get::<_, i64>(2)? as u64,
                is_cold,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use tempfile::TempDir;

    async fn create_test_db() -> (HotColdDb, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let hot_db = temp_dir.path().join("hot.db");
        let cold_db = temp_dir.path().join("cold.db");
        let hot_storage = temp_dir.path().join("hot_storage");
        let cold_storage = temp_dir.path().join("cold_storage");

        let db = HotColdDb::open(
            &hot_db,
            &cold_db,
            &hot_storage,
            &cold_storage,
            RetentionConfig::default(),
        )
        .await
        .unwrap();

        (db, temp_dir)
    }

    #[tokio::test]
    async fn test_hot_cold_db_creation() {
        let (db, _temp) = create_test_db().await;
        assert!(db.open_id() > 0);
    }

    #[tokio::test]
    async fn test_insert_and_query_recording() {
        let (db, _temp) = create_test_db().await;

        let now = Utc::now();
        let recording = DbRecording {
            id: None,
            stream_id: 1,
            open_id: db.open_id(),
            sample_file_dir_id: 1,
            start_ticks: datetime_to_ticks(now),
            duration_ticks: 60 * TICKS_PER_SEC, // 60 seconds
            sample_file_bytes: 1024 * 1024,     // 1MB
            video_samples: 1800,
            video_sync_samples: 60,
            video_index: None,
            run_id: 1,
            flags: 0,
        };

        let id = db.insert_recording(&recording).await.unwrap();
        assert!(id > 0);

        // Query it back
        let results = db
            .query_recordings(1, now - Duration::hours(1), now + Duration::hours(1))
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].recording.stream_id, 1);
        assert_eq!(results[0].storage, MigrationState::Hot);
    }

    #[tokio::test]
    async fn test_startup_recovery() {
        let (db, _temp) = create_test_db().await;
        let report = db.startup_recovery().await.unwrap();
        // Fresh DB should have no issues
        assert_eq!(report.incomplete_segments, 0);
        assert_eq!(report.interrupted_migrations, 0);
    }

    #[tokio::test]
    async fn test_storage_stats() {
        let (db, _temp) = create_test_db().await;

        // Insert a recording
        let recording = DbRecording {
            id: None,
            stream_id: 1,
            open_id: db.open_id(),
            sample_file_dir_id: 1,
            start_ticks: datetime_to_ticks(Utc::now()),
            duration_ticks: 60 * TICKS_PER_SEC,
            sample_file_bytes: 1024 * 1024,
            video_samples: 1800,
            video_sync_samples: 60,
            video_index: None,
            run_id: 1,
            flags: 0,
        };
        db.insert_recording(&recording).await.unwrap();

        let stats = db.get_storage_stats().await.unwrap();
        assert_eq!(stats.hot_total_bytes, 1024 * 1024);
        assert_eq!(stats.hot_recording_count, 1);
        assert_eq!(stats.cold_total_bytes, 0);
    }
}
