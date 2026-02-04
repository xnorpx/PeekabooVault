//! Retention and Migration Job for PeekabooVault
//!
//! This module handles:
//! - Periodic migration of recordings from hot to cold storage
//! - Enforcement of storage quotas and age limits
//! - Garbage collection of deleted files
//!
//! The migration process is crash-safe:
//! 1. Mark recording as "migrating" in hot DB
//! 2. Copy file from hot to cold storage
//! 3. Insert into cold DB + delete from hot DB (atomic-ish)
//! 4. Schedule hot file for garbage collection

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::config::RetentionConfig;
use crate::db::DbRecording;
use crate::hot_cold_db::HotColdDb;

/// Migration job that runs periodically to enforce retention policies
pub struct MigrationJob {
    db: Arc<RwLock<HotColdDb>>,
    hot_storage_path: PathBuf,
    cold_storage_path: PathBuf,
    config: RetentionConfig,
    /// Batch size for migration operations
    batch_size: usize,
}

impl MigrationJob {
    /// Create a new migration job
    pub fn new(
        db: Arc<RwLock<HotColdDb>>,
        hot_storage_path: PathBuf,
        cold_storage_path: PathBuf,
        config: RetentionConfig,
    ) -> Self {
        Self {
            db,
            hot_storage_path,
            cold_storage_path,
            config,
            batch_size: 10,
        }
    }

    /// Run the migration job once
    pub async fn run_once(&self) -> Result<MigrationReport> {
        let mut report = MigrationReport::default();

        // 1. Migrate hot → cold
        let migrated = self.migrate_hot_to_cold().await?;
        report.migrated_recordings = migrated.len() as u64;
        report.migrated_bytes = migrated.iter().map(|r| r.sample_file_bytes as u64).sum();

        // 2. Delete from cold if over quota
        let deleted = self.delete_from_cold().await?;
        report.deleted_recordings = deleted.len() as u64;
        report.deleted_bytes = deleted.iter().map(|r| r.sample_file_bytes as u64).sum();

        // 3. Run garbage collection
        report.garbage_collected = self.run_garbage_collection().await;

        if report.migrated_recordings > 0 || report.deleted_recordings > 0 {
            info!(
                migrated = report.migrated_recordings,
                deleted = report.deleted_recordings,
                migrated_bytes = report.migrated_bytes,
                deleted_bytes = report.deleted_bytes,
                "Migration job completed"
            );
        }

        Ok(report)
    }

