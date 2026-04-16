use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use eyre::Context;
use metrics::{counter, gauge};
use s3::creds::Credentials;
use s3::{Bucket, Region};
use sqlx::SqlitePool;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::audit::BackupRecord;
use crate::state::AppState;

// ── S3Config ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct S3Config {
    pub endpoint: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    pub region: String,
    pub path_style: bool,
}

impl S3Config {
    /// Construct directly (used by tests and from_env).
    pub fn new(
        endpoint: String,
        bucket: String,
        access_key: String,
        secret_key: String,
        region: Option<String>,
        path_style: Option<bool>,
    ) -> Self {
        Self {
            endpoint,
            bucket,
            access_key,
            secret_key,
            region: region.unwrap_or_else(|| "us-east-1".into()),
            path_style: path_style.unwrap_or(true),
        }
    }

    /// Parse from environment variables. Returns None if any required var is missing.
    pub fn from_env() -> Option<Self> {
        Some(Self::new(
            std::env::var("SUBSTRUKT_S3_ENDPOINT").ok()?,
            std::env::var("SUBSTRUKT_S3_BUCKET").ok()?,
            std::env::var("SUBSTRUKT_S3_ACCESS_KEY").ok()?,
            std::env::var("SUBSTRUKT_S3_SECRET_KEY").ok()?,
            std::env::var("SUBSTRUKT_S3_REGION").ok(),
            std::env::var("SUBSTRUKT_S3_PATH_STYLE")
                .ok()
                .map(|v| v != "false"),
        ))
    }
}

// ── S3 client helpers ───────────────────────────────────────

fn create_bucket(config: &S3Config) -> eyre::Result<Box<Bucket>> {
    let region = Region::Custom {
        region: config.region.clone(),
        endpoint: config.endpoint.clone(),
    };
    let credentials = Credentials::new(
        Some(&config.access_key),
        Some(&config.secret_key),
        None,
        None,
        None,
    )
    .map_err(|e| eyre::eyre!("Failed to create S3 credentials: {e}"))?;

    let bucket = if config.path_style {
        Bucket::new(&config.bucket, region, credentials)
            .map_err(|e| eyre::eyre!("Failed to create S3 bucket handle: {e}"))?
            .with_path_style()
    } else {
        Bucket::new(&config.bucket, region, credentials)
            .map_err(|e| eyre::eyre!("Failed to create S3 bucket handle: {e}"))?
    };
    Ok(bucket)
}

async fn upload_archive(bucket: &Bucket, key: &str, path: &Path) -> eyre::Result<()> {
    let mut file = tokio::fs::File::open(path)
        .await
        .wrap_err("Failed to open archive for upload")?;
    let response = bucket
        .put_object_stream(&mut file, key)
        .await
        .map_err(|e| eyre::eyre!("S3 upload failed: {e}"))?;
    if response.status_code() >= 300 {
        eyre::bail!("S3 upload returned status {}", response.status_code());
    }
    Ok(())
}

async fn list_backups(bucket: &Bucket) -> eyre::Result<Vec<String>> {
    let results = bucket
        .list("backups/".to_string(), None)
        .await
        .map_err(|e| eyre::eyre!("S3 list failed: {e}"))?;
    let mut keys: Vec<String> = results
        .into_iter()
        .flat_map(|r| r.contents)
        .map(|o| o.key)
        .collect();
    keys.sort();
    Ok(keys)
}

async fn delete_backup(bucket: &Bucket, key: &str) -> eyre::Result<()> {
    let response = bucket
        .delete_object(key)
        .await
        .map_err(|e| eyre::eyre!("S3 delete failed: {e}"))?;
    if response.status_code() >= 300 && response.status_code() != 404 {
        eyre::bail!("S3 delete returned status {}", response.status_code());
    }
    Ok(())
}

async fn enforce_retention(bucket: &Bucket, retention_count: i64) -> eyre::Result<usize> {
    let keys = list_backups(bucket).await?;
    let count = keys.len() as i64;
    if count <= retention_count {
        return Ok(0);
    }
    let to_delete = count - retention_count;
    let mut deleted = 0;
    for key in keys.iter().take(to_delete as usize) {
        if let Err(e) = delete_backup(bucket, key).await {
            tracing::warn!("Failed to delete old backup {key}: {e}");
        } else {
            deleted += 1;
        }
    }
    Ok(deleted)
}

// ── Temp file cleanup ───────────────────────────────────────

struct TempCleanup {
    paths: Vec<PathBuf>,
}

