pub mod form;

use std::path::Path;

use serde_json::Value;
use uuid::Uuid;

use crate::schema::models::{Kind, SchemaFile, StorageMode};

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
        StorageMode::SingleFile => list_single_file_entries(content_dir, slug, &schema.meta.kind),
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

fn list_single_file_entries(
    content_dir: &Path,
    slug: &str,
    kind: &Kind,
) -> eyre::Result<Vec<ContentEntry>> {
    let path = content_dir.join(format!("{slug}.json"));
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)?;
    let items: Vec<Value> = match kind {
        Kind::Single => {
            let obj: Value = serde_json::from_str(&content)?;
            vec![obj]
        }
        Kind::Collection => serde_json::from_str(&content)?,
    };
    Ok(items
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
            let entries = list_single_file_entries(content_dir, slug, &schema.meta.kind)?;
            Ok(entries.into_iter().find(|e| e.id == entry_id))
        }
    }
}

pub fn save_entry(
    content_dir: &Path,
    schema: &SchemaFile,
    entry_id: Option<&str>,
    mut data: Value,
) -> eyre::Result<String> {
    let slug = &schema.meta.slug;

    // Determine _status: respect explicit value, else preserve existing, else draft
    let status = if let Some(explicit) = data.get("_status").and_then(|v| v.as_str()) {
        // Caller explicitly set _status (API use case) — respect it
        match explicit {
            "draft" | "published" => explicit.to_string(),
            _ => "draft".to_string(), // invalid values fall back to draft
        }
    } else if let Some(eid) = entry_id {
        // Update path: preserve existing status from disk
        get_entry(content_dir, schema, eid)
            .ok()
            .flatten()
            .and_then(|e| {
                e.data
                    .get("_status")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "draft".to_string())
    } else {
        // Create path: default to draft
        "draft".to_string()
    };

    // Inject _status into data
    if let Some(obj) = data.as_object_mut() {
        obj.insert("_status".to_string(), Value::String(status));
    }

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

            let id = entry_id
                .map(|s| s.to_string())
                .unwrap_or_else(|| Uuid::new_v4().to_string());

            // Insert _id into data
            let mut data = data;
            if let Some(obj) = data.as_object_mut() {
                obj.insert("_id".to_string(), Value::String(id.clone()));
            }

            if schema.meta.kind == Kind::Single {
                let content = serde_json::to_string_pretty(&data)?;
                std::fs::write(path, content)?;
            } else {
                let mut entries = if path.exists() {
                    let content = std::fs::read_to_string(&path)?;
                    serde_json::from_str::<Vec<Value>>(&content)?
                } else {
                    Vec::new()
                };

                if let Some(existing_id) = entry_id {
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
            }
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
                if schema.meta.kind == Kind::Single {
                    std::fs::remove_file(&path)?;
                } else {
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
    }
    Ok(())
}

/// Set the _status of an entry without modifying its content.
/// Does not create a history snapshot (metadata-only change).
pub fn set_entry_status(
    content_dir: &Path,
    schema: &SchemaFile,
    entry_id: &str,
    status: &str,
) -> eyre::Result<()> {
    if !matches!(status, "draft" | "published") {
        eyre::bail!("Invalid status: {status}. Must be \"draft\" or \"published\".");
    }

    let slug = &schema.meta.slug;
    match schema.meta.storage {
        StorageMode::Directory => {
            let path = content_dir.join(slug).join(format!("{entry_id}.json"));
            if !path.exists() {
                eyre::bail!("Entry not found: {slug}/{entry_id}");
            }
            let content = std::fs::read_to_string(&path)?;
            let mut data: Value = serde_json::from_str(&content)?;
            if let Some(obj) = data.as_object_mut() {
                obj.insert("_status".to_string(), Value::String(status.to_string()));
            }
            std::fs::write(&path, serde_json::to_string_pretty(&data)?)?;
        }
        StorageMode::SingleFile => {
            let path = content_dir.join(format!("{slug}.json"));
            if !path.exists() {
                eyre::bail!("Entry not found: {slug}/{entry_id}");
            }
            if schema.meta.kind == Kind::Single {
                let content = std::fs::read_to_string(&path)?;
                let mut data: Value = serde_json::from_str(&content)?;
                if let Some(obj) = data.as_object_mut() {
                    obj.insert("_status".to_string(), Value::String(status.to_string()));
                }
                std::fs::write(&path, serde_json::to_string_pretty(&data)?)?;
            } else {
                // Collection in single file
                let content = std::fs::read_to_string(&path)?;
                let mut entries: Vec<Value> = serde_json::from_str(&content)?;
                let found = entries.iter_mut().any(|e| {
                    let matches = e
                        .get("_id")
                        .and_then(|v| v.as_str())
                        .is_some_and(|s| s == entry_id);
                    if matches && let Some(obj) = e.as_object_mut() {
                        obj.insert("_status".to_string(), Value::String(status.to_string()));
                    }
                    matches
                });
                if !found {
                    eyre::bail!("Entry not found: {slug}/{entry_id}");
                }
                std::fs::write(&path, serde_json::to_string_pretty(&entries)?)?;
            }
        }
    }
    Ok(())
}

/// Maximum nesting depth for recursive JSON traversal to prevent stack overflow.
const MAX_NESTING_DEPTH: usize = 32;

/// Check if any string value in the JSON data contains the query (case-insensitive).
/// The query must already be lowercased by the caller.
pub fn matches_query(data: &Value, query_lower: &str) -> bool {
    matches_query_inner(data, query_lower, 0)
}

fn matches_query_inner(data: &Value, query_lower: &str, depth: usize) -> bool {
    if depth > MAX_NESTING_DEPTH {
        return false;
    }
    match data {
        Value::String(s) => s.to_lowercase().contains(query_lower),
        Value::Object(map) => map
            .iter()
            .filter(|(k, _)| !k.starts_with('_'))
            .any(|(_, v)| matches_query_inner(v, query_lower, depth + 1)),
        Value::Array(arr) => arr
            .iter()
            .any(|v| matches_query_inner(v, query_lower, depth + 1)),
        _ => false,
    }
}

/// Filter entries by a search query. Case-insensitive substring match on all string values.
pub fn filter_entries(entries: Vec<ContentEntry>, query: &str) -> Vec<ContentEntry> {
    let query_lower = query.to_lowercase();
    entries
        .into_iter()
        .filter(|e| matches_query(&e.data, &query_lower))
        .collect()
}

/// Get the status of an entry. Returns "published" if no _status field (backwards compat).
pub fn get_entry_status(data: &Value) -> &str {
    data.get("_status")
        .and_then(|v| v.as_str())
        .unwrap_or("published")
}

/// Filter entries by status. "all" returns everything.
/// "published" returns entries with _status=published or missing _status (backwards compat).
/// "draft" returns only entries with _status=draft.
pub fn filter_by_status(entries: Vec<ContentEntry>, status: &str) -> Vec<ContentEntry> {
    match status {
        "all" => entries,
        "draft" => entries
            .into_iter()
            .filter(|e| get_entry_status(&e.data) == "draft")
            .collect(),
        _ => entries
            .into_iter()
            .filter(|e| get_entry_status(&e.data) == "published")
            .collect(),
    }
}

/// Strip `_status` from entry data for API responses.
pub fn strip_internal_status(data: &Value) -> Value {
    let mut data = data.clone();
    if let Some(obj) = data.as_object_mut() {
        obj.remove("_status");
    }
    data
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
    if let Some(props) = schema.get_mut("properties").and_then(|p| p.as_object_mut()) {
        for (_key, prop) in props.iter_mut() {
            let is_upload = prop.get("type").and_then(|t| t.as_str()) == Some("string")
                && prop.get("format").and_then(|f| f.as_str()) == Some("upload");
            if is_upload {
                // Allow string or object
                if let Some(obj) = prop.as_object_mut() {
                    obj.remove("type");
                    obj.insert("type".to_string(), serde_json::json!(["string", "object"]));
                }
            }
        }
    }
    schema
}

fn generate_entry_id(schema: &SchemaFile, data: &Value) -> String {
    // Try to use the id_field from meta, or find a suitable string field.
    // Prefer well-known naming fields (title, name, label, etc.) over
    // alphabetical iteration, which would pick "body" before "title".
    let id_field = schema.meta.id_field.clone().or_else(|| {
        schema
            .schema
            .get("properties")
            .and_then(|p| p.as_object())
            .and_then(|props| {
                let is_plain_string = |val: &Value| {
                    val.get("type").and_then(|t| t.as_str()) == Some("string")
                        && !matches!(
                            val.get("format").and_then(|f| f.as_str()),
                            Some("upload") | Some("reference")
                        )
                };

                // Check well-known title/name fields first, in priority order
                const PREFERRED_FIELDS: &[&str] =
                    &["title", "name", "label", "heading", "subject", "slug"];
                for &field in PREFERRED_FIELDS {
                    if let Some(val) = props.get(field) {
                        if is_plain_string(val) {
                            return Some(field.to_string());
                        }
                    }
                }

                // Fall back to first plain string field (alphabetical via BTreeMap)
                props.iter().find_map(|(key, val)| {
                    if is_plain_string(val) {
                        Some(key.clone())
                    } else {
                        None
                    }
                })
            })
    });

    if let Some(field) = id_field
        && let Some(val) = data.get(&field).and_then(|v| v.as_str())
    {
        let slugified = slug::slugify(val);
        if !slugified.is_empty() {
            return slugified;
        }
    }

    Uuid::new_v4().to_string()
}

/// Render a markdown string to sanitized HTML using pulldown-cmark with GFM extensions.
/// Raw HTML in the markdown input is stripped (not passed through) as a security measure.
/// Output is wrapped in `<div class="sk-markdown">...</div>` for CSS scoping.
/// Returns empty string for empty input (no wrapper).
pub fn render_markdown(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    use pulldown_cmark::{Event, Options, Parser, html};

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(input, options);
    // Strip raw HTML events to prevent XSS in rendered output
    let parser = parser.filter(|event| !matches!(event, Event::Html(_) | Event::InlineHtml(_)));

    let mut html_output = String::from("<div class=\"sk-markdown\">");
    html::push_html(&mut html_output, parser);
    html_output.push_str("</div>");
    html_output
}

/// Walk a JSON value and render all markdown fields to HTML, based on the schema.
/// Only transforms fields where the schema declares `"type": "string", "format": "markdown"`.
pub fn render_markdown_fields(data: &mut Value, schema: &Value) {
    render_markdown_fields_inner(data, schema, 0);
}

fn render_markdown_fields_inner(data: &mut Value, schema: &Value, depth: usize) {
    if depth > MAX_NESTING_DEPTH {
        return;
    }
    let Some(props) = schema.get("properties").and_then(|p| p.as_object()) else {
        return;
    };
    let Some(obj) = data.as_object_mut() else {
        return;
    };
    for (key, prop_schema) in props {
        let field_type = prop_schema.get("type").and_then(|t| t.as_str());
        let format = prop_schema.get("format").and_then(|f| f.as_str());

        match (field_type, format) {
            (Some("string"), Some("markdown")) => {
                if let Some(md) = obj.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()) {
                    let html = render_markdown(&md);
                    obj.insert(key.clone(), Value::String(html));
                }
            }
            (Some("object"), _) => {
                if let Some(nested) = obj.get_mut(key) {
                    render_markdown_fields_inner(nested, prop_schema, depth + 1);
                }
            }
            (Some("array"), _) => {
                if let Some(items_schema) = prop_schema.get("items")
                    && let Some(Value::Array(arr)) = obj.get_mut(key)
                {
                    for item in arr.iter_mut() {
                        render_markdown_fields_inner(item, items_schema, depth + 1);
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn test_schema(kind: Kind, storage: StorageMode) -> SchemaFile {
        SchemaFile {
            meta: crate::schema::models::SubstruktMeta {
                title: "Test".to_string(),
                slug: "test".to_string(),
                kind,
                storage,
                id_field: None,
                render: None,
            },
            schema: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" }
                }
            }),
        }
    }

    #[test]
    fn save_entry_create_injects_draft_status() {
        let tmp = TempDir::new().unwrap();
        let schema = test_schema(Kind::Collection, StorageMode::Directory);
        let data = json!({"title": "Hello"});
        let id = save_entry(tmp.path(), &schema, None, data).unwrap();

        let entry = get_entry(tmp.path(), &schema, &id).unwrap().unwrap();
        assert_eq!(
            entry.data.get("_status").and_then(|v| v.as_str()),
            Some("draft"),
            "new entry should have _status: draft"
        );
    }

    #[test]
    fn save_entry_update_preserves_status() {
        let tmp = TempDir::new().unwrap();
        let schema = test_schema(Kind::Collection, StorageMode::Directory);

        // Create entry (gets _status: draft)
        let data = json!({"title": "Hello"});
        let id = save_entry(tmp.path(), &schema, None, data).unwrap();

        // Manually set to published
        let mut entry = get_entry(tmp.path(), &schema, &id).unwrap().unwrap();
        entry
            .data
            .as_object_mut()
            .unwrap()
            .insert("_status".to_string(), json!("published"));
        let path = tmp.path().join("test").join(format!("{id}.json"));
        std::fs::write(&path, serde_json::to_string_pretty(&entry.data).unwrap()).unwrap();

        // Update via save_entry
        let new_data = json!({"title": "Updated"});
        save_entry(tmp.path(), &schema, Some(&id), new_data).unwrap();

        let updated = get_entry(tmp.path(), &schema, &id).unwrap().unwrap();
        assert_eq!(
            updated.data.get("_status").and_then(|v| v.as_str()),
            Some("published"),
            "updated entry should preserve _status: published"
        );
        assert_eq!(
            updated.data.get("title").and_then(|v| v.as_str()),
            Some("Updated")
        );
    }

    #[test]
    fn save_entry_update_no_existing_falls_back_to_draft() {
        let tmp = TempDir::new().unwrap();
        let schema = test_schema(Kind::Single, StorageMode::SingleFile);

        // First upsert — no existing entry
        let data = json!({"title": "Settings"});
        save_entry(tmp.path(), &schema, Some("_single"), data).unwrap();

        let entry = get_entry(tmp.path(), &schema, "_single").unwrap().unwrap();
        assert_eq!(
            entry.data.get("_status").and_then(|v| v.as_str()),
            Some("draft"),
            "first upsert with no existing should default to draft"
        );
    }

    #[test]
    fn strip_internal_status_removes_status_only() {
        let data = json!({"_id": "test", "_status": "draft", "title": "Hello"});
        let stripped = strip_internal_status(&data);
        assert!(
            stripped.get("_status").is_none(),
            "_status should be stripped"
        );
        assert!(stripped.get("_id").is_some(), "_id should remain");
        assert!(stripped.get("title").is_some(), "title should remain");
    }

    #[test]
    fn matches_query_skips_underscore_prefixed_keys() {
        let data = json!({"_status": "draft", "_id": "my-id", "title": "Hello World"});
        assert!(!matches_query(&data, "draft"), "should not match _status");
        assert!(!matches_query(&data, "my-id"), "should not match _id");
        assert!(matches_query(&data, "hello"), "should match title");
    }

    #[test]
    fn missing_status_treated_as_published() {
        // Entry data without _status (legacy)
        let data = json!({"title": "Old entry"});
        let status = data
            .get("_status")
            .and_then(|v| v.as_str())
            .unwrap_or("published");
        assert_eq!(status, "published");
    }

    #[test]
    fn filter_by_status_published_only() {
        let entries = vec![
            ContentEntry {
                id: "a".into(),
                data: json!({"_status": "draft", "title": "Draft"}),
            },
            ContentEntry {
                id: "b".into(),
                data: json!({"_status": "published", "title": "Published"}),
            },
            ContentEntry {
                id: "c".into(),
                data: json!({"title": "Legacy"}),
            },
        ];
        let filtered = filter_by_status(entries, "published");
        assert_eq!(
            filtered.len(),
            2,
            "should return published + legacy (no _status = published)"
        );
        assert!(filtered.iter().any(|e| e.id == "b"));
        assert!(filtered.iter().any(|e| e.id == "c"));
    }

    #[test]
    fn filter_by_status_draft_only() {
        let entries = vec![
            ContentEntry {
                id: "a".into(),
                data: json!({"_status": "draft", "title": "Draft"}),
            },
            ContentEntry {
                id: "b".into(),
                data: json!({"_status": "published", "title": "Published"}),
            },
        ];
        let filtered = filter_by_status(entries, "draft");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "a");
    }

    #[test]
    fn filter_by_status_all_returns_everything() {
        let entries = vec![
            ContentEntry {
                id: "a".into(),
                data: json!({"_status": "draft"}),
            },
            ContentEntry {
                id: "b".into(),
                data: json!({"_status": "published"}),
            },
        ];
        let filtered = filter_by_status(entries, "all");
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn get_entry_status_returns_correct_status() {
        let data_draft = json!({"_status": "draft", "title": "Test"});
        let data_published = json!({"_status": "published", "title": "Test"});
        let data_legacy = json!({"title": "Test"});

        assert_eq!(get_entry_status(&data_draft), "draft");
        assert_eq!(get_entry_status(&data_published), "published");
        assert_eq!(get_entry_status(&data_legacy), "published");
    }

    #[test]
    fn save_entry_explicit_status_published_on_create() {
        let tmp = TempDir::new().unwrap();
        let schema = test_schema(Kind::Collection, StorageMode::Directory);
        let data = json!({"title": "Hello", "_status": "published"});
        let id = save_entry(tmp.path(), &schema, None, data).unwrap();

        let entry = get_entry(tmp.path(), &schema, &id).unwrap().unwrap();
        assert_eq!(
            entry.data.get("_status").and_then(|v| v.as_str()),
            Some("published"),
            "explicit _status in data should be respected on create"
        );
    }

    #[test]
    fn save_entry_explicit_status_draft_on_update() {
        let tmp = TempDir::new().unwrap();
        let schema = test_schema(Kind::Collection, StorageMode::Directory);
        let id = save_entry(tmp.path(), &schema, None, json!({"title": "Hello"})).unwrap();

        // Publish the entry via set_entry_status
        set_entry_status(tmp.path(), &schema, &id, "published").unwrap();

        // Update with explicit _status: "draft" — should override existing published status
        let data = json!({"title": "Updated", "_status": "draft"});
        save_entry(tmp.path(), &schema, Some(&id), data).unwrap();

        let entry = get_entry(tmp.path(), &schema, &id).unwrap().unwrap();
        assert_eq!(
            entry.data.get("_status").and_then(|v| v.as_str()),
            Some("draft"),
            "explicit _status: draft should override existing published"
        );
    }

    #[test]
    fn save_entry_explicit_invalid_status_falls_back_to_draft() {
        let tmp = TempDir::new().unwrap();
        let schema = test_schema(Kind::Collection, StorageMode::Directory);
        let data = json!({"title": "Hello", "_status": "archived"});
        let id = save_entry(tmp.path(), &schema, None, data).unwrap();

        let entry = get_entry(tmp.path(), &schema, &id).unwrap().unwrap();
        assert_eq!(
            entry.data.get("_status").and_then(|v| v.as_str()),
            Some("draft"),
            "invalid _status value should normalize to draft"
        );
    }

    #[test]
    fn set_entry_status_directory_mode() {
        let tmp = TempDir::new().unwrap();
        let schema = test_schema(Kind::Collection, StorageMode::Directory);
        let data = json!({"title": "Hello"});
        let id = save_entry(tmp.path(), &schema, None, data).unwrap();

        // Starts as draft
        let entry = get_entry(tmp.path(), &schema, &id).unwrap().unwrap();
        assert_eq!(get_entry_status(&entry.data), "draft");

        // Publish it
        set_entry_status(tmp.path(), &schema, &id, "published").unwrap();
        let entry = get_entry(tmp.path(), &schema, &id).unwrap().unwrap();
        assert_eq!(get_entry_status(&entry.data), "published");
        // Content untouched
        assert_eq!(
            entry.data.get("title").and_then(|v| v.as_str()),
            Some("Hello")
        );

        // Unpublish it
        set_entry_status(tmp.path(), &schema, &id, "draft").unwrap();
        let entry = get_entry(tmp.path(), &schema, &id).unwrap().unwrap();
        assert_eq!(get_entry_status(&entry.data), "draft");
    }

    #[test]
    fn set_entry_status_single_file_single() {
        let tmp = TempDir::new().unwrap();
        let schema = test_schema(Kind::Single, StorageMode::SingleFile);
        save_entry(
            tmp.path(),
            &schema,
            Some("_single"),
            json!({"title": "Settings"}),
        )
        .unwrap();

        set_entry_status(tmp.path(), &schema, "_single", "published").unwrap();
        let entry = get_entry(tmp.path(), &schema, "_single").unwrap().unwrap();
        assert_eq!(get_entry_status(&entry.data), "published");
    }

    #[test]
    fn set_entry_status_single_file_collection() {
        let tmp = TempDir::new().unwrap();
        let schema = test_schema(Kind::Collection, StorageMode::SingleFile);
        let id_a = save_entry(tmp.path(), &schema, None, json!({"title": "A"})).unwrap();
        let id_b = save_entry(tmp.path(), &schema, None, json!({"title": "B"})).unwrap();

        // Publish only entry A
        set_entry_status(tmp.path(), &schema, &id_a, "published").unwrap();

        let entry_a = get_entry(tmp.path(), &schema, &id_a).unwrap().unwrap();
        let entry_b = get_entry(tmp.path(), &schema, &id_b).unwrap().unwrap();
        assert_eq!(get_entry_status(&entry_a.data), "published");
        assert_eq!(
            get_entry_status(&entry_b.data),
            "draft",
            "other entry should be untouched"
        );
    }

    #[test]
    fn set_entry_status_nonexistent_directory() {
        let tmp = TempDir::new().unwrap();
        let schema = test_schema(Kind::Collection, StorageMode::Directory);
        let result = set_entry_status(tmp.path(), &schema, "nonexistent", "published");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn set_entry_status_nonexistent_single_file_collection() {
        let tmp = TempDir::new().unwrap();
        let schema = test_schema(Kind::Collection, StorageMode::SingleFile);
        // Create one entry so the file exists
        save_entry(tmp.path(), &schema, None, json!({"title": "A"})).unwrap();

        let result = set_entry_status(tmp.path(), &schema, "nonexistent", "published");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn set_entry_status_nonexistent_file() {
        let tmp = TempDir::new().unwrap();
        let schema = test_schema(Kind::Single, StorageMode::SingleFile);
        // File does not exist at all
        let result = set_entry_status(tmp.path(), &schema, "_single", "published");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn set_entry_status_invalid_status() {
        let tmp = TempDir::new().unwrap();
        let schema = test_schema(Kind::Collection, StorageMode::Directory);
        save_entry(tmp.path(), &schema, None, json!({"title": "Hello"})).unwrap();

        let result = set_entry_status(tmp.path(), &schema, "hello", "archived");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid status"));
    }

    #[test]
    fn set_entry_status_idempotent() {
        let tmp = TempDir::new().unwrap();
        let schema = test_schema(Kind::Collection, StorageMode::Directory);
        let id = save_entry(tmp.path(), &schema, None, json!({"title": "Hello"})).unwrap();

        // Entry starts as draft — unpublishing again should succeed (idempotent)
        set_entry_status(tmp.path(), &schema, &id, "draft").unwrap();
        let entry = get_entry(tmp.path(), &schema, &id).unwrap().unwrap();
        assert_eq!(get_entry_status(&entry.data), "draft");
    }

    #[test]
    fn set_entry_status_adds_field_to_legacy_entry() {
        let tmp = TempDir::new().unwrap();
        let schema = test_schema(Kind::Collection, StorageMode::Directory);
        // Write a legacy entry with no _status field
        let dir = tmp.path().join("test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("legacy.json"), r#"{"title": "Old"}"#).unwrap();

        set_entry_status(tmp.path(), &schema, "legacy", "published").unwrap();
        let entry = get_entry(tmp.path(), &schema, "legacy").unwrap().unwrap();
        assert_eq!(get_entry_status(&entry.data), "published");
        assert_eq!(
            entry.data.get("title").and_then(|v| v.as_str()),
            Some("Old")
        );
    }

    #[test]
    fn render_markdown_basic() {
        let html =
            render_markdown("# Hello\n\nThis is **bold** and a [link](https://example.com).");
        assert!(html.starts_with("<div class=\"sk-markdown\">"));
        assert!(html.ends_with("</div>"));
        assert!(html.contains("<h1>Hello</h1>"));
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<a href=\"https://example.com\">link</a>"));
    }

    #[test]
    fn render_markdown_empty_string() {
        assert_eq!(render_markdown(""), "");
    }

    #[test]
    fn render_markdown_gfm_table() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        let html = render_markdown(md);
        assert!(html.contains("<table>"));
        assert!(html.contains("<td>1</td>"));
    }

    #[test]
    fn render_markdown_gfm_strikethrough() {
        let html = render_markdown("~~deleted~~");
        assert!(html.contains("<del>deleted</del>"));
    }

    #[test]
    fn render_markdown_gfm_tasklist() {
        let html = render_markdown("- [x] done\n- [ ] todo");
        assert!(html.contains("type=\"checkbox\""));
        assert!(html.contains("checked"));
    }

    #[test]
    fn render_markdown_strips_raw_html() {
        let html = render_markdown("Hello <script>alert('xss')</script> world");
        assert!(!html.contains("<script>"));
        assert!(!html.contains("</script>"));
        assert!(html.contains("Hello"));
        assert!(html.contains("world"));
    }

    #[test]
    fn render_markdown_strips_inline_html_tags() {
        let html = render_markdown("Hello <b>bold</b> world");
        assert!(!html.contains("<b>"));
        assert!(!html.contains("</b>"));
        // The text content is preserved (without the tags)
        assert!(html.contains("Hello"));
        assert!(html.contains("world"));
    }

    #[test]
    fn render_markdown_fields_transforms_markdown_only() {
        let schema = json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "title": "Title" },
                "body": { "type": "string", "format": "markdown", "title": "Body" },
                "count": { "type": "number" },
                "active": { "type": "boolean" }
            }
        });
        let mut data = json!({
            "title": "Hello",
            "body": "**bold**",
            "count": 42,
            "active": true
        });
        render_markdown_fields(&mut data, &schema);

        // Markdown field is rendered
        assert!(
            data["body"]
                .as_str()
                .unwrap()
                .contains("<strong>bold</strong>")
        );
        // Other fields are untouched
        assert_eq!(data["title"], "Hello");
        assert_eq!(data["count"], 42);
        assert_eq!(data["active"], true);
    }

    #[test]
    fn render_markdown_fields_nested_object() {
        let schema = json!({
            "type": "object",
            "properties": {
                "meta": {
                    "type": "object",
                    "properties": {
                        "description": { "type": "string", "format": "markdown" }
                    }
                }
            }
        });
        let mut data = json!({
            "meta": {
                "description": "# Heading"
            }
        });
        render_markdown_fields(&mut data, &schema);
        assert!(
            data["meta"]["description"]
                .as_str()
                .unwrap()
                .contains("<h1>Heading</h1>")
        );
    }

    #[test]
    fn render_markdown_fields_array_of_objects() {
        let schema = json!({
            "type": "object",
            "properties": {
                "sections": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "body": { "type": "string", "format": "markdown" }
                        }
                    }
                }
            }
        });
        let mut data = json!({
            "sections": [
                { "body": "**first**" },
                { "body": "*second*" }
            ]
        });
        render_markdown_fields(&mut data, &schema);
        assert!(
            data["sections"][0]["body"]
                .as_str()
                .unwrap()
                .contains("<strong>first</strong>")
        );
        assert!(
            data["sections"][1]["body"]
                .as_str()
                .unwrap()
                .contains("<em>second</em>")
        );
    }

    #[test]
    fn render_markdown_fields_skips_null() {
        let schema = json!({
            "type": "object",
            "properties": {
                "body": { "type": "string", "format": "markdown" }
            }
        });
        let mut data = json!({ "body": null });
        render_markdown_fields(&mut data, &schema);
        assert!(data["body"].is_null());
    }

    #[test]
    fn render_markdown_fields_skips_non_markdown_format() {
        let schema = json!({
            "type": "object",
            "properties": {
                "notes": { "type": "string", "format": "textarea" },
                "body": { "type": "string", "format": "markdown" }
            }
        });
        let mut data = json!({
            "notes": "**not rendered**",
            "body": "**rendered**"
        });
        render_markdown_fields(&mut data, &schema);
        assert_eq!(data["notes"], "**not rendered**");
        assert!(
            data["body"]
                .as_str()
                .unwrap()
                .contains("<strong>rendered</strong>")
        );
    }

    #[test]
    fn render_markdown_fields_respects_depth_limit() {
        // Build a schema nested 40 levels deep (exceeds MAX_NESTING_DEPTH of 32)
        let mut schema = json!({
            "type": "object",
            "properties": {
                "body": { "type": "string", "format": "markdown" }
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
        let mut data = json!({ "nested": {} });
        // Should not panic or stack overflow -- just silently stops at depth limit
        render_markdown_fields(&mut data, &schema);
    }

    #[test]
    fn render_markdown_code_blocks() {
        let md = "```rust\nfn main() {}\n```";
        let html = render_markdown(md);
        assert!(html.contains("<pre>"), "expected <pre>, got: {html}");
        assert!(html.contains("<code"), "expected <code>, got: {html}");
        assert!(
            html.contains("fn main() {}"),
            "expected code content, got: {html}"
        );
    }

    #[test]
    fn render_markdown_strips_iframe() {
        let html = render_markdown("Before <iframe src=\"https://evil.com\"></iframe> After");
        assert!(!html.contains("<iframe"), "iframe should be stripped");
        assert!(html.contains("Before"));
        assert!(html.contains("After"));
    }

    #[test]
    fn render_markdown_strips_event_handler_attributes() {
        // <img> with onerror is raw HTML and should be stripped entirely
        let html = render_markdown("<img src=x onerror=alert(1)>");
        assert!(
            !html.contains("onerror"),
            "onerror attribute should be stripped along with the tag"
        );
    }

    #[test]
    fn render_markdown_fields_non_object_data_is_noop() {
        let schema = json!({
            "type": "object",
            "properties": {
                "body": { "type": "string", "format": "markdown" }
            }
        });
        // Data is a string, not an object -- should not panic
        let mut data = json!("just a string");
        render_markdown_fields(&mut data, &schema);
        assert_eq!(data, json!("just a string"));

        // Data is an array
        let mut data = json!(["item1", "item2"]);
        render_markdown_fields(&mut data, &schema);
        assert_eq!(data, json!(["item1", "item2"]));

        // Data is null
        let mut data = json!(null);
        render_markdown_fields(&mut data, &schema);
        assert!(data.is_null());
    }

    #[test]
    fn render_markdown_fields_missing_field_in_data() {
        // Schema declares a markdown field, but data doesn't have it
        let schema = json!({
            "type": "object",
            "properties": {
                "title": { "type": "string" },
                "body": { "type": "string", "format": "markdown" }
            }
        });
        let mut data = json!({"title": "Hello"});
        // Should not panic -- missing body field is silently skipped
        render_markdown_fields(&mut data, &schema);
        assert_eq!(data["title"], "Hello");
        assert!(data.get("body").is_none());
    }

    #[test]
    fn render_markdown_fields_plain_string_no_format() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            }
        });
        let mut data = json!({"name": "**not rendered**"});
        render_markdown_fields(&mut data, &schema);
        assert_eq!(data["name"], "**not rendered**");
    }

    #[test]
    fn render_markdown_fields_multiple_markdown_fields() {
        let schema = json!({
            "type": "object",
            "properties": {
                "intro": { "type": "string", "format": "markdown" },
                "body": { "type": "string", "format": "markdown" },
                "footer": { "type": "string", "format": "markdown" }
            }
        });
        let mut data = json!({
            "intro": "# Intro",
            "body": "**body text**",
            "footer": "*footer*"
        });
        render_markdown_fields(&mut data, &schema);
        assert!(data["intro"].as_str().unwrap().contains("<h1>Intro</h1>"));
        assert!(
            data["body"]
                .as_str()
                .unwrap()
                .contains("<strong>body text</strong>")
        );
        assert!(data["footer"].as_str().unwrap().contains("<em>footer</em>"));
    }

    #[test]
    fn render_markdown_fields_schema_without_properties() {
        // Schema with no properties key -- should not panic
        let schema = json!({"type": "object"});
        let mut data = json!({"body": "**test**"});
        render_markdown_fields(&mut data, &schema);
        assert_eq!(data["body"], "**test**");
    }

    #[test]
    fn render_markdown_fields_array_with_no_items_schema() {
        // Array field without items schema should not attempt to recurse
        let schema = json!({
            "type": "object",
            "properties": {
                "tags": { "type": "array" }
            }
        });
        let mut data = json!({"tags": ["one", "two"]});
        render_markdown_fields(&mut data, &schema);
        assert_eq!(data["tags"], json!(["one", "two"]));
    }
}
