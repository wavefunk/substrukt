use std::path::Path;
use std::sync::Arc;

use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;
use std::str::FromStr;

pub async fn init_pool(db_path: &Path) -> eyre::Result<SqlitePool> {
    let url = format!("sqlite:{}?mode=rwc", db_path.display());
    let options = SqliteConnectOptions::from_str(&url)?
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(options).await?;
    sqlx::migrate!("./audit_migrations").run(&pool).await?;
    Ok(pool)
}

#[derive(Clone)]
pub struct AuditLogger {
    pool: Arc<SqlitePool>,
}

impl AuditLogger {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }

    pub fn log(&self, actor: &str, action: &str, resource_type: &str, resource_id: &str, details: Option<&str>) {
        let pool = self.pool.clone();
        let timestamp = chrono::Utc::now().to_rfc3339();
        let actor = actor.to_string();
        let action = action.to_string();
        let resource_type = resource_type.to_string();
        let resource_id = resource_id.to_string();
        let details = details.map(|s| s.to_string());

        tokio::spawn(async move {
            let result = sqlx::query(
                "INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id, details) VALUES (?, ?, ?, ?, ?, ?)"
            )
            .bind(&timestamp)
            .bind(&actor)
            .bind(&action)
            .bind(&resource_type)
            .bind(&resource_id)
            .bind(&details)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = result {
                tracing::warn!("Failed to write audit log: {e}");
            }
        });
    }
}
