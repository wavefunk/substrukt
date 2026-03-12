pub mod form;

use std::path::Path;

use serde_json::Value;
use uuid::Uuid;

use crate::schema::models::{SchemaFile, StorageMode};

/// A single content entry
#[derive(Debug, Clone, serde::Serialize)]
pub struct ContentEntry {
    pub id: String,
    pub data: Value,
}

pub fn list_entries(content_dir: &Path, schema: &SchemaFile) -> eyre::Result<Vec<ContentEntry>> {
    let slug = &schema.meta.slug;
    match schema.meta.storage {
        StorageMode::Directory => list_directory_entries(content_dir, slug),
        StorageMode::SingleFile => list_single_file_entries(content_dir, slug),
    }
}

fn list_directory_entries(content_dir: &Path, slug: &str) -> eyre::Result<Vec<ContentEntry>> {
    let dir = content_dir.join(slug);
    let mut entries = Vec::new();
    if !dir.exists() {
        return Ok(entries);
    }
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            let id = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let content = std::fs::read_to_string(&path)?;
            let data: Value = serde_json::from_str(&content)?;
            entries.push(ContentEntry { id, data });
        }
    }
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(entries)
}

fn list_single_file_entries(content_dir: &Path, slug: &str) -> eyre::Result<Vec<ContentEntry>> {
    let path = content_dir.join(format!("{slug}.json"));
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)?;
    let arr: Vec<Value> = serde_json::from_str(&content)?;
    Ok(arr
        .into_iter()
        .enumerate()
        .map(|(i, data)| {
            let id = data
                .get("_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| i.to_string());
            ContentEntry { id, data }
        })
        .collect())
}

pub fn get_entry(
    content_dir: &Path,
    schema: &SchemaFile,
    entry_id: &str,
) -> eyre::Result<Option<ContentEntry>> {
    let slug = &schema.meta.slug;
    match schema.meta.storage {
        StorageMode::Directory => {
            let path = content_dir.join(slug).join(format!("{entry_id}.json"));
            if !path.exists() {
                return Ok(None);
            }
            let content = std::fs::read_to_string(&path)?;
            let data: Value = serde_json::from_str(&content)?;
            Ok(Some(ContentEntry {
                id: entry_id.to_string(),
                data,
            }))
        }
        StorageMode::SingleFile => {
            let entries = list_single_file_entries(content_dir, slug)?;
            Ok(entries.into_iter().find(|e| e.id == entry_id))
        }
    }
}

pub fn save_entry(
    content_dir: &Path,
    schema: &SchemaFile,
    entry_id: Option<&str>,
    data: Value,
) -> eyre::Result<String> {
    let slug = &schema.meta.slug;
    match schema.meta.storage {
        StorageMode::Directory => {
            let dir = content_dir.join(slug);
            std::fs::create_dir_all(&dir)?;
            let id = entry_id
                .map(|s| s.to_string())
                .unwrap_or_else(|| generate_entry_id(schema, &data));
            let path = dir.join(format!("{id}.json"));
            let content = serde_json::to_string_pretty(&data)?;
            std::fs::write(path, content)?;
            Ok(id)
        }
        StorageMode::SingleFile => {
            let path = content_dir.join(format!("{slug}.json"));
            let mut entries = if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                serde_json::from_str::<Vec<Value>>(&content)?
            } else {
                Vec::new()
            };

            let id = entry_id
                .map(|s| s.to_string())
                .unwrap_or_else(|| Uuid::new_v4().to_string());

            // Insert _id into data
            let mut data = data;
            if let Some(obj) = data.as_object_mut() {
                obj.insert("_id".to_string(), Value::String(id.clone()));
            }

            if let Some(existing_id) = entry_id {
                // Update existing
                if let Some(pos) = entries.iter().position(|e| {
                    e.get("_id")
                        .and_then(|v| v.as_str())
                        .is_some_and(|s| s == existing_id)
                }) {
                    entries[pos] = data;
                } else {
                    entries.push(data);
                }
            } else {
                entries.push(data);
            }

            let content = serde_json::to_string_pretty(&entries)?;
            std::fs::write(path, content)?;
            Ok(id)
        }
    }
}

pub fn delete_entry(content_dir: &Path, schema: &SchemaFile, entry_id: &str) -> eyre::Result<()> {
    let slug = &schema.meta.slug;
    match schema.meta.storage {
        StorageMode::Directory => {
            let path = content_dir.join(slug).join(format!("{entry_id}.json"));
            if path.exists() {
                std::fs::remove_file(path)?;
            }
        }
        StorageMode::SingleFile => {
            let path = content_dir.join(format!("{slug}.json"));
            if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                let mut entries: Vec<Value> = serde_json::from_str(&content)?;
                entries.retain(|e| {
                    e.get("_id")
                        .and_then(|v| v.as_str())
                        .is_none_or(|s| s != entry_id)
                });
                let content = serde_json::to_string_pretty(&entries)?;
                std::fs::write(path, content)?;
            }
        }
    }
    Ok(())
}

pub fn validate_content(schema: &SchemaFile, data: &Value) -> Result<(), Vec<String>> {
    // Patch schema to accept objects for upload fields, since uploads are stored
    // as {hash, filename, mime} objects rather than plain strings.
    let patched = patch_upload_types(&schema.schema);
    match jsonschema::validator_for(&patched) {
        Ok(validator) => {
            let errors: Vec<String> = validator
                .iter_errors(data)
                .map(|e| format!("{}: {}", e.instance_path, e))
                .collect();
            if errors.is_empty() {
                Ok(())
            } else {
                Err(errors)
            }
        }
        Err(e) => Err(vec![format!("Invalid schema: {e}")]),
    }
}

/// Rewrite `{"type": "string", "format": "upload"}` properties to accept
/// either a string or an object so that stored upload references pass validation.
fn patch_upload_types(schema: &Value) -> Value {
    let mut schema = schema.clone();
    if let Some(props) = schema
        .get_mut("properties")
        .and_then(|p| p.as_object_mut())
    {
        for (_key, prop) in props.iter_mut() {
            let is_upload = prop.get("type").and_then(|t| t.as_str()) == Some("string")
                && prop.get("format").and_then(|f| f.as_str()) == Some("upload");
            if is_upload {
                // Allow string or object
                if let Some(obj) = prop.as_object_mut() {
                    obj.remove("type");
                    obj.insert(
                        "type".to_string(),
                        serde_json::json!(["string", "object"]),
                    );
                }
            }
        }
    }
    schema
}

fn generate_entry_id(schema: &SchemaFile, data: &Value) -> String {
    // Try to use the id_field from meta, or find first string field
    let id_field = schema.meta.id_field.clone().or_else(|| {
        schema
            .schema
            .get("properties")
            .and_then(|p| p.as_object())
            .and_then(|props| {
                props.iter().find_map(|(key, val)| {
                    if val.get("type").and_then(|t| t.as_str()) == Some("string")
                        && val.get("format").and_then(|f| f.as_str()) != Some("upload")
                    {
                        Some(key.clone())
                    } else {
                        None
                    }
                })
            })
    });

    if let Some(field) = id_field {
        if let Some(val) = data.get(&field).and_then(|v| v.as_str()) {
            let slugified = slug::slugify(val);
            if !slugified.is_empty() {
                return slugified;
            }
        }
    }

    Uuid::new_v4().to_string()
}