    /// Run the migration job in a loop
    pub async fn run_loop(self: Arc<Self>, mut shutdown: tokio::sync::broadcast::Receiver<()>) {
        let interval = Duration::from_secs(self.config.migration_interval_secs);
        info!(
            interval_secs = self.config.migration_interval_secs,
            "Starting migration job loop"
        );

        loop {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {
                    if let Err(e) = self.run_once().await {
                        error!(error = %e, "Migration job failed");
                    }
                }
                _ = shutdown.recv() => {
                    info!("Migration job shutting down");
                    break;
                }
            }
        }
    }

    /// Migrate recordings from hot to cold storage
    async fn migrate_hot_to_cold(&self) -> Result<Vec<DbRecording>> {
        let db = self.db.read().await;
        let candidates = db.get_migration_candidates(self.batch_size).await?;

        if candidates.is_empty() {
            return Ok(vec![]);
        }

        debug!(count = candidates.len(), "Found migration candidates");

        let mut migrated = Vec::new();

        for recording in candidates {
            match self.migrate_single_recording(&db, &recording).await {
                Ok(()) => {
                    migrated.push(recording);
                }
                Err(e) => {
                    warn!(
                        recording_id = recording.id,
                        error = %e,
                        "Failed to migrate recording, will retry later"
                    );
                }
            }
        }

        Ok(migrated)
    }

    /// Migrate a single recording from hot to cold
    async fn migrate_single_recording(
        &self,
        db: &HotColdDb,
        recording: &DbRecording,
    ) -> Result<()> {
        let recording_id = recording.id.context("Recording must have ID")?;

        // Step 1: Mark as migrating
        db.mark_migrating(recording_id).await?;

        // Step 2: Copy file from hot to cold
        let hot_file = self.get_hot_file_path(recording);
        let cold_file = self.get_cold_file_path(recording);

        // Ensure cold directory exists
        if let Some(parent) = cold_file.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Copy the file
        if hot_file.exists() {
            tokio::fs::copy(&hot_file, &cold_file)
                .await
                .with_context(|| {
                    format!("Failed to copy {} to {}", hot_file.display(), cold_file.display())
                })?;
        } else {
            // File doesn't exist - might have been deleted manually
            warn!(
                path = %hot_file.display(),
                "Hot file not found, marking as migrated anyway"
            );
        }

        // Step 3: Complete migration in DB
        db.complete_migration(recording).await?;

        // Step 4: Delete hot file (or schedule for garbage collection)
        if hot_file.exists() && let Err(e) = tokio::fs::remove_file(&hot_file).await {
            warn!(
                path = %hot_file.display(),
                error = %e,
                "Failed to delete hot file after migration"
            );
        }

        debug!(
            recording_id = recording_id,
            bytes = recording.sample_file_bytes,
            "Successfully migrated recording"
        );

        Ok(())
    }

    /// Delete recordings from cold storage that exceed quota/age
    async fn delete_from_cold(&self) -> Result<Vec<DbRecording>> {
        let db = self.db.read().await;
        let candidates = db.get_cold_deletion_candidates(self.batch_size).await?;

        if candidates.is_empty() {
            return Ok(vec![]);
        }

        debug!(count = candidates.len(), "Found cold deletion candidates");

        let mut deleted = Vec::new();

        for recording in candidates {
            match self.delete_cold_recording(&db, &recording).await {
                Ok(()) => {
                    deleted.push(recording);
                }
                Err(e) => {
                    warn!(
                        recording_id = recording.id,
                        error = %e,
                        "Failed to delete cold recording"
                    );
                }
            }
        }

        Ok(deleted)
    }

    /// Delete a single cold recording
    async fn delete_cold_recording(&self, db: &HotColdDb, recording: &DbRecording) -> Result<()> {
        let recording_id = recording.id.context("Recording must have ID")?;

        // Delete the file
        let cold_file = self.get_cold_file_path(recording);
        if cold_file.exists() {
            tokio::fs::remove_file(&cold_file).await.with_context(|| {
                format!("Failed to delete cold file: {}", cold_file.display())
            })?;
        }

        // Delete from database
        db.delete_cold_recording(recording_id).await?;

        debug!(
            recording_id = recording_id,
            bytes = recording.sample_file_bytes,
            "Deleted cold recording"
        );

        Ok(())
    }

    /// Run garbage collection for orphaned files
    async fn run_garbage_collection(&self) -> u64 {
        let db = self.db.read().await;

        // Query garbage entries that are ready for deletion
        let entries = match db.get_garbage_entries().await {
            Ok(entries) => entries,
            Err(e) => {
                warn!(error = %e, "Failed to query garbage entries");
                return 0;
            }
        };

        if entries.is_empty() {
            return 0;
        }

        debug!(count = entries.len(), "Processing garbage collection");

        let mut collected = 0;

        for entry in entries {
            // Construct file path
            let file_path = self
                .hot_storage_path
                .join(format!("{}", entry.sample_file_dir_id))
                .join(&entry.filename);

            // Try to delete the file
            if file_path.exists() {
                match tokio::fs::remove_file(&file_path).await {
                    Ok(()) => {
                        debug!(
                            file = %file_path.display(),
                            "Deleted garbage file"
                        );
                    }
                    Err(e) => {
                        warn!(
                            file = %file_path.display(),
                            error = %e,
                            "Failed to delete garbage file, will retry later"
                        );
                        continue; // Don't remove from table if file deletion failed
                    }
                }
            } else {
                debug!(
                    file = %file_path.display(),
                    "Garbage file already deleted"
                );
            }

            // Remove from garbage table
            match db.delete_garbage_entry(entry.id).await {
                Ok(()) => {
                    collected += 1;
                }
                Err(e) => {
                    warn!(
                        garbage_id = entry.id,
                        error = %e,
                        "Failed to remove garbage entry from database"
                    );
                }
            }
        }

        if collected > 0 {
            info!(collected = collected, "Garbage collection completed");
        }

        collected
    }

    /// Get the path to a recording file in hot storage
    fn get_hot_file_path(&self, recording: &DbRecording) -> PathBuf {
        self.hot_storage_path
            .join(format!("{}", recording.sample_file_dir_id))
            .join(format!("{}.mp4", recording.id.unwrap_or(0)))
    }

    /// Get the path to a recording file in cold storage
    fn get_cold_file_path(&self, recording: &DbRecording) -> PathBuf {
        self.cold_storage_path
            .join(format!("{}", recording.sample_file_dir_id))
            .join(format!("{}.mp4", recording.id.unwrap_or(0)))
    }
}

