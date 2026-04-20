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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn test_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "title": "Title" },
                "body": { "type": "string", "title": "Body" }
            }
        })
    }

    #[test]
    fn diff_entries_identical_returns_empty() {
        let data = json!({"title": "Hello", "body": "World"});
        let schema = test_schema();
        let diffs = diff_entries(&data, &data, &schema);
        assert!(diffs.is_empty());
    }

    #[test]
    fn diff_entries_changed_field() {
        let old = json!({"title": "Old Title", "body": "Same"});
        let new = json!({"title": "New Title", "body": "Same"});
        let schema = test_schema();
        let diffs = diff_entries(&old, &new, &schema);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path, "title");
        assert_eq!(diffs[0].label, "Title");
        assert!(matches!(&diffs[0].kind, DiffKind::Changed { old, new }
            if old == &json!("Old Title") && new == &json!("New Title")));
    }

    #[test]
    fn diff_entries_added_field() {
        let old = json!({"title": "Hello"});
        let new = json!({"title": "Hello", "body": "Added"});
        let schema = test_schema();
        let diffs = diff_entries(&old, &new, &schema);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path, "body");
        assert!(matches!(&diffs[0].kind, DiffKind::Added { value } if value == &json!("Added")));
    }

    #[test]
    fn diff_entries_removed_field() {
        let old = json!({"title": "Hello", "body": "Will be removed"});
        let new = json!({"title": "Hello"});
        let schema = test_schema();
        let diffs = diff_entries(&old, &new, &schema);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path, "body");
        assert!(matches!(&diffs[0].kind, DiffKind::Removed { .. }));
    }

    #[test]
    fn diff_entries_skips_underscore_fields() {
        let old = json!({"_status": "draft", "_id": "a", "title": "Hello"});
        let new = json!({"_status": "published", "_id": "a", "title": "Hello"});
        let schema = test_schema();
        let diffs = diff_entries(&old, &new, &schema);
        assert!(
            diffs.is_empty(),
            "_status and _id changes should be excluded"
        );
    }

    #[test]
    fn diff_entries_nested_object() {
        let schema = json!({
            "type": "object",
            "properties": {
                "meta": {
                    "type": "object",
                    "title": "Meta",
                    "properties": {
                        "author": { "type": "string", "title": "Author" }
                    }
                }
            }
        });
        let old = json!({"meta": {"author": "Alice"}});
        let new = json!({"meta": {"author": "Bob"}});
        let diffs = diff_entries(&old, &new, &schema);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path, "meta.author");
        assert_eq!(diffs[0].label, "Author");
    }

    #[test]
    fn diff_entries_array_changed() {
        let schema = json!({
            "type": "object",
            "properties": {
                "tags": { "type": "array", "title": "Tags" }
            }
        });
        let old = json!({"tags": ["a", "b"]});
        let new = json!({"tags": ["a", "c"]});
        let diffs = diff_entries(&old, &new, &schema);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path, "tags");
        assert!(matches!(&diffs[0].kind, DiffKind::Changed { .. }));
    }

    #[test]
    fn diff_entries_schema_drift_old_has_extra_field() {
        let old = json!({"title": "Hello", "legacy_field": "old"});
        let new = json!({"title": "Hello"});
        let schema = test_schema();
        let diffs = diff_entries(&old, &new, &schema);
        assert!(
            diffs
                .iter()
                .any(|d| d.path == "legacy_field" && matches!(&d.kind, DiffKind::Removed { .. }))
        );
    }

    #[test]
    fn diff_entries_schema_drift_new_has_extra_field() {
        let old = json!({"title": "Hello"});
        let new = json!({"title": "Hello", "new_field": "value"});
        let schema = test_schema();
        let diffs = diff_entries(&old, &new, &schema);
        assert!(
            diffs
                .iter()
                .any(|d| d.path == "new_field" && matches!(&d.kind, DiffKind::Added { .. }))
        );
    }

    #[test]
    fn diff_entries_depth_limit() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "val": { "type": "string", "title": "Val" }
            }
        });
        for _ in 0..40 {
            schema = json!({
                "type": "object",
                "properties": {
                    "nested": schema
                }
            });
        }
        let old = json!({"nested": {}});
        let new = json!({"nested": {}});
        let diffs = diff_entries(&old, &new, &schema);
        assert!(diffs.is_empty(), "should not panic at deep nesting");
    }

    #[test]
    fn snapshot_with_metadata_writes_sidecar() {
        let tmp = TempDir::new().unwrap();
        let data = json!({"title": "Hello"});
        let meta = SnapshotMeta {
            user_id: "user-123".into(),
            username: "alice".into(),
            source: SnapshotSource::AdminUi,
        };
        snapshot_entry(tmp.path(), "posts", "entry-1", &data, 10, Some(&meta)).unwrap();

        let versions = list_versions(tmp.path(), "posts", "entry-1").unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].user_id.as_deref(), Some("user-123"));
        assert_eq!(versions[0].username.as_deref(), Some("alice"));
        assert_eq!(versions[0].source.as_deref(), Some("admin_ui"));
    }

    #[test]
    fn snapshot_without_metadata_returns_none_fields() {
        let tmp = TempDir::new().unwrap();
        let data = json!({"title": "Hello"});
        snapshot_entry(tmp.path(), "posts", "entry-1", &data, 10, None).unwrap();

        let versions = list_versions(tmp.path(), "posts", "entry-1").unwrap();
        assert_eq!(versions.len(), 1);
        assert!(versions[0].user_id.is_none());
        assert!(versions[0].username.is_none());
        assert!(versions[0].source.is_none());
    }

    #[test]
    fn prune_removes_sidecar_alongside_data() {
        let tmp = TempDir::new().unwrap();
        let data = json!({"title": "Hello"});
        let meta = SnapshotMeta {
            user_id: "u".into(),
            username: "a".into(),
            source: SnapshotSource::Api,
        };
        for _ in 0..5 {
            snapshot_entry(tmp.path(), "posts", "e", &data, 100, Some(&meta)).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let versions_before = list_versions(tmp.path(), "posts", "e").unwrap();
        assert_eq!(versions_before.len(), 5);

        let dir = tmp.path().join("_history").join("posts").join("e");
        let meta_count_before = std::fs::read_dir(&dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".meta.json")
            })
            .count();
        assert_eq!(meta_count_before, 5, "should have 5 meta sidecars");

        snapshot_entry(tmp.path(), "posts", "e", &data, 3, Some(&meta)).unwrap();

        let versions_after = list_versions(tmp.path(), "posts", "e").unwrap();
        assert_eq!(versions_after.len(), 3, "should be pruned to 3");

        let meta_count_after = std::fs::read_dir(&dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".meta.json")
            })
            .count();
        assert_eq!(meta_count_after, 3, "sidecar files should also be pruned");
    }

    #[test]
    fn list_versions_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let versions = list_versions(tmp.path(), "posts", "nonexistent").unwrap();
        assert!(versions.is_empty());
    }

    #[test]
    fn get_version_nonexistent_returns_none() {
        let tmp = TempDir::new().unwrap();
        let result = get_version(tmp.path(), "posts", "entry-1", 99999).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn snapshot_and_get_round_trip() {
        let tmp = TempDir::new().unwrap();
        let data = json!({"title": "Hello", "body": "World"});
        snapshot_entry(tmp.path(), "posts", "e1", &data, 10, None).unwrap();

        let versions = list_versions(tmp.path(), "posts", "e1").unwrap();
        assert_eq!(versions.len(), 1);

        let restored = get_version(tmp.path(), "posts", "e1", versions[0].timestamp)
            .unwrap()
            .unwrap();
        assert_eq!(restored, data);
    }

    #[test]
    fn snapshot_max_versions_zero_skips() {
        let tmp = TempDir::new().unwrap();
        let data = json!({"title": "Hello"});
        snapshot_entry(tmp.path(), "posts", "e1", &data, 0, None).unwrap();

        let versions = list_versions(tmp.path(), "posts", "e1").unwrap();
        assert!(versions.is_empty(), "max_versions=0 should skip snapshot");
    }

    #[test]
    fn delete_history_removes_all() {
        let tmp = TempDir::new().unwrap();
        let data = json!({"title": "Hello"});
        let meta = SnapshotMeta {
            user_id: "u".into(),
            username: "a".into(),
            source: SnapshotSource::AdminUi,
        };
        snapshot_entry(tmp.path(), "posts", "e1", &data, 10, Some(&meta)).unwrap();
        snapshot_entry(tmp.path(), "posts", "e1", &data, 10, Some(&meta)).unwrap();

        let dir = tmp.path().join("_history").join("posts").join("e1");
        assert!(dir.exists());

        delete_history(tmp.path(), "posts", "e1");
        assert!(
            !dir.exists(),
            "delete_history should remove entire directory"
        );
    }

    #[test]
    fn snapshot_source_serializes_correctly() {
        let meta = SnapshotMeta {
            user_id: "u".into(),
            username: "a".into(),
            source: SnapshotSource::Revert,
        };
        let json = serde_json::to_value(&meta).unwrap();
        assert_eq!(json["source"], "revert");

        let meta2 = SnapshotMeta {
            user_id: "u".into(),
            username: "a".into(),
            source: SnapshotSource::AdminUi,
        };
        let json2 = serde_json::to_value(&meta2).unwrap();
        assert_eq!(json2["source"], "admin_ui");
    }
}
