use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

use crate::content;
use crate::schema;
use crate::state::{ContentCache, EtagCache, OpenApiCache};

/// Populate the cache from disk on startup. Auto-discovers app directories.
pub fn populate(cache: &ContentCache, data_dir: &Path) {
    if !data_dir.exists() {
        return;
    }

    let entries = match std::fs::read_dir(data_dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("Failed to read data dir for cache: {e}");
            return;
        }
    };

    for dir_entry in entries.flatten() {
        if !dir_entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }
        let app_dir = dir_entry.path();
        let schemas_dir = app_dir.join("schemas");
        if !schemas_dir.exists() {
            continue;
        }
        let app_slug = dir_entry.file_name().to_string_lossy().to_string();
        populate_app(cache, &app_dir, &app_slug);
    }

    tracing::info!("Cache populated with {} entries", cache.len());
}

/// Populate cache entries for a single app.
fn populate_app(cache: &ContentCache, app_dir: &Path, app_slug: &str) {
    let schemas_dir = app_dir.join("schemas");
    let content_dir = app_dir.join("content");

    let schemas = match schema::list_schemas(&schemas_dir) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to list schemas for app '{}': {e}", app_slug);
            return;
        }
    };

    for s in &schemas {
        let entries = match content::list_entries(&content_dir, s) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(
                    "Failed to list entries for {}/{}: {e}",
                    app_slug,
                    s.meta.slug
                );
                continue;
            }
        };
        for entry in entries {
            let key = format!("{}/{}/{}", app_slug, s.meta.slug, entry.id);
            cache.insert(key, entry.data);
        }
    }
}

/// Remove all cache entries for an app.
pub fn remove_app(cache: &ContentCache, app_slug: &str) {
    let prefix = format!("{app_slug}/");
    cache.retain(|k, _| !k.starts_with(&prefix));
}

/// Reload all entries for a specific schema within an app.
pub fn reload_schema(
    cache: &ContentCache,
    etag_cache: &EtagCache,
    content_dir: &Path,
    schema: &schema::models::SchemaFile,
    app_slug: &str,
) {
    let prefix = format!("{}/{}/", app_slug, schema.meta.slug);
    // Remove old entries for this schema in this app
    cache.retain(|k, _| !k.starts_with(&prefix));
    etag_cache.retain(|k, _| !k.starts_with(&prefix));

    // Reload
    match content::list_entries(content_dir, schema) {
        Ok(entries) => {
            for entry in entries {
                let key = format!("{}/{}/{}", app_slug, schema.meta.slug, entry.id);
                cache.insert(key, entry.data);
            }
        }
        Err(e) => {
            tracing::warn!(
                "Failed to reload cache for {}/{}: {e}",
                app_slug,
                schema.meta.slug
            );
        }
    }
}

/// Reload a single entry within an app.
pub fn reload_entry(
    cache: &ContentCache,
    etag_cache: &EtagCache,
    content_dir: &Path,
    schema: &schema::models::SchemaFile,
    entry_id: &str,
    app_slug: &str,
) {
    let key = format!("{}/{}/{}", app_slug, schema.meta.slug, entry_id);
    etag_cache.remove(&key);
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

/// Clear and rebuild the entire cache from all apps.
/// Also clears the ETag cache since content has changed.
pub fn rebuild(cache: &ContentCache, etag_cache: &crate::state::EtagCache, data_dir: &Path) {
    cache.clear();
    etag_cache.clear();
    populate(cache, data_dir);
}

/// Spawn a file watcher that rebuilds the cache on content/schema changes.
/// Watches the entire data directory recursively (covers all apps).
/// Also clears the OpenAPI spec cache so it regenerates on next request.
/// Returns a guard that keeps the watcher alive; drop it to stop watching.
pub fn spawn_watcher(
    cache: Arc<ContentCache>,
    etag_cache: Arc<EtagCache>,
    openapi_cache: OpenApiCache,
    data_dir: PathBuf,
) -> Option<impl Drop> {
    let cache_for_handler = cache.clone();
    let etag_for_handler = etag_cache.clone();
    let openapi_for_handler = openapi_cache.clone();
    let data_dir_for_handler = data_dir.clone();

    // Debounce events with a channel -- coalesce rapid changes into one rebuild
    let (tx, rx) = std::sync::mpsc::channel();

    let mut watcher = match notify::recommended_watcher(move |res: Result<notify::Event, _>| {
        if let Ok(event) = res
            && (event.kind.is_modify() || event.kind.is_create() || event.kind.is_remove())
        {
            if event
                .paths
                .iter()
                .all(|p| p.to_string_lossy().contains("/_history/"))
            {
                return;
            }
            let _ = tx.send(());
        }
    }) {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!("Failed to create file watcher: {e}");
            return None;
        }
    };

    if let Err(e) = watcher.watch(&data_dir, RecursiveMode::Recursive) {
        tracing::warn!("Failed to watch data dir: {e}");
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
            rebuild(&cache_for_handler, &etag_for_handler, &data_dir_for_handler);

            // Invalidate the OpenAPI spec cache so it regenerates with new schemas
            if let Ok(mut openapi) = openapi_for_handler.write() {
                *openapi = None;
            }
        }
    });

    tracing::info!("File watcher started for data directory");
    Some(watcher)
}
