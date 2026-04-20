pub mod app_context;
pub mod audit;
pub mod auth;
pub mod backup;
pub mod cache;
pub mod config;
pub mod content;
pub mod db;
pub mod email;
pub mod history;
pub mod metrics;
pub mod openapi;
pub mod prime;
pub mod rate_limit;
pub mod routes;
pub mod schema;
pub mod state;
pub mod sync;
pub mod templates;
pub mod uploads;
pub mod webhooks;

/// Migrate old single-app layout to multi-app layout.
/// Moves schemas/, content/, uploads/, _history/ into data/default/.
pub fn migrate_single_app_layout(data_dir: &std::path::Path) -> eyre::Result<()> {
    let old_schemas = data_dir.join("schemas");
    let default_dir = data_dir.join("default");

    // If data/schemas/ exists at root, this is the old layout
    if old_schemas.exists() && old_schemas.is_dir() {
        tracing::info!("Migrating single-app layout to multi-app...");
        std::fs::create_dir_all(&default_dir)?;

        let dirs_to_move = ["schemas", "content", "uploads", "_history"];
        let mut moved = Vec::new();

        for dir_name in &dirs_to_move {
            let src = data_dir.join(dir_name);
            let dst = default_dir.join(dir_name);
            if src.exists() {
                if let Err(e) = std::fs::rename(&src, &dst) {
                    // rename fails across filesystems; fall back to copy + delete
                    tracing::warn!("rename failed ({e}), falling back to copy for {dir_name}");
                    if let Err(copy_err) = copy_dir_recursive(&src, &dst) {
                        // Rollback already-moved directories
                        for moved_dir in &moved {
                            let rollback_src = default_dir.join(moved_dir);
                            let rollback_dst = data_dir.join(moved_dir);
                            let _ = std::fs::rename(&rollback_src, &rollback_dst);
                        }
                        eyre::bail!(
                            "Migration failed while moving {dir_name}: {copy_err}. \
                             Rolled back all changes. Server cannot start."
                        );
                    }
                    std::fs::remove_dir_all(&src)?;
                }
                moved.push(*dir_name);
            }
        }
        tracing::info!("Migration complete: data moved to data/default/");
    }
    Ok(())
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> eyre::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
