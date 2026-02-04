//! Integration tests for PeekabooVault
//!
//! These tests verify end-to-end functionality including:
//! - Recording workflow
//! - Database operations
//! - Storage management
//! - Migration jobs

use anyhow::Result;
use chrono::Utc;
use peeka_boo_vault::{
    Config, HotColdDb, MigrationJob, RecorderConfig,
};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::RwLock;

/// Helper to create a test environment
struct TestEnv {
    _temp_dir: TempDir,
    hot_db_path: PathBuf,
    cold_db_path: PathBuf,
    hot_storage: PathBuf,
    cold_storage: PathBuf,
    db: Arc<RwLock<HotColdDb>>,
}

impl TestEnv {
    async fn new() -> Result<Self> {
        let temp_dir = TempDir::new()?;
        let hot_db_path = temp_dir.path().join("hot.db");
        let cold_db_path = temp_dir.path().join("cold.db");
        let hot_storage = temp_dir.path().join("hot_storage");
        let cold_storage = temp_dir.path().join("cold_storage");

        tokio::fs::create_dir_all(&hot_storage).await?;
        tokio::fs::create_dir_all(&cold_storage).await?;

        let db = HotColdDb::open(
            &hot_db_path,
            &cold_db_path,
            &hot_storage,
            &cold_storage,
            peeka_boo_vault::config::RetentionConfig {
                hot_quota_bytes: 10 * 1024 * 1024, // 10MB for testing
                cold_quota_bytes: 100 * 1024 * 1024, // 100MB for testing
                hot_max_age_secs: 86400, // 1 day
                cold_max_age_secs: 0, // unlimited
                migration_interval_secs: 60,
                migration_threshold: 0.8,
            },
        )
        .await?;

        Ok(Self {
            _temp_dir: temp_dir,
            hot_db_path,
            cold_db_path,
            hot_storage,
            cold_storage,
            db: Arc::new(RwLock::new(db)),
        })
    }
}

#[tokio::test]
async fn test_database_creation() -> Result<()> {
    let env = TestEnv::new().await?;

    // Verify databases were created
    assert!(env.hot_db_path.exists());
    assert!(env.cold_db_path.exists());
    assert!(env.hot_storage.exists());
    assert!(env.cold_storage.exists());

    Ok(())
}

#[tokio::test]
async fn test_camera_and_stream_management() -> Result<()> {
    let env = TestEnv::new().await?;
    let db = env.db.read().await;

    // Create a camera
    let camera_id = db.upsert_camera(
        uuid::Uuid::new_v4().to_string(),
        "Test Camera".to_string(),
        Some("192.168.1.100".to_string()),
    ).await?;

    assert!(camera_id > 0);

    // Get sample file dir
    let sample_file_dir_id = db.get_or_create_sample_file_dir(&env.hot_storage).await?;

    // Create a stream
    let stream_id = db.upsert_stream(
        camera_id,
        "main".to_string(),
        Some("rtsp://192.168.1.100:554/stream1".to_string()),
        sample_file_dir_id,
    ).await?;

    assert!(stream_id > 0);

    // Retrieve stream
    let camera_uuid = uuid::Uuid::new_v4().to_string();
    let retrieved_stream_id = db.get_stream_id(&camera_uuid, "main").await?;
    assert!(retrieved_stream_id.is_none()); // Different UUID, should not find

    Ok(())
}

#[tokio::test]
async fn test_recording_workflow() -> Result<()> {
    let env = TestEnv::new().await?;
    let db = env.db.write().await;

    // Create camera and stream
    let camera_uuid = uuid::Uuid::new_v4().to_string();
    let camera_id = db.upsert_camera(
        camera_uuid.clone(),
        "Test Camera".to_string(),
        None,
    ).await?;

    let sample_file_dir_id = db.get_or_create_sample_file_dir(&env.hot_storage).await?;
    let stream_id = db.upsert_stream(
        camera_id,
        "main".to_string(),
        Some("rtsp://test".to_string()),
        sample_file_dir_id,
    ).await?;

    // Create a recording
    let recording = peeka_boo_vault::db::DbRecording {
        id: None,
        stream_id,
        open_id: db.open_id(),
        sample_file_dir_id,
        start_ticks: peeka_boo_vault::db::datetime_to_ticks(Utc::now()),
        duration_ticks: 60 * peeka_boo_vault::db::TICKS_PER_SEC, // 60 seconds
        sample_file_bytes: 1024 * 1024, // 1MB
        video_samples: 1800,
        video_sync_samples: 60,
        video_index: None,
        run_id: 1,
        flags: 0,
    };

    let recording_id = db.insert_recording(&recording).await?;
    assert!(recording_id > 0);

    // Mark as complete
    db.mark_complete(recording_id).await?;

    // Query recordings
    let recordings = db.query_recordings(
        stream_id,
        Utc::now() - chrono::Duration::hours(1),
        Utc::now() + chrono::Duration::hours(1),
    ).await?;

    assert_eq!(recordings.len(), 1);
    assert_eq!(recordings[0].recording.id, Some(recording_id));

    Ok(())
}