/// Report from a migration job run
#[derive(Debug, Default)]
pub struct MigrationReport {
    pub migrated_recordings: u64,
    pub migrated_bytes: u64,
    pub deleted_recordings: u64,
    pub deleted_bytes: u64,
    pub garbage_collected: u64,
}

/// Builder for creating a migration job with custom settings
pub struct MigrationJobBuilder {
    db: Option<Arc<RwLock<HotColdDb>>>,
    hot_storage_path: Option<PathBuf>,
    cold_storage_path: Option<PathBuf>,
    config: RetentionConfig,
    batch_size: usize,
}

impl MigrationJobBuilder {
    pub fn new() -> Self {
        Self {
            db: None,
            hot_storage_path: None,
            cold_storage_path: None,
            config: RetentionConfig::default(),
            batch_size: 10,
        }
    }

    pub fn db(mut self, db: Arc<RwLock<HotColdDb>>) -> Self {
        self.db = Some(db);
        self
    }

    pub fn hot_storage_path(mut self, path: PathBuf) -> Self {
        self.hot_storage_path = Some(path);
        self
    }

    pub fn cold_storage_path(mut self, path: PathBuf) -> Self {
        self.cold_storage_path = Some(path);
        self
    }

    pub fn config(mut self, config: RetentionConfig) -> Self {
        self.config = config;
        self
    }

    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = size;
        self
    }

    pub fn build(self) -> Result<MigrationJob> {
        Ok(MigrationJob {
            db: self.db.context("Database is required")?,
            hot_storage_path: self.hot_storage_path.context("Hot storage path is required")?,
            cold_storage_path: self.cold_storage_path.context("Cold storage path is required")?,
            config: self.config,
            batch_size: self.batch_size,
        })
    }
}

impl Default for MigrationJobBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::datetime_to_ticks;
    use crate::db::TICKS_PER_SEC;
    use chrono::Utc;
    use tempfile::TempDir;

    async fn create_test_setup() -> (Arc<RwLock<HotColdDb>>, MigrationJob, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let hot_db = temp_dir.path().join("hot.db");
        let cold_db = temp_dir.path().join("cold.db");
        let hot_storage = temp_dir.path().join("hot_storage");
        let cold_storage = temp_dir.path().join("cold_storage");

        std::fs::create_dir_all(&hot_storage).unwrap();
        std::fs::create_dir_all(&cold_storage).unwrap();

        let db = HotColdDb::open(
            &hot_db,
            &cold_db,
            &hot_storage,
            &cold_storage,
            RetentionConfig {
                hot_quota_bytes: 1024, // Very small for testing
                migration_threshold: 0.5,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let db = Arc::new(RwLock::new(db));

        let job = MigrationJob::new(
            db.clone(),
            hot_storage.clone(),
            cold_storage.clone(),
            RetentionConfig {
                hot_quota_bytes: 1024,
                migration_threshold: 0.5,
                ..Default::default()
            },
        );

        (db, job, temp_dir)
    }

    #[tokio::test]
    async fn test_migration_job_empty() {
        let (_db, job, _temp) = create_test_setup().await;
        let report = job.run_once().await.unwrap();
        assert_eq!(report.migrated_recordings, 0);
        assert_eq!(report.deleted_recordings, 0);
    }

    #[tokio::test]
    async fn test_migration_when_quota_exceeded() {
        let (db, job, temp_dir) = create_test_setup().await;

        // Insert a recording that exceeds quota
        let recording = DbRecording {
            id: None,
            stream_id: 1,
            open_id: db.read().await.open_id(),
            sample_file_dir_id: 1,
            start_ticks: datetime_to_ticks(Utc::now()),
            duration_ticks: 60 * TICKS_PER_SEC,
            sample_file_bytes: 2048, // Exceeds 1024 byte quota
            video_samples: 1800,
            video_sync_samples: 60,
            video_index: None,
            run_id: 1,
            flags: 0,
        };

        let id = db.write().await.insert_recording(&recording).await.unwrap();

        // Create the file
        let hot_dir = temp_dir.path().join("hot_storage").join("1");
        std::fs::create_dir_all(&hot_dir).unwrap();
        std::fs::write(hot_dir.join(format!("{}.mp4", id)), vec![0u8; 2048]).unwrap();

        // Run migration
        let report = job.run_once().await.unwrap();
        assert_eq!(report.migrated_recordings, 1);
        assert_eq!(report.migrated_bytes, 2048);

        // Verify file was moved
        let cold_file = temp_dir.path().join("cold_storage").join("1").join(format!("{}.mp4", id));
        assert!(cold_file.exists());
    }
}
