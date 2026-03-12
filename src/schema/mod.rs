pub mod models;

use std::path::Path;

use crate::schema::models::{SchemaFile, SubstruktMeta};

pub fn list_schemas(schemas_dir: &Path) -> eyre::Result<Vec<SchemaFile>> {
    let mut schemas = Vec::new();
    if !schemas_dir.exists() {
        return Ok(schemas);
    }
    for entry in std::fs::read_dir(schemas_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            match load_schema(&path) {
                Ok(schema) => schemas.push(schema),
                Err(e) => {
                    tracing::warn!("Failed to load schema {}: {e}", path.display());
                }
            }
        }
    }
    schemas.sort_by(|a, b| a.meta.title.cmp(&b.meta.title));
    Ok(schemas)
}

pub fn load_schema(path: &Path) -> eyre::Result<SchemaFile> {
    let content = std::fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&content)?;
    let meta = parse_meta(&value)?;
    Ok(SchemaFile {
        meta,
        schema: value,
    })
}

pub fn get_schema(schemas_dir: &Path, slug: &str) -> eyre::Result<Option<SchemaFile>> {
    if !is_valid_slug(slug) {
        return Ok(None);
    }
    let path = schemas_dir.join(format!("{slug}.json"));
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(load_schema(&path)?))
}

pub fn save_schema(schemas_dir: &Path, slug: &str, schema: &serde_json::Value) -> eyre::Result<()> {
    if !is_valid_slug(slug) {
        eyre::bail!("Invalid slug: {slug}");
    }
    let path = schemas_dir.join(format!("{slug}.json"));
    let content = serde_json::to_string_pretty(schema)?;
    std::fs::write(path, content)?;
    Ok(())
}

pub fn delete_schema(schemas_dir: &Path, slug: &str) -> eyre::Result<()> {
    if !is_valid_slug(slug) {
        return Ok(());
    }
    let path = schemas_dir.join(format!("{slug}.json"));
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

fn parse_meta(value: &serde_json::Value) -> eyre::Result<SubstruktMeta> {
    let ext = value
        .get("x-substrukt")
        .ok_or_else(|| eyre::eyre!("Missing x-substrukt extension"))?;
    let meta: SubstruktMeta = serde_json::from_value(ext.clone())?;
    Ok(meta)
}

pub fn validate_schema(schema: &serde_json::Value) -> eyre::Result<()> {
    // Check that x-substrukt is present
    let meta = parse_meta(schema)?;

    // Validate slug: alphanumeric, hyphens, underscores only
    if !meta.slug.is_empty() && !is_valid_slug(&meta.slug) {
        eyre::bail!(
            "Invalid slug '{}': must contain only lowercase letters, numbers, and hyphens",
            meta.slug
        );
    }

    // Check that it's a valid JSON Schema by trying to compile it
    jsonschema::validator_for(schema).map_err(|e| eyre::eyre!("Invalid JSON Schema: {e}"))?;

    Ok(())
}

/// Validate that a slug is safe for use in file paths and URLs.
pub fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 128
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && !slug.starts_with('-')
        && !slug.starts_with('.')
        && !slug.contains("..")
}

/// Count properties in a schema
pub fn property_count(schema: &serde_json::Value) -> usize {
    schema
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|o| o.len())
        .unwrap_or(0)
}
