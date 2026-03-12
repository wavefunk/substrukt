use std::collections::HashSet;
use std::path::Path;

use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UploadMeta {
    pub hash: String,
    pub filename: String,
    pub mime: String,
    pub size: u64,
}

/// Sanitize a filename: strip path components, replace unsafe chars.
fn sanitize_filename(filename: &str) -> String {
    // Take only the filename part (strip directory components)
    let name = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    // Replace anything that isn't alphanumeric, dot, hyphen, or underscore
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    // Prevent hidden files and empty names
    let sanitized = sanitized.trim_start_matches('.');
    if sanitized.is_empty() {
        "upload".to_string()
    } else {
        sanitized.to_string()
    }
}

/// Validate that a hash string is valid hex (prevents path traversal).
fn is_valid_hash(hash: &str) -> bool {
    hash.len() >= 3 && hash.chars().all(|c| c.is_ascii_hexdigit())
}

/// Check if a string is a valid SHA-256 hash (exactly 64 hex characters).
fn is_sha256_hash(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

pub async fn store_upload(
    uploads_dir: &Path,
    pool: &SqlitePool,
    filename: &str,
    mime: &str,
    data: &[u8],
) -> eyre::Result<UploadMeta> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let hash = hex::encode(hasher.finalize());
    let safe_filename = sanitize_filename(filename);

    let prefix = &hash[..2];
    let rest = &hash[2..];
    let dir = uploads_dir.join(prefix);
    std::fs::create_dir_all(&dir)?;

    let file_path = dir.join(rest);
    if !file_path.exists() {
        std::fs::write(&file_path, data)?;
    }

    let meta = UploadMeta {
        hash: hash.clone(),
        filename: safe_filename,
        mime: mime.to_string(),
        size: data.len() as u64,
    };
    db_insert_upload(pool, &meta).await?;

    Ok(meta)
}

pub fn get_upload_path(uploads_dir: &Path, hash: &str) -> Option<std::path::PathBuf> {
    if !is_valid_hash(hash) {
        return None;
    }
    let prefix = &hash[..2];
    let rest = &hash[2..];
    let path = uploads_dir.join(prefix).join(rest);
    if path.exists() { Some(path) } else { None }
}

// -- SQLite operations --

/// Insert upload metadata into SQLite. Uses INSERT OR IGNORE for dedup.
pub async fn db_insert_upload(pool: &SqlitePool, meta: &UploadMeta) -> eyre::Result<()> {
    sqlx::query("INSERT OR IGNORE INTO uploads (hash, filename, mime, size) VALUES (?, ?, ?, ?)")
        .bind(&meta.hash)
        .bind(&meta.filename)
        .bind(&meta.mime)
        .bind(meta.size as i64)
        .execute(pool)
        .await?;
    Ok(())
}

/// Get upload metadata from SQLite by hash.
pub async fn db_get_upload_meta(pool: &SqlitePool, hash: &str) -> eyre::Result<Option<UploadMeta>> {
    let row = sqlx::query_as::<_, (String, String, String, i64)>(
        "SELECT hash, filename, mime, size FROM uploads WHERE hash = ?",
    )
    .bind(hash)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(hash, filename, mime, size)| UploadMeta {
        hash,
        filename,
        mime,
        size: size as u64,
    }))
}

