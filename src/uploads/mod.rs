use std::path::Path;

use sha2::{Digest, Sha256};

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
    let name = filename
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(filename);
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

pub fn store_upload(
    uploads_dir: &Path,
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

    // Write sidecar metadata
    let meta = UploadMeta {
        hash: hash.clone(),
        filename: safe_filename,
        mime: mime.to_string(),
        size: data.len() as u64,
    };
    let meta_path = dir.join(format!("{rest}.meta.json"));
    if !meta_path.exists() {
        let meta_json = serde_json::to_string_pretty(&meta)?;
        std::fs::write(meta_path, meta_json)?;
    }

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

pub fn get_upload_meta(uploads_dir: &Path, hash: &str) -> Option<UploadMeta> {
    if !is_valid_hash(hash) {
        return None;
    }
    let prefix = &hash[..2];
    let rest = &hash[2..];
    let meta_path = uploads_dir.join(prefix).join(format!("{rest}.meta.json"));
    if !meta_path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(meta_path).ok()?;
    serde_json::from_str(&content).ok()
}
