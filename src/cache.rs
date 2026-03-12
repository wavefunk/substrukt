use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

use crate::content;
use crate::schema;
use crate::state::ContentCache;

/// Populate the cache from disk on startup.
pub fn populate(cache: &ContentCache, schemas_dir: &Path, content_dir: &Path) {
    let schemas = match schema::list_schemas(schemas_dir) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to list schemas for cache: {e}");
            return;
        }
    };

    for s in &schemas {
        let entries = match content::list_entries(content_dir, s) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Failed to list entries for {}: {e}", s.meta.slug);
                continue;
            }
        };
        for entry in entries {
            let key = format!("{}/{}", s.meta.slug, entry.id);
            cache.insert(key, entry.data);
        }
    }

    tracing::info!("Cache populated with {} entries", cache.len());
}

/// Reload all entries for a specific schema.
pub fn reload_schema(
    cache: &ContentCache,
    content_dir: &Path,
    schema: &schema::models::SchemaFile,
) {
    let prefix = format!("{}/", schema.meta.slug);
    // Remove old entries for this schema
    cache.retain(|k, _| !k.starts_with(&prefix));

    // Reload
    match content::list_entries(content_dir, schema) {
        Ok(entries) => {
            for entry in entries {
                let key = format!("{}/{}", schema.meta.slug, entry.id);
                cache.insert(key, entry.data);
            }
        }
        Err(e) => {
            tracing::warn!("Failed to reload cache for {}: {e}", schema.meta.slug);
        }
    }
}

/// Reload a single entry.
pub fn reload_entry(
    cache: &ContentCache,
    content_dir: &Path,
    schema: &schema::models::SchemaFile,
    entry_id: &str,
) {
    let key = format!("{}/{}", schema.meta.slug, entry_id);
    match content::get_entry(content_dir, schema, entry_id) {
        Ok(Some(entry)) => {
            cache.insert(key, entry.data);
        }
        Ok(None) => {
            cache.remove(&key);
        }
        Err(e) => {
            tracing::warn!("Failed to reload cache entry {key}: {e}");
        }
    }
}

/// Clear and rebuild the entire cache.
pub fn rebuild(cache: &ContentCache, schemas_dir: &Path, content_dir: &Path) {
    cache.clear();
    populate(cache, schemas_dir, content_dir);
}

/// Spawn a file watcher that rebuilds the cache on content/schema changes.
/// Returns a guard that keeps the watcher alive; drop it to stop watching.
pub fn spawn_watcher(
    cache: Arc<ContentCache>,
    schemas_dir: PathBuf,
    content_dir: PathBuf,
) -> Option<impl Drop> {
    let cache_for_handler = cache.clone();
    let schemas_for_handler = schemas_dir.clone();
    let content_for_handler = content_dir.clone();

    // Debounce events with a channel — coalesce rapid changes into one rebuild
    let (tx, rx) = std::sync::mpsc::channel();

    let mut watcher = match notify::recommended_watcher(move |res: Result<notify::Event, _>| {
        if let Ok(event) = res {
            if event.kind.is_modify() || event.kind.is_create() || event.kind.is_remove() {
                let _ = tx.send(());
            }
        }
    }) {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!("Failed to create file watcher: {e}");
            return None;
        }
    };

    if let Err(e) = watcher.watch(&schemas_dir, RecursiveMode::Recursive) {
        tracing::warn!("Failed to watch schemas dir: {e}");
    }
    if let Err(e) = watcher.watch(&content_dir, RecursiveMode::Recursive) {
        tracing::warn!("Failed to watch content dir: {e}");
    }

    // Background thread that debounces and rebuilds
    std::thread::spawn(move || {
        loop {
            // Wait for first event
            if rx.recv().is_err() {
                break; // Channel closed, watcher dropped
            }
            // Drain additional events within 200ms window
            while rx.recv_timeout(Duration::from_millis(200)).is_ok() {}

            tracing::debug!("File change detected, rebuilding cache");
            rebuild(&cache_for_handler, &schemas_for_handler, &content_for_handler);
        }
    });

    tracing::info!("File watcher started for schemas and content dirs");
    Some(watcher)
}
