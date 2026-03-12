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

    #[cfg(test)]
    pub async fn execute_raw(&self, query: &str) -> eyre::Result<()> {
        sqlx::query(query).execute(self.pool.as_ref()).await?;
        Ok(())
    }

    pub async fn is_dirty(&self, environment: &str) -> eyre::Result<bool> {
        let last_fired: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT last_fired_at FROM webhook_state WHERE environment = ?",
        )
        .bind(environment)
        .fetch_optional(self.pool.as_ref())
        .await?;

        let last_fired_at = match last_fired {
            Some((Some(ts),)) => ts,
            _ => return Ok(true),
        };

        let latest_mutation: (Option<String>,) = sqlx::query_as(
            "SELECT MAX(timestamp) FROM audit_log WHERE action IN ('content_create', 'content_update', 'content_delete', 'schema_create', 'schema_update', 'schema_delete')",
        )
        .fetch_one(self.pool.as_ref())
        .await?;

        match latest_mutation {
            (Some(ts),) => Ok(ts > last_fired_at),
            _ => Ok(false),
        }
    }

    pub async fn mark_fired(&self, environment: &str) -> eyre::Result<String> {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("UPDATE webhook_state SET last_fired_at = ? WHERE environment = ?")
            .bind(&now)
            .bind(environment)
            .execute(self.pool.as_ref())
            .await?;
        Ok(now)
    }

    pub fn log(
        &self,
        actor: &str,
        action: &str,
        resource_type: &str,
        resource_id: &str,
        details: Option<&str>,
    ) {
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./audit_migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn test_is_dirty_when_no_mutations() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        assert!(logger.is_dirty("staging").await.unwrap());
    }

    #[tokio::test]
    async fn test_is_dirty_after_mutation() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        logger.mark_fired("staging").await.unwrap();
        // Insert a mutation with a timestamp in the future (RFC3339 format to match mark_fired)
        let future_ts = (chrono::Utc::now() + chrono::Duration::seconds(10)).to_rfc3339();
        let query = format!("INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id) VALUES ('{future_ts}', 'test', 'content_create', 'content', 'test/1')");
        logger.execute_raw(&query).await.unwrap();
        assert!(logger.is_dirty("staging").await.unwrap());
    }

    #[tokio::test]
    async fn test_not_dirty_after_mark_fired() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        logger
            .execute_raw("INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id) VALUES (datetime('now'), 'test', 'content_create', 'content', 'test/1')")
            .await
            .unwrap();
        logger.mark_fired("staging").await.unwrap();
        assert!(!logger.is_dirty("staging").await.unwrap());
    }

    #[tokio::test]
    async fn test_dirty_ignores_non_mutation_events() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        logger.mark_fired("staging").await.unwrap();
        logger
            .execute_raw("INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id) VALUES (datetime('now', '+1 second'), 'test', 'login', 'session', '')")
            .await
            .unwrap();
        assert!(!logger.is_dirty("staging").await.unwrap());
    }

    #[tokio::test]
    async fn test_staging_and_production_independent() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        logger
            .execute_raw("INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id) VALUES (datetime('now'), 'test', 'content_create', 'content', 'test/1')")
            .await
            .unwrap();
        logger.mark_fired("staging").await.unwrap();
        assert!(!logger.is_dirty("staging").await.unwrap());
        assert!(logger.is_dirty("production").await.unwrap());
    }
}