/// Replace all upload references for a content entry.
pub async fn db_update_references(
    pool: &SqlitePool,
    schema_slug: &str,
    entry_id: &str,
    hashes: &HashSet<String>,
) -> eyre::Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM upload_references WHERE schema_slug = ? AND entry_id = ?")
        .bind(schema_slug)
        .bind(entry_id)
        .execute(&mut *tx)
        .await?;

    for hash in hashes {
        sqlx::query(
            "INSERT OR IGNORE INTO upload_references (upload_hash, schema_slug, entry_id) VALUES (?, ?, ?)"
        )
        .bind(hash)
        .bind(schema_slug)
        .bind(entry_id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Delete all upload references for a content entry.
pub async fn db_delete_references(
    pool: &SqlitePool,
    schema_slug: &str,
    entry_id: &str,
) -> eyre::Result<()> {
    sqlx::query("DELETE FROM upload_references WHERE schema_slug = ? AND entry_id = ?")
        .bind(schema_slug)
        .bind(entry_id)
        .execute(pool)
        .await?;
    Ok(())
}

// -- Hash extraction --

/// Recursively walk JSON and extract hashes from upload objects.
/// Upload objects have shape: {"hash": "<64-char hex>", "filename": "...", "mime": "..."}
pub fn extract_upload_hashes(value: &Value) -> HashSet<String> {
    let mut hashes = HashSet::new();
    collect_upload_hashes(value, &mut hashes);
    hashes
}

fn collect_upload_hashes(value: &Value, hashes: &mut HashSet<String>) {
    match value {
        Value::Object(map) => {
            // Check if this object looks like an upload reference
            if let (Some(Value::String(hash)), Some(Value::String(_)), Some(Value::String(_))) =
                (map.get("hash"), map.get("filename"), map.get("mime"))
                && is_sha256_hash(hash)
            {
                hashes.insert(hash.clone());
                return;
            }
            // Otherwise recurse into values
            for v in map.values() {
                collect_upload_hashes(v, hashes);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                collect_upload_hashes(v, hashes);
            }
        }
        _ => {}
    }
}

// -- Sidecar migration --

/// One-time migration: populate SQLite from existing .meta.json sidecars.
/// Idempotent: only runs if .meta.json files exist on disk.
/// Deletes .meta.json files after successful migration.
pub async fn migrate_meta_sidecars(
    uploads_dir: &Path,
    data_dir: &Path,
    pool: &SqlitePool,
) -> eyre::Result<()> {
    // Find all .meta.json files
    let mut meta_files = Vec::new();
    if uploads_dir.exists() {
        for prefix_entry in std::fs::read_dir(uploads_dir)? {
            let prefix_entry = prefix_entry?;
            if prefix_entry.file_type()?.is_dir() {
                for file_entry in std::fs::read_dir(prefix_entry.path())? {
                    let file_entry = file_entry?;
                    let path = file_entry.path();
                    if path.to_string_lossy().ends_with(".meta.json") {
                        meta_files.push(path);
                    }
                }
            }
        }
    }

    if meta_files.is_empty() {
        return Ok(());
    }

    tracing::info!("Found {} .meta.json sidecars to migrate", meta_files.len());

    // Insert upload metadata (INSERT OR IGNORE handles re-runs safely)
    for meta_path in &meta_files {
        let content = std::fs::read_to_string(meta_path)?;
        let meta: UploadMeta = serde_json::from_str(&content)?;
        db_insert_upload(pool, &meta).await?;
    }

    // Scan content files and populate references
    populate_references_from_content(data_dir, pool).await?;

    // Delete .meta.json sidecars
    for meta_path in &meta_files {
        std::fs::remove_file(meta_path)?;
    }

    tracing::info!(
        "Migrated {} upload metadata files to SQLite",
        meta_files.len()
    );
    Ok(())
}

/// Scan all content JSON files and populate upload_references table.
/// Used by both startup migration and import.
pub async fn populate_references_from_content(
    data_dir: &Path,
    pool: &SqlitePool,
) -> eyre::Result<()> {
    let schemas_dir = data_dir.join("schemas");
    let content_dir = data_dir.join("content");
    if !schemas_dir.exists() || !content_dir.exists() {
        return Ok(());
    }

    for schema_entry in std::fs::read_dir(&schemas_dir)? {
        let schema_entry = schema_entry?;
        let schema_path = schema_entry.path();
        if schema_path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let schema_slug = schema_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();

        let schema_str = std::fs::read_to_string(&schema_path)?;
        let schema_val: Value = serde_json::from_str(&schema_str)?;
        let storage = schema_val
            .pointer("/x-substrukt/storage")
            .and_then(|v| v.as_str())
            .unwrap_or("directory");

        if storage == "directory" {
            let entry_dir = content_dir.join(&schema_slug);
            if entry_dir.exists() {
                for entry_file in std::fs::read_dir(&entry_dir)? {
                    let entry_file = entry_file?;
                    let entry_path = entry_file.path();
                    if entry_path.extension().and_then(|e| e.to_str()) != Some("json") {
                        continue;
                    }
                    let entry_id = entry_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default()
                        .to_string();
                    let data: Value = serde_json::from_str(&std::fs::read_to_string(&entry_path)?)?;
                    let hashes = extract_upload_hashes(&data);
                    db_update_references(pool, &schema_slug, &entry_id, &hashes).await?;
                }
            }
        } else {
            // SingleFile mode
            let single_path = content_dir.join(format!("{schema_slug}.json"));
            if single_path.exists() {
                let arr: Value = serde_json::from_str(&std::fs::read_to_string(&single_path)?)?;
                if let Value::Array(entries) = &arr {
                    for entry in entries {
                        let entry_id = entry
                            .get("_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let hashes = extract_upload_hashes(entry);
                        db_update_references(pool, &schema_slug, &entry_id, &hashes).await?;
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_upload_hashes_flat() {
        let data = json!({
            "title": "Hello",
            "image": {
                "hash": "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
                "filename": "photo.jpg",
                "mime": "image/jpeg"
            }
        });
        let hashes = extract_upload_hashes(&data);
        assert_eq!(hashes.len(), 1);
        assert!(
            hashes.contains("abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890")
        );
    }

    #[test]
    fn test_extract_upload_hashes_nested() {
        let data = json!({
            "author": {
                "name": "Alice",
                "avatar": {
                    "hash": "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
                    "filename": "avatar.png",
                    "mime": "image/png"
                }
            }
        });
        let hashes = extract_upload_hashes(&data);
        assert_eq!(hashes.len(), 1);
        assert!(
            hashes.contains("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef")
        );
    }

    #[test]
    fn test_extract_upload_hashes_in_array() {
        let data = json!({
            "gallery": [
                {
                    "hash": "aaaa567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
                    "filename": "img1.jpg",
                    "mime": "image/jpeg"
                },
                {
                    "hash": "bbbb567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
                    "filename": "img2.jpg",
                    "mime": "image/jpeg"
                }
            ]
        });
        let hashes = extract_upload_hashes(&data);
        assert_eq!(hashes.len(), 2);
    }

    #[test]
    fn test_extract_upload_hashes_ignores_non_uploads() {
        let data = json!({
            "title": "Hello",
            "count": 42,
            "nested": { "hash": "not-a-valid-hex-hash" }
        });
        let hashes = extract_upload_hashes(&data);
        assert_eq!(hashes.len(), 0);
    }
}