impl Drop for TempCleanup {
    fn drop(&mut self) {
        for path in &self.paths {
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(path);
            } else {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

// ── Backup lock guard ───────────────────────────────────────

struct BackupLockGuard<'a> {
    flag: &'a AtomicBool,
}

impl Drop for BackupLockGuard<'_> {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

// ── Helpers ─────────────────────────────────────────────────

fn should_exclude(path: &Path, data_dir: &Path) -> bool {
    let parent = match path.parent() {
        Some(p) => p,
        None => return false,
    };
    if parent != data_dir {
        return false;
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some("db" | "db-wal" | "db-shm") => true,
        _ => {
            // Also check for compound extensions like foo.db-wal
            let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
            filename.ends_with(".db-wal") || filename.ends_with(".db-shm")
        }
    }
}

pub fn is_backup_stuck(record: &BackupRecord) -> bool {
    if record.status != "running" {
        return false;
    }
    let Ok(started) = chrono::DateTime::parse_from_rfc3339(&record.started_at) else {
        return false;
    };
    let elapsed = chrono::Utc::now().signed_duration_since(started);
    elapsed > chrono::Duration::hours(1)
}

// ── Archive creation ────────────────────────────────────────

pub async fn create_archive(
    data_dir: &Path,
    main_pool: &SqlitePool,
    audit_pool: &SqlitePool,
) -> eyre::Result<(PathBuf, serde_json::Value)> {
    let temp_id = uuid::Uuid::new_v4();
    let temp_dir = std::env::temp_dir().join(format!("substrukt-backup-{temp_id}"));
    std::fs::create_dir_all(&temp_dir)?;
    let _cleanup = TempCleanup {
        paths: vec![temp_dir.clone()],
    };

    let archive_path = std::env::temp_dir().join(format!("substrukt-backup-{temp_id}.tar.gz"));

    // Snapshot databases via VACUUM INTO
    let main_db_snapshot = temp_dir.join("substrukt.db");
    let audit_db_snapshot = temp_dir.join("audit.db");

    sqlx::query(sqlx::AssertSqlSafe(format!(
        "VACUUM INTO '{}'",
        main_db_snapshot.display()
    )))
    .execute(main_pool)
    .await
    .wrap_err("Failed to snapshot main database")?;

    sqlx::query(sqlx::AssertSqlSafe(format!(
        "VACUUM INTO '{}'",
        audit_db_snapshot.display()
    )))
    .execute(audit_pool)
    .await
    .wrap_err("Failed to snapshot audit database")?;

    // Build tar.gz
    let archive_file = std::fs::File::create(&archive_path)?;
    let enc = flate2::write::GzEncoder::new(archive_file, flate2::Compression::default());
    let mut tar = tar::Builder::new(enc);

    let mut total_files: u64 = 0;
    let mut total_size_bytes: u64 = 0;
    let mut data_dir_entries: Vec<String> = Vec::new();

    // Collect top-level directory names
    if let Ok(entries) = std::fs::read_dir(data_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                let path = entry.path();
                if path.is_dir() {
                    data_dir_entries.push(name.to_string());
                }
            }
        }
    }
    data_dir_entries.sort();

    // Walk data directory, add all files (excluding DB files at root)
    for entry in walkdir::WalkDir::new(data_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path == data_dir {
            continue;
        }
        if should_exclude(path, data_dir) {
            continue;
        }
        let rel_path = path.strip_prefix(data_dir).unwrap_or(path);
        if path.is_file() {
            let metadata = std::fs::metadata(path)?;
            total_files += 1;
            total_size_bytes += metadata.len();
            tar.append_path_with_name(path, rel_path)?;
        } else if path.is_dir() {
            tar.append_dir(rel_path, path)?;
        }
    }

    // Add database snapshots
    let main_meta = std::fs::metadata(&main_db_snapshot)?;
    total_files += 1;
    total_size_bytes += main_meta.len();
    tar.append_path_with_name(&main_db_snapshot, "substrukt.db")?;

    let audit_meta = std::fs::metadata(&audit_db_snapshot)?;
    total_files += 1;
    total_size_bytes += audit_meta.len();
    tar.append_path_with_name(&audit_db_snapshot, "audit.db")?;