#[tokio::test]
async fn test_garbage_collection() -> Result<()> {
    let env = TestEnv::new().await?;
    let db = env.db.read().await;

    let sample_file_dir_id = db.get_or_create_sample_file_dir(&env.hot_storage).await?;

    // Schedule a file for garbage collection
    db.schedule_garbage(sample_file_dir_id, "test_file.mp4".to_string()).await?;

    // Query garbage entries
    let entries = db.get_garbage_entries().await?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].filename, "test_file.mp4");

    // Delete garbage entry
    db.delete_garbage_entry(entries[0].id).await?;

    // Verify deletion
    let entries_after = db.get_garbage_entries().await?;
    assert_eq!(entries_after.len(), 0);

    Ok(())
}

#[tokio::test]
async fn test_migration_job_empty() -> Result<()> {
    let env = TestEnv::new().await?;

    let job = MigrationJob::new(
        env.db.clone(),
        env.hot_storage.clone(),
        env.cold_storage.clone(),
        peeka_boo_vault::config::RetentionConfig::default(),
    );

    // Run migration on empty database
    let report = job.run_once().await?;

    assert_eq!(report.migrated_recordings, 0);
    assert_eq!(report.deleted_recordings, 0);
    assert_eq!(report.garbage_collected, 0);

    Ok(())
}

#[tokio::test]
async fn test_video_index_encoding() -> Result<()> {
    // Test video index encoding/decoding
    // Format: 1 byte flags + 4 bytes duration_ticks + 4 bytes size = 9 bytes per frame
    let mut index_data = Vec::new();

    // Frame 1: keyframe, 0 ticks duration, 100 bytes
    index_data.push(1u8); // flags: keyframe
    index_data.extend_from_slice(&0i32.to_le_bytes()); // duration_ticks
    index_data.extend_from_slice(&100u32.to_le_bytes()); // size

    // Frame 2: not keyframe, 3000 ticks duration, 200 bytes
    index_data.push(0u8); // flags: not keyframe
    index_data.extend_from_slice(&3000i32.to_le_bytes()); // duration_ticks
    index_data.extend_from_slice(&200u32.to_le_bytes()); // size

    let decoded = peeka_boo_vault::recorder::decode_video_index(&index_data)?;

    assert_eq!(decoded.len(), 2);
    assert_eq!(decoded[0].is_keyframe, true);
    assert_eq!(decoded[0].size, 100);
    assert_eq!(decoded[0].duration_ticks, 0);
    assert_eq!(decoded[1].is_keyframe, false);
    assert_eq!(decoded[1].size, 200);
    assert_eq!(decoded[1].duration_ticks, 3000);

    Ok(())
}

#[tokio::test]
async fn test_config_serialization() -> Result<()> {
    let config = Config::default();

    // Test that config can be serialized to TOML
    let toml_string = toml::to_string(&config)?;
    assert!(toml_string.contains("[server]"));
    assert!(toml_string.contains("[storage]"));
    assert!(toml_string.contains("[discovery]"));
    assert!(toml_string.contains("[webrtc]"));

    // Test deserialization
    let _deserialized: Config = toml::from_str(&toml_string)?;

    Ok(())
}

#[tokio::test]
async fn test_sample_file_dir_management() -> Result<()> {
    let env = TestEnv::new().await?;
    let db = env.db.read().await;

    // Create sample file dir
    let dir_id = db.get_or_create_sample_file_dir(&env.hot_storage).await?;
    assert!(dir_id > 0);

    // Getting same dir again should return same ID
    let dir_id_2 = db.get_or_create_sample_file_dir(&env.hot_storage).await?;
    assert_eq!(dir_id, dir_id_2);

    Ok(())
}

#[tokio::test]
async fn test_recorder_config_defaults() {
    let config = RecorderConfig::default();

    assert_eq!(config.segment_duration.as_secs(), 60);
    assert_eq!(config.max_segment_bytes, 100 * 1024 * 1024); // 100MB
}

#[tokio::test]
async fn test_retention_config_defaults() {
    use peeka_boo_vault::config::RetentionConfig;

    let config = RetentionConfig::default();

    assert_eq!(config.hot_quota_bytes, 50 * 1024 * 1024 * 1024); // 50GB
    assert_eq!(config.cold_quota_bytes, 500 * 1024 * 1024 * 1024); // 500GB
    assert_eq!(config.hot_max_age_secs, 7 * 24 * 60 * 60); // 7 days
    assert_eq!(config.migration_threshold, 0.8);
}
