use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

const MAX_DIFF_DEPTH: usize = 32;

#[derive(Debug, Clone, serde::Serialize)]
pub struct VersionInfo {
    pub timestamp: u64,
    pub size: u64,
    pub user_id: Option<String>,
    pub username: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FieldDiff {
    pub path: String,
    pub label: String,
    pub kind: DiffKind,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DiffKind {
    Changed { old: Value, new: Value },
    Added { value: Value },
    Removed { value: Value },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SnapshotMeta {
    pub user_id: String,
    pub username: String,
    pub source: SnapshotSource,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotSource {
    AdminUi,
    Api,
    Import,
    Revert,
}

pub fn diff_entries(old: &Value, new: &Value, schema: &Value) -> Vec<FieldDiff> {
    let mut diffs = Vec::new();
    diff_entries_inner(old, new, schema, "", &mut diffs, 0);

    let props = schema.get("properties").and_then(|p| p.as_object());

    if let (Some(old_obj), Some(new_obj)) = (old.as_object(), new.as_object()) {
        let schema_keys: std::collections::HashSet<&str> = props
            .map(|p| p.keys().map(|k| k.as_str()).collect())
            .unwrap_or_default();

        for key in old_obj.keys() {
            if key.starts_with('_') || schema_keys.contains(key.as_str()) {
                continue;
            }
            if !new_obj.contains_key(key) {
                diffs.push(FieldDiff {
                    path: key.clone(),
                    label: key.clone(),
                    kind: DiffKind::Removed {
                        value: old_obj[key].clone(),
                    },
                });
            }
        }
        for key in new_obj.keys() {
            if key.starts_with('_') || schema_keys.contains(key.as_str()) {
                continue;
            }
            if !old_obj.contains_key(key) {
                diffs.push(FieldDiff {
                    path: key.clone(),
                    label: key.clone(),
                    kind: DiffKind::Added {
                        value: new_obj[key].clone(),
                    },
                });
            }
        }
    }

    diffs
}

fn diff_entries_inner(
    old: &Value,
    new: &Value,
    schema: &Value,
    prefix: &str,
    diffs: &mut Vec<FieldDiff>,
    depth: usize,
) {
    if depth > MAX_DIFF_DEPTH {
        return;
    }
    let Some(props) = schema.get("properties").and_then(|p| p.as_object()) else {
        return;
    };
    for (key, prop_schema) in props {
        if key.starts_with('_') {
            continue;
        }
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        let label = prop_schema
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or(key)
            .to_string();

        let old_val = old.get(key);
        let new_val = new.get(key);
        let field_type = prop_schema.get("type").and_then(|t| t.as_str());

        match (old_val, new_val) {
            (None, None) | (Some(Value::Null), Some(Value::Null)) => {}
            (None | Some(Value::Null), Some(v)) if !v.is_null() => {
                diffs.push(FieldDiff {
                    path,
                    label,
                    kind: DiffKind::Added { value: v.clone() },
                });
            }
            (Some(v), None | Some(Value::Null)) if !v.is_null() => {
                diffs.push(FieldDiff {
                    path,
                    label,
                    kind: DiffKind::Removed { value: v.clone() },
                });
            }
            (Some(ov), Some(nv)) => {
                if field_type == Some("object") {
                    diff_entries_inner(ov, nv, prop_schema, &path, diffs, depth + 1);
                } else if field_type == Some("array") {
                    if ov != nv {
                        diffs.push(FieldDiff {
                            path,
                            label,
                            kind: DiffKind::Changed {
                                old: ov.clone(),
                                new: nv.clone(),
                            },
                        });
                    }
                } else if ov != nv {
                    diffs.push(FieldDiff {
                        path,
                        label,
                        kind: DiffKind::Changed {
                            old: ov.clone(),
                            new: nv.clone(),
                        },
                    });
                }
            }
            _ => {}
        }
    }
}

fn history_dir(data_dir: &Path, schema_slug: &str, entry_id: &str) -> std::path::PathBuf {
    data_dir.join("_history").join(schema_slug).join(entry_id)
}

/// Snapshot the current entry data before overwriting.
pub fn snapshot_entry(
    data_dir: &Path,
    schema_slug: &str,
    entry_id: &str,
    current_data: &Value,
    max_versions: usize,
    meta: Option<&SnapshotMeta>,
) -> eyre::Result<()> {
    if max_versions == 0 {
        return Ok(());
    }

    let dir = history_dir(data_dir, schema_slug, entry_id);
    std::fs::create_dir_all(&dir)?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let path = dir.join(format!("{timestamp}.json"));
    let content = serde_json::to_string_pretty(current_data)?;
    std::fs::write(&path, content)?;

    if let Some(meta) = meta {
        let meta_path = dir.join(format!("{timestamp}.meta.json"));
        let meta_content = serde_json::to_string_pretty(meta)?;
        if let Err(e) = std::fs::write(&meta_path, meta_content) {
            tracing::warn!("Failed to write snapshot metadata: {e}");
        }
    }

    prune_versions(&dir, max_versions)?;

    Ok(())
}

/// List available versions for an entry, newest first.
pub fn list_versions(
    data_dir: &Path,
    schema_slug: &str,
    entry_id: &str,
) -> eyre::Result<Vec<VersionInfo>> {
    let dir = history_dir(data_dir, schema_slug, entry_id);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut versions = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        let is_data_json = path.extension().is_some_and(|e| e == "json")
            && !path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .ends_with(".meta.json");
        if is_data_json {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if let Ok(ts) = stem.parse::<u64>() {
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    let meta_path = dir.join(format!("{ts}.meta.json"));
                    let (user_id, username, source) =
                        if let Ok(meta_str) = std::fs::read_to_string(&meta_path) {
                            if let Ok(meta) = serde_json::from_str::<SnapshotMeta>(&meta_str) {
                                (
                                    Some(meta.user_id),
                                    Some(meta.username),
                                    Some(
                                        serde_json::to_value(&meta.source)
                                            .ok()
                                            .and_then(|v| v.as_str().map(|s| s.to_string()))
                                            .unwrap_or_else(|| "unknown".to_string()),
                                    ),
                                )
                            } else {
                                (None, None, None)
                            }
                        } else {
                            (None, None, None)
                        };
                    versions.push(VersionInfo {
                        timestamp: ts,
                        size,
                        user_id,
                        username,
                        source,
                    });
                }
            }
        }
    }

    versions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    Ok(versions)
}

/// Load a specific version by timestamp.
pub fn get_version(
    data_dir: &Path,
    schema_slug: &str,
    entry_id: &str,
    timestamp: u64,
) -> eyre::Result<Option<Value>> {
    let path = history_dir(data_dir, schema_slug, entry_id).join(format!("{timestamp}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)?;
    let data: Value = serde_json::from_str(&content)?;
    Ok(Some(data))
}

/// Remove all history for an entry (call when the entry is deleted).
pub fn delete_history(data_dir: &Path, schema_slug: &str, entry_id: &str) {
    let dir = history_dir(data_dir, schema_slug, entry_id);
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

fn prune_versions(dir: &Path, max_versions: usize) -> eyre::Result<()> {
    let mut files: Vec<(u64, std::path::PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let is_data_json = path.extension().is_some_and(|e| e == "json")
            && !path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .ends_with(".meta.json");
        if is_data_json {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if let Ok(ts) = stem.parse::<u64>() {
                    files.push((ts, path));
                }
            }
        }
    }

    if files.len() <= max_versions {
        return Ok(());
    }

    files.sort_by_key(|(ts, _)| *ts);
    let to_remove = files.len() - max_versions;
    for (ts, path) in files.into_iter().take(to_remove) {
        std::fs::remove_file(&path)?;
        let meta_path = dir.join(format!("{ts}.meta.json"));
        let _ = std::fs::remove_file(meta_path);
    }

    Ok(())
}