    // Build manifest
    let manifest = serde_json::json!({
        "version": 1,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "substrukt_version": env!("CARGO_PKG_VERSION"),
        "data_dir_entries": data_dir_entries,
        "databases": ["substrukt.db", "audit.db"],
        "total_files": total_files + 1, // +1 for manifest itself
        "total_size_bytes": total_size_bytes,
    });

    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    let mut header = tar::Header::new_gnu();
    header.set_size(manifest_bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append_data(&mut header, "manifest.json", &manifest_bytes[..])?;

    tar.into_inner()?.finish()?;

    Ok((archive_path, manifest))
}

// ── Run backup ──────────────────────────────────────────────

pub async fn run_backup(state: &AppState, s3_config: &S3Config, trigger_source: &str) {
    let start = std::time::Instant::now();

    // Acquire lock
    if state
        .backup_running
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        if trigger_source == "scheduled" {
            tracing::info!("Skipping scheduled backup: another backup is already running");
        } else {
            tracing::warn!("Cannot start manual backup: another backup is already running");
        }
        return;
    }
    let _lock = BackupLockGuard {
        flag: &state.backup_running,
    };

    // Start backup record
    let record_id = match state.audit.start_backup_record(trigger_source).await {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("Failed to create backup record: {e}");
            counter!("substrukt_backup_attempts_total", "status" => "failed").increment(1);
            return;
        }
    };

    state.audit.log(
        "system",
        "backup_started",
        "backup",
        &record_id.to_string(),
        Some(&serde_json::json!({"trigger": trigger_source}).to_string()),
    );

    // Create archive
    let (archive_path, manifest) =
        match create_archive(&state.config.data_dir, &state.pool, state.audit.pool_ref()).await {
            Ok(result) => result,
            Err(e) => {
                let err_msg = format!("Archive creation failed: {e}");
                tracing::error!("{err_msg}");
                let _ = state.audit.fail_backup_record(record_id, &err_msg).await;
                state.audit.log(
                    "system",
                    "backup_failed",
                    "backup",
                    &record_id.to_string(),
                    Some(&serde_json::json!({"error": err_msg}).to_string()),
                );
                counter!("substrukt_backup_attempts_total", "status" => "failed").increment(1);
                return;
            }
        };

    // Check archive size (fail if > 4GB)
    let archive_size = match std::fs::metadata(&archive_path) {
        Ok(m) => m.len(),
        Err(e) => {
            let err_msg = format!("Failed to read archive size: {e}");
            tracing::error!("{err_msg}");
            let _ = std::fs::remove_file(&archive_path);
            let _ = state.audit.fail_backup_record(record_id, &err_msg).await;
            state.audit.log(
                "system",
                "backup_failed",
                "backup",
                &record_id.to_string(),
                Some(&serde_json::json!({"error": err_msg}).to_string()),
            );
            counter!("substrukt_backup_attempts_total", "status" => "failed").increment(1);
            return;
        }
    };

    if archive_size > 4_294_967_296 {
        let err_msg =
            "Archive too large (>4GB). Reduce data directory size or use external backup tooling.";
        tracing::error!("{err_msg}");
        let _ = std::fs::remove_file(&archive_path);
        let _ = state.audit.fail_backup_record(record_id, err_msg).await;
        state.audit.log(
            "system",
            "backup_failed",
            "backup",
            &record_id.to_string(),
            Some(&serde_json::json!({"error": err_msg}).to_string()),
        );
        counter!("substrukt_backup_attempts_total", "status" => "failed").increment(1);
        return;
    }

    // Create bucket handle
    let bucket = match create_bucket(s3_config) {
        Ok(b) => b,
        Err(e) => {
            let err_msg = format!("Failed to create S3 bucket handle: {e}");
            tracing::error!("{err_msg}");
            let _ = std::fs::remove_file(&archive_path);
            let _ = state.audit.fail_backup_record(record_id, &err_msg).await;
            state.audit.log(
                "system",
                "backup_failed",
                "backup",
                &record_id.to_string(),
                Some(&serde_json::json!({"error": err_msg}).to_string()),
            );
            counter!("substrukt_backup_attempts_total", "status" => "failed").increment(1);
            return;
        }
    };

    // Generate S3 key
    let s3_key = format!(
        "backups/{}.tar.gz",
        chrono::Utc::now().format("%Y-%m-%dT%H-%M-%SZ")
    );

    // Upload
    if let Err(e) = upload_archive(&bucket, &s3_key, &archive_path).await {
        let err_msg = format!("S3 upload failed: {e}");
        tracing::error!("{err_msg}");
        let _ = std::fs::remove_file(&archive_path);
        let _ = state.audit.fail_backup_record(record_id, &err_msg).await;
        state.audit.log(
            "system",
            "backup_failed",
            "backup",
            &record_id.to_string(),
            Some(&serde_json::json!({"error": err_msg}).to_string()),
        );
        counter!("substrukt_backup_attempts_total", "status" => "failed").increment(1);
        return;
    }

    // Success
    let manifest_str = serde_json::to_string(&manifest).unwrap_or_default();
    if let Err(e) = state
        .audit
        .complete_backup_record(record_id, archive_size as i64, &s3_key, &manifest_str)
        .await
    {
        tracing::error!("Failed to complete backup record: {e}");
    }

    let duration = start.elapsed();
    tracing::info!(
        "Backup completed: {} ({} bytes, {:.1}s)",
        s3_key,
        archive_size,
        duration.as_secs_f64()
    );

    state.audit.log(
        "system",
        "backup_completed",
        "backup",
        &record_id.to_string(),
        Some(&serde_json::json!({"size_bytes": archive_size, "s3_key": s3_key}).to_string()),
    );

    gauge!("substrukt_backup_last_success_timestamp").set(chrono::Utc::now().timestamp() as f64);
    gauge!("substrukt_backup_last_duration_seconds").set(duration.as_secs_f64());
    counter!("substrukt_backup_attempts_total", "status" => "success").increment(1);

    // Retention cleanup
    let retention_count = match state.audit.get_backup_config().await {
        Ok(config) => config.retention_count,
        Err(e) => {
            tracing::warn!("Failed to read backup config for retention: {e}");
            7 // default
        }
    };
    match enforce_retention(&bucket, retention_count).await {
        Ok(deleted) if deleted > 0 => {
            tracing::info!("Deleted {deleted} old backups (retention: {retention_count})");
        }
        Err(e) => {
            tracing::warn!("Retention cleanup failed: {e}");
        }
        _ => {}
    }

    // Prune history
    if let Err(e) = state.audit.prune_backup_history(50).await {
        tracing::warn!("Failed to prune backup history: {e}");
    }

    // Cleanup archive
    let _ = std::fs::remove_file(&archive_path);
}

