pub mod models;

use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;
use std::path::Path;
use std::str::FromStr;

pub async fn init_pool(db_path: &Path) -> eyre::Result<SqlitePool> {
    let url = format!("sqlite:{}?mode=rwc", db_path.display());
    let options = SqliteConnectOptions::from_str(&url)?
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .pragma("foreign_keys", "ON")
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(options).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}