// ── Schedule calculation ────────────────────────────────────

pub fn calculate_next_backup_delay(
    last_success: Option<&BackupRecord>,
    frequency_hours: i64,
) -> std::time::Duration {
    let Some(record) = last_success else {
        return std::time::Duration::ZERO;
    };
    let Ok(started) = chrono::DateTime::parse_from_rfc3339(&record.started_at) else {
        return std::time::Duration::ZERO;
    };
    let started_utc = started.with_timezone(&chrono::Utc);
    let next = started_utc + chrono::Duration::hours(frequency_hours);
    let now = chrono::Utc::now();
    if next <= now {
        std::time::Duration::ZERO
    } else {
        let remaining = next - now;
        remaining.to_std().unwrap_or(std::time::Duration::ZERO)
    }
}

// ── Background task ─────────────────────────────────────────

pub fn spawn_backup_task(
    state: AppState,
    s3_config: S3Config,
    mut trigger_rx: mpsc::Receiver<()>,
    cancel_token: CancellationToken,
) {
    tokio::spawn(async move {
        let mut first_iteration = true;

        loop {
            // Read config from DB
            let config = match state.audit.get_backup_config().await {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Failed to read backup config: {e}");
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => continue,
                        _ = cancel_token.cancelled() => return,
                    }
                }
            };

            // On first iteration, check for stuck backups
            if first_iteration {
                first_iteration = false;
                if let Ok(Some(latest)) = state.audit.latest_backup().await
                    && is_backup_stuck(&latest)
                {
                    tracing::warn!(
                        "Detected stuck backup (id={}), marking as failed",
                        latest.id
                    );
                    let _ = state
                        .audit
                        .fail_backup_record(latest.id, "Server restarted during backup")
                        .await;
                    state.backup_running.store(false, Ordering::SeqCst);
                }
            }

            if !config.enabled {
                // Wait for manual trigger or shutdown
                tokio::select! {
                    _ = trigger_rx.recv() => {
                        run_backup(&state, &s3_config, "manual").await;
                    }
                    _ = cancel_token.cancelled() => return,
                }
            } else {
                // Compute delay until next scheduled backup
                let last_success = state.audit.last_successful_backup().await.ok().flatten();
                let delay =
                    calculate_next_backup_delay(last_success.as_ref(), config.frequency_hours);

                tokio::select! {
                    _ = tokio::time::sleep(delay) => {
                        run_backup(&state, &s3_config, "scheduled").await;
                    }
                    _ = trigger_rx.recv() => {
                        run_backup(&state, &s3_config, "manual").await;
                    }
                    _ = cancel_token.cancelled() => return,
                }
            }
        }
    });
}

// ── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_s3_config_new_all_values() {
        let config = S3Config::new(
            "http://localhost:9000".into(),
            "test-bucket".into(),
            "access".into(),
            "secret".into(),
            Some("eu-west-1".into()),
            Some(false),
        );
        assert_eq!(config.endpoint, "http://localhost:9000");
        assert_eq!(config.bucket, "test-bucket");
        assert_eq!(config.access_key, "access");
        assert_eq!(config.secret_key, "secret");
        assert_eq!(config.region, "eu-west-1");
        assert!(!config.path_style);
    }

    #[test]
    fn test_s3_config_new_defaults() {
        let config = S3Config::new(
            "http://localhost:9000".into(),
            "bucket".into(),
            "key".into(),
            "secret".into(),
            None,
            None,
        );
        assert_eq!(config.region, "us-east-1");
        assert!(config.path_style);
    }

    #[test]
    fn test_s3_config_path_style_false() {
        let config = S3Config::new(
            "http://localhost:9000".into(),
            "bucket".into(),
            "key".into(),
            "secret".into(),
            None,
            Some(false),
        );
        assert!(!config.path_style);
    }

    #[test]
    fn test_s3_config_path_style_explicit_true() {
        let config = S3Config::new(
            "http://localhost:9000".into(),
            "bucket".into(),
            "key".into(),
            "secret".into(),
            None,
            Some(true),
        );
        assert!(config.path_style);
    }

    #[test]
    fn test_should_exclude_db_at_root() {
        let data_dir = Path::new("/data");
        assert!(should_exclude(Path::new("/data/substrukt.db"), data_dir));
        assert!(should_exclude(
            Path::new("/data/substrukt.db-wal"),
            data_dir
        ));
        assert!(should_exclude(
            Path::new("/data/substrukt.db-shm"),
            data_dir
        ));
        assert!(should_exclude(Path::new("/data/audit.db"), data_dir));
        assert!(should_exclude(Path::new("/data/audit.db-wal"), data_dir));
        assert!(should_exclude(Path::new("/data/audit.db-shm"), data_dir));
    }

    #[test]
    fn test_should_exclude_db_in_subdir() {
        let data_dir = Path::new("/data");
        assert!(!should_exclude(
            Path::new("/data/content/test.db"),
            data_dir
        ));
        assert!(!should_exclude(
            Path::new("/data/uploads/ab/test.db-wal"),
            data_dir
        ));
    }

    #[test]
    fn test_should_exclude_non_db() {
        let data_dir = Path::new("/data");
        assert!(!should_exclude(
            Path::new("/data/schemas/blog.json"),
            data_dir
        ));
        assert!(!should_exclude(
            Path::new("/data/content/posts/hello.json"),
            data_dir
        ));
    }

    #[tokio::test]
    async fn test_create_archive_produces_valid_targz() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(data_dir.join("schemas")).unwrap();
        std::fs::write(data_dir.join("schemas/test.json"), r#"{"title":"Test"}"#).unwrap();

        // Create file-based SQLite pools (VACUUM INTO requires file-based)
        let main_db = tmp.path().join("main.db");
        let main_pool = crate::db::init_pool(&main_db).await.unwrap();

        let audit_db = tmp.path().join("audit.db");
        let audit_pool = crate::audit::init_pool(&audit_db).await.unwrap();

        let (archive_path, manifest) = create_archive(&data_dir, &main_pool, &audit_pool)
            .await
            .unwrap();

        assert!(archive_path.exists());

        // Verify tar.gz contents
        let file = std::fs::File::open(&archive_path).unwrap();
        let dec = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(dec);
        let entries: Vec<String> = archive
            .entries()
            .unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.path().ok().map(|p| p.to_string_lossy().to_string()))
            .collect();

        assert!(entries.contains(&"schemas/test.json".to_string()));
        assert!(entries.contains(&"substrukt.db".to_string()));
        assert!(entries.contains(&"audit.db".to_string()));
        assert!(entries.contains(&"manifest.json".to_string()));

        // Verify manifest fields
        assert_eq!(manifest["version"], 1);
        assert!(manifest["timestamp"].is_string());
        assert!(manifest["substrukt_version"].is_string());
        assert!(manifest["databases"].is_array());
        assert!(manifest["total_files"].is_number());
        assert!(manifest["total_size_bytes"].is_number());
        assert!(manifest["data_dir_entries"].is_array());

        // Cleanup
        let _ = std::fs::remove_file(&archive_path);
    }

    #[tokio::test]
    async fn test_manifest_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(data_dir.join("schemas")).unwrap();
        std::fs::create_dir_all(data_dir.join("content")).unwrap();
        std::fs::write(data_dir.join("schemas/blog.json"), "{}").unwrap();

        let main_db = tmp.path().join("main.db");
        let main_pool = crate::db::init_pool(&main_db).await.unwrap();
        let audit_db = tmp.path().join("audit.db");
        let audit_pool = crate::audit::init_pool(&audit_db).await.unwrap();

        let (archive_path, manifest) = create_archive(&data_dir, &main_pool, &audit_pool)
            .await
            .unwrap();

        assert_eq!(manifest["version"], 1);
        assert_eq!(manifest["substrukt_version"], env!("CARGO_PKG_VERSION"));

        let entries = manifest["data_dir_entries"].as_array().unwrap();
        let entry_strs: Vec<&str> = entries.iter().filter_map(|v| v.as_str()).collect();
        assert!(entry_strs.contains(&"schemas"));
        assert!(entry_strs.contains(&"content"));

        let dbs = manifest["databases"].as_array().unwrap();
        assert_eq!(dbs.len(), 2);

        let total_files = manifest["total_files"].as_u64().unwrap();
        assert!(total_files >= 4); // schema file + 2 dbs + manifest

        let _ = std::fs::remove_file(&archive_path);
    }

    #[test]
    fn test_is_backup_stuck_old_record() {
        let two_hours_ago = (chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
        let record = BackupRecord {
            id: 1,
            started_at: two_hours_ago,
            completed_at: None,
            status: "running".to_string(),
            trigger_source: "scheduled".to_string(),
            error_message: None,
            size_bytes: None,
            s3_key: None,
            manifest: None,
        };
        assert!(is_backup_stuck(&record));
    }

    #[test]
    fn test_is_backup_stuck_recent_record() {
        let five_mins_ago = (chrono::Utc::now() - chrono::Duration::minutes(5)).to_rfc3339();
        let record = BackupRecord {
            id: 1,
            started_at: five_mins_ago,
            completed_at: None,
            status: "running".to_string(),
            trigger_source: "scheduled".to_string(),
            error_message: None,
            size_bytes: None,
            s3_key: None,
            manifest: None,
        };
        assert!(!is_backup_stuck(&record));
    }

    #[test]
    fn test_calculate_next_backup_delay_never_backed_up() {
        let delay = calculate_next_backup_delay(None, 24);
        assert_eq!(delay, std::time::Duration::ZERO);
    }

    #[test]
    fn test_calculate_next_backup_delay_recent_backup() {
        let two_hours_ago = (chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
        let record = BackupRecord {
            id: 1,
            started_at: two_hours_ago,
            completed_at: None,
            status: "success".to_string(),
            trigger_source: "scheduled".to_string(),
            error_message: None,
            size_bytes: None,
            s3_key: None,
            manifest: None,
        };
        let delay = calculate_next_backup_delay(Some(&record), 24);
        // Should be roughly 22 hours
        let hours = delay.as_secs() as f64 / 3600.0;
        assert!(
            hours > 21.0 && hours < 23.0,
            "Expected ~22 hours, got {hours}"
        );
    }

    #[test]
    fn test_calculate_next_backup_delay_overdue() {
        let two_days_ago = (chrono::Utc::now() - chrono::Duration::hours(48)).to_rfc3339();
        let record = BackupRecord {
            id: 1,
            started_at: two_days_ago,
            completed_at: None,
            status: "success".to_string(),
            trigger_source: "scheduled".to_string(),
            error_message: None,
            size_bytes: None,
            s3_key: None,
            manifest: None,
        };
        let delay = calculate_next_backup_delay(Some(&record), 24);
        assert_eq!(delay, std::time::Duration::ZERO);
    }

    #[test]
    fn test_calculate_next_backup_delay_unparseable_timestamp() {
        let record = BackupRecord {
            id: 1,
            started_at: "not-a-timestamp".to_string(),
            completed_at: None,
            status: "success".to_string(),
            trigger_source: "scheduled".to_string(),
            error_message: None,
            size_bytes: None,
            s3_key: None,
            manifest: None,
        };
        let delay = calculate_next_backup_delay(Some(&record), 24);
        assert_eq!(
            delay,
            std::time::Duration::ZERO,
            "Unparseable timestamp should return ZERO (backup immediately)"
        );
    }

    #[test]
    fn test_is_backup_stuck_completed_record() {
        let record = BackupRecord {
            id: 1,
            started_at: (chrono::Utc::now() - chrono::Duration::hours(3)).to_rfc3339(),
            completed_at: Some(chrono::Utc::now().to_rfc3339()),
            status: "success".to_string(),
            trigger_source: "scheduled".to_string(),
            error_message: None,
            size_bytes: Some(1024),
            s3_key: Some("backups/test.tar.gz".to_string()),
            manifest: None,
        };
        assert!(
            !is_backup_stuck(&record),
            "Completed backups should not be considered stuck"
        );
    }

    #[test]
    fn test_is_backup_stuck_failed_record() {
        let record = BackupRecord {
            id: 1,
            started_at: (chrono::Utc::now() - chrono::Duration::hours(3)).to_rfc3339(),
            completed_at: Some(chrono::Utc::now().to_rfc3339()),
            status: "failed".to_string(),
            trigger_source: "manual".to_string(),
            error_message: Some("connection refused".to_string()),
            size_bytes: None,
            s3_key: None,
            manifest: None,
        };
        assert!(
            !is_backup_stuck(&record),
            "Failed backups should not be considered stuck"
        );
    }

    #[test]
    fn test_is_backup_stuck_invalid_timestamp() {
        let record = BackupRecord {
            id: 1,
            started_at: "garbage".to_string(),
            completed_at: None,
            status: "running".to_string(),
            trigger_source: "scheduled".to_string(),
            error_message: None,
            size_bytes: None,
            s3_key: None,
            manifest: None,
        };
        assert!(
            !is_backup_stuck(&record),
            "Invalid timestamp should return false (cannot determine if stuck)"
        );
    }

    #[test]
    fn test_should_exclude_no_extension_at_root() {
        let data_dir = Path::new("/data");
        assert!(
            !should_exclude(Path::new("/data/some_file"), data_dir),
            "Files without .db extension should not be excluded"
        );
        assert!(
            !should_exclude(Path::new("/data/README"), data_dir),
            "Files without extension at root should not be excluded"
        );
    }

    #[tokio::test]
    async fn test_create_archive_empty_data_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        // Empty data dir -- no schemas, content, or uploads

        let main_db = tmp.path().join("main.db");
        let main_pool = crate::db::init_pool(&main_db).await.unwrap();
        let audit_db = tmp.path().join("audit.db");
        let audit_pool = crate::audit::init_pool(&audit_db).await.unwrap();

        let (archive_path, manifest) = create_archive(&data_dir, &main_pool, &audit_pool)
            .await
            .unwrap();

        assert!(archive_path.exists());

        // Should still contain DB snapshots and manifest
        let file = std::fs::File::open(&archive_path).unwrap();
        let dec = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(dec);
        let entries: Vec<String> = archive
            .entries()
            .unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.path().ok().map(|p| p.to_string_lossy().to_string()))
            .collect();

        assert!(entries.contains(&"substrukt.db".to_string()));
        assert!(entries.contains(&"audit.db".to_string()));
        assert!(entries.contains(&"manifest.json".to_string()));

        // Manifest should show 3 total files (2 DBs + manifest)
        assert_eq!(manifest["total_files"], 3);
        assert!(manifest["data_dir_entries"].as_array().unwrap().is_empty());

        let _ = std::fs::remove_file(&archive_path);
    }

    #[tokio::test]
    async fn test_create_archive_excludes_db_at_root_includes_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(data_dir.join("content/myschema")).unwrap();

        // DB files at root (should be excluded from walk, replaced by VACUUM snapshots)
        std::fs::write(data_dir.join("substrukt.db"), "fake-db").unwrap();
        std::fs::write(data_dir.join("substrukt.db-wal"), "fake-wal").unwrap();
        std::fs::write(data_dir.join("audit.db"), "fake-audit").unwrap();

        // DB file in subdirectory (should be included)
        std::fs::write(data_dir.join("content/myschema/data.db"), "subdir-db").unwrap();
        std::fs::write(data_dir.join("content/myschema/entry.json"), "{}").unwrap();

        let main_db = tmp.path().join("main.db");
        let main_pool = crate::db::init_pool(&main_db).await.unwrap();
        let audit_db = tmp.path().join("audit.db");
        let audit_pool = crate::audit::init_pool(&audit_db).await.unwrap();

        let (archive_path, _manifest) = create_archive(&data_dir, &main_pool, &audit_pool)
            .await
            .unwrap();

        let file = std::fs::File::open(&archive_path).unwrap();
        let dec = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(dec);
        let entries: Vec<String> = archive
            .entries()
            .unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.path().ok().map(|p| p.to_string_lossy().to_string()))
            .collect();

        // DB files in subdir should be present
        assert!(
            entries.contains(&"content/myschema/data.db".to_string()),
            "DB files in subdirectories should be included: {:?}",
            entries
        );
        assert!(entries.contains(&"content/myschema/entry.json".to_string()));

        // substrukt.db and audit.db should come from VACUUM snapshots, not raw files
        assert!(entries.contains(&"substrukt.db".to_string()));
        assert!(entries.contains(&"audit.db".to_string()));

        // The raw data-dir DB-wal file should NOT be included
        assert!(
            !entries.contains(&"substrukt.db-wal".to_string()),
            "WAL files at data root should be excluded"
        );

        let _ = std::fs::remove_file(&archive_path);
    }

    // ── S3 integration tests (require Minio) ────────────────
    // Start Minio: docker run -p 9000:9000 -e MINIO_ROOT_USER=minioadmin -e MINIO_ROOT_PASSWORD=minioadmin minio/minio server /data
    // Run: SUBSTRUKT_S3_ENDPOINT=http://localhost:9000 SUBSTRUKT_S3_BUCKET=test-backups SUBSTRUKT_S3_ACCESS_KEY=minioadmin SUBSTRUKT_S3_SECRET_KEY=minioadmin cargo test -- --ignored test_full_backup

    #[tokio::test]
    #[ignore]
    async fn test_full_backup_cycle_s3() {
        let config = S3Config::new(
            std::env::var("SUBSTRUKT_S3_ENDPOINT").expect("SUBSTRUKT_S3_ENDPOINT required"),
            std::env::var("SUBSTRUKT_S3_BUCKET").expect("SUBSTRUKT_S3_BUCKET required"),
            std::env::var("SUBSTRUKT_S3_ACCESS_KEY").expect("SUBSTRUKT_S3_ACCESS_KEY required"),
            std::env::var("SUBSTRUKT_S3_SECRET_KEY").expect("SUBSTRUKT_S3_SECRET_KEY required"),
            std::env::var("SUBSTRUKT_S3_REGION").ok(),
            Some(true),
        );

        // Create a test archive
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(data_dir.join("schemas")).unwrap();
        std::fs::write(data_dir.join("schemas/test.json"), r#"{"title":"Test"}"#).unwrap();

        let main_db = tmp.path().join("main.db");
        let main_pool = crate::db::init_pool(&main_db).await.unwrap();
        let audit_db = tmp.path().join("audit.db");
        let audit_pool = crate::audit::init_pool(&audit_db).await.unwrap();

        let (archive_path, _manifest) = create_archive(&data_dir, &main_pool, &audit_pool)
            .await
            .unwrap();

        // Upload
        let bucket = create_bucket(&config).unwrap();
        let key = format!(
            "backups/test-{}.tar.gz",
            chrono::Utc::now().format("%Y-%m-%dT%H-%M-%SZ")
        );
        upload_archive(&bucket, &key, &archive_path).await.unwrap();

        // Verify it exists in list
        let keys = list_backups(&bucket).await.unwrap();
        assert!(keys.contains(&key), "Uploaded backup should appear in list");

        // Cleanup
        delete_backup(&bucket, &key).await.unwrap();
        let _ = std::fs::remove_file(&archive_path);
    }

    #[tokio::test]
    #[ignore]
    async fn test_retention_cleanup_s3() {
        let config = S3Config::new(
            std::env::var("SUBSTRUKT_S3_ENDPOINT").expect("SUBSTRUKT_S3_ENDPOINT required"),
            std::env::var("SUBSTRUKT_S3_BUCKET").expect("SUBSTRUKT_S3_BUCKET required"),
            std::env::var("SUBSTRUKT_S3_ACCESS_KEY").expect("SUBSTRUKT_S3_ACCESS_KEY required"),
            std::env::var("SUBSTRUKT_S3_SECRET_KEY").expect("SUBSTRUKT_S3_SECRET_KEY required"),
            std::env::var("SUBSTRUKT_S3_REGION").ok(),
            Some(true),
        );

        let bucket = create_bucket(&config).unwrap();

        // Upload 4 dummy objects
        let keys: Vec<String> = (0..4)
            .map(|i| format!("backups/retention-test-{i}.tar.gz"))
            .collect();

        for key in &keys {
            bucket.put_object(key, b"test").await.unwrap();
        }

        // Enforce retention of 2
        let deleted = enforce_retention(&bucket, 2).await.unwrap();
        assert_eq!(deleted, 2, "Should delete 2 oldest backups");

        // Cleanup remaining
        let remaining = list_backups(&bucket).await.unwrap();
        for key in remaining.iter().filter(|k| k.contains("retention-test")) {
            let _ = delete_backup(&bucket, key).await;
        }
    }
}
