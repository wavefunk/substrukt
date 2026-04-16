use std::path::Path;
use std::sync::Arc;

use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;
use std::str::FromStr;

pub async fn init_pool(db_path: &Path) -> eyre::Result<SqlitePool> {
    let url = format!("sqlite:{}?mode=rwc", db_path.display());
    let options = SqliteConnectOptions::from_str(&url)?
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .create_if_missing(true)
        .pragma("foreign_keys", "ON");
    let pool = SqlitePool::connect_with(options).await?;
    sqlx::migrate!("./audit_migrations").run(&pool).await?;
    Ok(pool)
}

#[derive(Clone)]
pub struct AuditLogger {
    pool: Arc<SqlitePool>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Deployment {
    pub id: i64,
    pub app_id: Option<i64>,
    pub name: String,
    pub slug: String,
    pub webhook_url: String,
    pub webhook_auth_token: Option<String>,
    pub include_drafts: bool,
    pub auto_deploy: bool,
    pub debounce_seconds: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WebhookHistoryGroup {
    pub id: i64,
    pub deployment_id: i64,
    pub deployment_name: String,
    pub deployment_slug: String,
    pub trigger_source: String,
    pub status: String,
    pub http_status: Option<i32>,
    pub error_message: Option<String>,
    pub response_time_ms: Option<i64>,
    pub attempt_count: i32,
    pub group_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AuditLogEntry {
    pub id: i64,
    pub timestamp: String,
    pub actor: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    pub details: Option<String>,
    pub app_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BackupConfig {
    pub frequency_hours: i64,
    pub retention_count: i64,
    pub enabled: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BackupRecord {
    pub id: i64,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub status: String,
    pub trigger_source: String,
    pub error_message: Option<String>,
    pub size_bytes: Option<i64>,
    pub s3_key: Option<String>,
    pub manifest: Option<String>,
}

impl AuditLogger {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }

    pub fn pool_ref(&self) -> &SqlitePool {
        self.pool.as_ref()
    }

    #[cfg(test)]
    pub async fn execute_raw(&self, query: &str) -> eyre::Result<()> {
        let query = query.to_string();
        sqlx::query(sqlx::AssertSqlSafe(query))
            .execute(self.pool.as_ref())
            .await?;
        Ok(())
    }

    // ── Deployment CRUD ──────────────────────────────────────────

    pub async fn create_deployment(
        &self,
        app_id: i64,
        name: &str,
        slug: &str,
        webhook_url: &str,
        webhook_auth_token: Option<&str>,
        include_drafts: bool,
        auto_deploy: bool,
        debounce_seconds: i64,
    ) -> eyre::Result<Deployment> {
        let now = chrono::Utc::now().to_rfc3339();
        let include_drafts_i = if include_drafts { 1i32 } else { 0 };
        let auto_deploy_i = if auto_deploy { 1i32 } else { 0 };
        let result = sqlx::query(
            "INSERT INTO deployments (app_id, name, slug, webhook_url, webhook_auth_token, include_drafts, auto_deploy, debounce_seconds, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(app_id)
        .bind(name)
        .bind(slug)
        .bind(webhook_url)
        .bind(webhook_auth_token)
        .bind(include_drafts_i)
        .bind(auto_deploy_i)
        .bind(debounce_seconds)
        .bind(&now)
        .bind(&now)
        .execute(self.pool.as_ref())
        .await?;

        let id = result.last_insert_rowid();
        Ok(Deployment {
            id,
            app_id: Some(app_id),
            name: name.to_string(),
            slug: slug.to_string(),
            webhook_url: webhook_url.to_string(),
            webhook_auth_token: webhook_auth_token.map(|s| s.to_string()),
            include_drafts,
            auto_deploy,
            debounce_seconds,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    pub async fn get_deployment_by_slug(&self, slug: &str) -> eyre::Result<Option<Deployment>> {
        let row: Option<(i64, Option<i64>, String, String, String, Option<String>, i32, i32, i64, String, String)> =
            sqlx::query_as(
                "SELECT id, app_id, name, slug, webhook_url, webhook_auth_token, include_drafts, auto_deploy, debounce_seconds, created_at, updated_at FROM deployments WHERE slug = ?"
            )
            .bind(slug)
            .fetch_optional(self.pool.as_ref())
            .await?;

        Ok(row.map(
            |(
                id,
                app_id,
                name,
                slug,
                webhook_url,
                webhook_auth_token,
                include_drafts,
                auto_deploy,
                debounce_seconds,
                created_at,
                updated_at,
            )| {
                Deployment {
                    id,
                    app_id,
                    name,
                    slug,
                    webhook_url,
                    webhook_auth_token,
                    include_drafts: include_drafts != 0,
                    auto_deploy: auto_deploy != 0,
                    debounce_seconds,
                    created_at,
                    updated_at,
                }
            },
        ))
    }

    pub async fn get_deployment_by_id(&self, id: i64) -> eyre::Result<Option<Deployment>> {
        let row: Option<(i64, Option<i64>, String, String, String, Option<String>, i32, i32, i64, String, String)> =
            sqlx::query_as(
                "SELECT id, app_id, name, slug, webhook_url, webhook_auth_token, include_drafts, auto_deploy, debounce_seconds, created_at, updated_at FROM deployments WHERE id = ?"
            )
            .bind(id)
            .fetch_optional(self.pool.as_ref())
            .await?;

        Ok(row.map(
            |(
                id,
                app_id,
                name,
                slug,
                webhook_url,
                webhook_auth_token,
                include_drafts,
                auto_deploy,
                debounce_seconds,
                created_at,
                updated_at,
            )| {
                Deployment {
                    id,
                    app_id,
                    name,
                    slug,
                    webhook_url,
                    webhook_auth_token,
                    include_drafts: include_drafts != 0,
                    auto_deploy: auto_deploy != 0,
                    debounce_seconds,
                    created_at,
                    updated_at,
                }
            },
        ))
    }

    pub async fn list_deployments(&self) -> eyre::Result<Vec<Deployment>> {
        let rows: Vec<(i64, Option<i64>, String, String, String, Option<String>, i32, i32, i64, String, String)> =
            sqlx::query_as(
                "SELECT id, app_id, name, slug, webhook_url, webhook_auth_token, include_drafts, auto_deploy, debounce_seconds, created_at, updated_at FROM deployments ORDER BY name"
            )
            .fetch_all(self.pool.as_ref())
            .await?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    app_id,
                    name,
                    slug,
                    webhook_url,
                    webhook_auth_token,
                    include_drafts,
                    auto_deploy,
                    debounce_seconds,
                    created_at,
                    updated_at,
                )| {
                    Deployment {
                        id,
                        app_id,
                        name,
                        slug,
                        webhook_url,
                        webhook_auth_token,
                        include_drafts: include_drafts != 0,
                        auto_deploy: auto_deploy != 0,
                        debounce_seconds,
                        created_at,
                        updated_at,
                    }
                },
            )
            .collect())
    }

    pub async fn update_deployment(
        &self,
        id: i64,
        name: &str,
        slug: &str,
        webhook_url: &str,
        webhook_auth_token: Option<&str>,
        include_drafts: bool,
        auto_deploy: bool,
        debounce_seconds: i64,
    ) -> eyre::Result<()> {
        let include_drafts_i = if include_drafts { 1i32 } else { 0 };
        let auto_deploy_i = if auto_deploy { 1i32 } else { 0 };
        sqlx::query(
            "UPDATE deployments SET name = ?, slug = ?, webhook_url = ?, webhook_auth_token = ?, include_drafts = ?, auto_deploy = ?, debounce_seconds = ?, updated_at = datetime('now') WHERE id = ?"
        )
        .bind(name)
        .bind(slug)
        .bind(webhook_url)
        .bind(webhook_auth_token)
        .bind(include_drafts_i)
        .bind(auto_deploy_i)
        .bind(debounce_seconds)
        .bind(id)
        .execute(self.pool.as_ref())
        .await?;
        Ok(())
    }

    pub async fn delete_deployment(&self, id: i64) -> eyre::Result<()> {
        sqlx::query("DELETE FROM deployments WHERE id = ?")
            .bind(id)
            .execute(self.pool.as_ref())
            .await?;
        Ok(())
    }

    pub async fn list_auto_deploy_deployments(&self) -> eyre::Result<Vec<Deployment>> {
        let rows: Vec<(i64, Option<i64>, String, String, String, Option<String>, i32, i32, i64, String, String)> =
            sqlx::query_as(
                "SELECT id, app_id, name, slug, webhook_url, webhook_auth_token, include_drafts, auto_deploy, debounce_seconds, created_at, updated_at FROM deployments WHERE auto_deploy = 1 ORDER BY name"
            )
            .fetch_all(self.pool.as_ref())
            .await?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    app_id,
                    name,
                    slug,
                    webhook_url,
                    webhook_auth_token,
                    include_drafts,
                    auto_deploy,
                    debounce_seconds,
                    created_at,
                    updated_at,
                )| {
                    Deployment {
                        id,
                        app_id,
                        name,
                        slug,
                        webhook_url,
                        webhook_auth_token,
                        include_drafts: include_drafts != 0,
                        auto_deploy: auto_deploy != 0,
                        debounce_seconds,
                        created_at,
                        updated_at,
                    }
                },
            )
            .collect())
    }

    pub async fn list_deployments_for_app(&self, app_id: i64) -> eyre::Result<Vec<Deployment>> {
        let rows: Vec<(i64, Option<i64>, String, String, String, Option<String>, i32, i32, i64, String, String)> =
            sqlx::query_as(
                "SELECT id, app_id, name, slug, webhook_url, webhook_auth_token, include_drafts, auto_deploy, debounce_seconds, created_at, updated_at FROM deployments WHERE app_id = ? ORDER BY name"
            )
            .bind(app_id)
            .fetch_all(self.pool.as_ref())
            .await?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    app_id,
                    name,
                    slug,
                    webhook_url,
                    webhook_auth_token,
                    include_drafts,
                    auto_deploy,
                    debounce_seconds,
                    created_at,
                    updated_at,
                )| {
                    Deployment {
                        id,
                        app_id,
                        name,
                        slug,
                        webhook_url,
                        webhook_auth_token,
                        include_drafts: include_drafts != 0,
                        auto_deploy: auto_deploy != 0,
                        debounce_seconds,
                        created_at,
                        updated_at,
                    }
                },
            )
            .collect())
    }

    pub async fn get_deployment_by_slug_and_app(
        &self,
        app_id: i64,
        slug: &str,
    ) -> eyre::Result<Option<Deployment>> {
        let row: Option<(i64, Option<i64>, String, String, String, Option<String>, i32, i32, i64, String, String)> =
            sqlx::query_as(
                "SELECT id, app_id, name, slug, webhook_url, webhook_auth_token, include_drafts, auto_deploy, debounce_seconds, created_at, updated_at FROM deployments WHERE app_id = ? AND slug = ?"
            )
            .bind(app_id)
            .bind(slug)
            .fetch_optional(self.pool.as_ref())
            .await?;

        Ok(row.map(
            |(
                id,
                app_id,
                name,
                slug,
                webhook_url,
                webhook_auth_token,
                include_drafts,
                auto_deploy,
                debounce_seconds,
                created_at,
                updated_at,
            )| {
                Deployment {
                    id,
                    app_id,
                    name,
                    slug,
                    webhook_url,
                    webhook_auth_token,
                    include_drafts: include_drafts != 0,
                    auto_deploy: auto_deploy != 0,
                    debounce_seconds,
                    created_at,
                    updated_at,
                }
            },
        ))
    }

    // ── Dirty detection ──────────────────────────────────────────

    pub async fn is_dirty_for_deployment(&self, deployment_id: i64) -> eyre::Result<bool> {
        let last_fired: Option<(Option<String>,)> =
            sqlx::query_as("SELECT last_fired_at FROM deployment_state WHERE deployment_id = ?")
                .bind(deployment_id)
                .fetch_optional(self.pool.as_ref())
                .await?;

        let last_fired_at = match last_fired {
            Some((Some(ts),)) => ts,
            _ => return Ok(true), // Never fired -> dirty
        };

        // Look up the deployment's app_id for scoped dirty detection
        let dep = self.get_deployment_by_id(deployment_id).await?;
        let dep_app_id = dep.and_then(|d| d.app_id);

        let latest_mutation: (Option<String>,) = if let Some(app_id) = dep_app_id {
            sqlx::query_as(
                "SELECT MAX(timestamp) FROM audit_log WHERE app_id = ? AND action IN (\
                    'content_create', 'content_update', 'content_delete', \
                    'schema_create', 'schema_update', 'schema_delete', \
                    'entry_published', 'entry_unpublished')",
            )
            .bind(app_id)
            .fetch_one(self.pool.as_ref())
            .await?
        } else {
            // Fallback: no app_id on deployment, check all mutations (backward compat)
            sqlx::query_as(
                "SELECT MAX(timestamp) FROM audit_log WHERE action IN (\
                    'content_create', 'content_update', 'content_delete', \
                    'schema_create', 'schema_update', 'schema_delete', \
                    'entry_published', 'entry_unpublished')",
            )
            .fetch_one(self.pool.as_ref())
            .await?
        };

        match latest_mutation {
            (Some(ts),) => Ok(ts > last_fired_at),
            _ => Ok(false),
        }
    }

    pub async fn mark_deployment_fired(&self, deployment_id: i64) -> eyre::Result<String> {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO deployment_state (deployment_id, last_fired_at) VALUES (?, ?) \
             ON CONFLICT(deployment_id) DO UPDATE SET last_fired_at = excluded.last_fired_at",
        )
        .bind(deployment_id)
        .bind(&now)
        .execute(self.pool.as_ref())
        .await?;
        Ok(now)
    }

    // ── Webhook history ──────────────────────────────────────────

    pub async fn record_webhook_attempt(
        &self,
        deployment_id: i64,
        trigger_source: &str,
        status: &str,
        http_status: Option<u16>,
        error_message: Option<&str>,
        response_time_ms: Option<i64>,
        attempt: i32,
        group_id: &str,
    ) -> eyre::Result<i64> {
        let now = chrono::Utc::now().to_rfc3339();
        let result = sqlx::query(
            "INSERT INTO webhook_history (deployment_id, trigger_source, status, http_status, error_message, response_time_ms, attempt, group_id, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(deployment_id)
        .bind(trigger_source)
        .bind(status)
        .bind(http_status.map(|s| s as i32))
        .bind(error_message)
        .bind(response_time_ms)
        .bind(attempt)
        .bind(group_id)
        .bind(&now)
        .execute(self.pool.as_ref())
        .await?;
        Ok(result.last_insert_rowid())
    }

    pub async fn list_webhook_history_for_deployment(
        &self,
        deployment_id: Option<i64>,
        status_filter: Option<&str>,
    ) -> eyre::Result<Vec<WebhookHistoryGroup>> {
        let base = "SELECT h.id, h.deployment_id, d.name, d.slug, h.trigger_source, h.status, h.http_status, h.error_message, h.response_time_ms, g.attempt_count, h.group_id, h.created_at
            FROM webhook_history h
            INNER JOIN (
                SELECT group_id, MAX(id) AS max_id, COUNT(*) AS attempt_count
                FROM webhook_history
                GROUP BY group_id
            ) g ON h.id = g.max_id
            INNER JOIN deployments d ON h.deployment_id = d.id";

        let mut conditions = Vec::new();
        if deployment_id.is_some() {
            conditions.push("h.deployment_id = ?");
        }
        if status_filter.is_some() {
            conditions.push("h.status = ?");
        }

        let query = if conditions.is_empty() {
            format!("{base} ORDER BY h.created_at DESC LIMIT 100")
        } else {
            format!(
                "{base} WHERE {} ORDER BY h.created_at DESC LIMIT 100",
                conditions.join(" AND ")
            )
        };

        let mut q = sqlx::query_as::<
            _,
            (
                i64,
                i64,
                String,
                String,
                String,
                String,
                Option<i32>,
                Option<String>,
                Option<i64>,
                i32,
                String,
                String,
            ),
        >(sqlx::AssertSqlSafe(query));

        if let Some(dep_id) = deployment_id {
            q = q.bind(dep_id);
        }
        if let Some(status) = status_filter {
            q = q.bind(status);
        }

        let rows = q.fetch_all(self.pool.as_ref()).await?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    deployment_id,
                    deployment_name,
                    deployment_slug,
                    trigger_source,
                    status,
                    http_status,
                    error_message,
                    response_time_ms,
                    attempt_count,
                    group_id,
                    created_at,
                )| {
                    WebhookHistoryGroup {
                        id,
                        deployment_id,
                        deployment_name,
                        deployment_slug,
                        trigger_source,
                        status,
                        http_status,
                        error_message,
                        response_time_ms,
                        attempt_count,
                        group_id,
                        created_at,
                    }
                },
            )
            .collect())
    }

    // ── Audit log ────────────────────────────────────────────────

    pub async fn list_audit_log(
        &self,
        action_filter: Option<&str>,
        actor_filter: Option<&str>,
        app_filter: Option<&str>,
        date_from: Option<&str>,
        date_to: Option<&str>,
        page: u32,
    ) -> eyre::Result<(Vec<AuditLogEntry>, bool)> {
        let page = page.max(1);
        let offset = (page - 1) as i64 * 100;
        let base = "SELECT id, timestamp, actor, action, resource_type, resource_id, details, app_id FROM audit_log";

        let mut conditions = Vec::new();
        if action_filter.is_some() {
            conditions.push("action = ?".to_string());
        }
        if actor_filter.is_some() {
            conditions.push("actor = ?".to_string());
        }
        if date_from.is_some() {
            conditions.push("timestamp >= ?".to_string());
        }
        if date_to.is_some() {
            // Add a day so "to" date is inclusive
            conditions.push("timestamp < ?".to_string());
        }
        // app_filter: None = no filter, Some("global") = app_id IS NULL, Some("<id>") = app_id = <id>
        let app_filter_id: Option<i64> = match app_filter {
            Some("global") => {
                conditions.push("app_id IS NULL".to_string());
                None
            }
            Some(id_str) => {
                if let Ok(id) = id_str.parse::<i64>() {
                    conditions.push("app_id = ?".to_string());
                    Some(id)
                } else {
                    None
                }
            }
            None => None,
        };

        let query = if conditions.is_empty() {
            format!("{base} ORDER BY timestamp DESC, id DESC LIMIT 101 OFFSET ?")
        } else {
            format!(
                "{base} WHERE {} ORDER BY timestamp DESC, id DESC LIMIT 101 OFFSET ?",
                conditions.join(" AND ")
            )
        };

        let mut q = sqlx::query_as::<
            _,
            (
                i64,
                String,
                String,
                String,
                String,
                String,
                Option<String>,
                Option<i64>,
            ),
        >(sqlx::AssertSqlSafe(query));

        if let Some(action) = action_filter {
            q = q.bind(action);
        }
        if let Some(actor) = actor_filter {
            q = q.bind(actor);
        }
        if let Some(from) = date_from {
            q = q.bind(from);
        }
        if let Some(to) = date_to {
            // Add a day to make the date inclusive
            let to_next = chrono::NaiveDate::parse_from_str(to, "%Y-%m-%d")
                .map(|d| (d + chrono::Duration::days(1)).to_string())
                .unwrap_or_else(|_| to.to_string());
            q = q.bind(to_next);
        }
        if let Some(id) = app_filter_id {
            q = q.bind(id);
        }
        q = q.bind(offset);

        let rows = q.fetch_all(self.pool.as_ref()).await?;
        let has_next = rows.len() > 100;
        let entries: Vec<AuditLogEntry> = rows
            .into_iter()
            .take(100)
            .map(
                |(id, timestamp, actor, action, resource_type, resource_id, details, app_id)| {
                    AuditLogEntry {
                        id,
                        timestamp,
                        actor,
                        action,
                        resource_type,
                        resource_id,
                        details,
                        app_id,
                    }
                },
            )
            .collect();

        Ok((entries, has_next))
    }

    pub async fn list_audit_actors(&self) -> eyre::Result<Vec<String>> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT DISTINCT actor FROM audit_log ORDER BY actor")
                .fetch_all(self.pool.as_ref())
                .await?;
        Ok(rows.into_iter().map(|(actor,)| actor).collect())
    }

    pub fn log(
        &self,
        actor: &str,
        action: &str,
        resource_type: &str,
        resource_id: &str,
        details: Option<&str>,
    ) {
        self.log_with_app(actor, action, resource_type, resource_id, details, None);
    }

    pub fn log_with_app(
        &self,
        actor: &str,
        action: &str,
        resource_type: &str,
        resource_id: &str,
        details: Option<&str>,
        app_id: Option<i64>,
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
                "INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id, details, app_id) VALUES (?, ?, ?, ?, ?, ?, ?)"
            )
            .bind(&timestamp)
            .bind(&actor)
            .bind(&action)
            .bind(&resource_type)
            .bind(&resource_id)
            .bind(&details)
            .bind(app_id)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = result {
                tracing::warn!("Failed to write audit log: {e}");
            }
        });
    }

    // ── Backup config & history ─────────────────────────────────

    pub async fn get_backup_config(&self) -> eyre::Result<BackupConfig> {
        let row: (i64, i64, i32) = sqlx::query_as(
            "SELECT frequency_hours, retention_count, enabled FROM backup_config WHERE id = 1",
        )
        .fetch_one(self.pool.as_ref())
        .await?;
        Ok(BackupConfig {
            frequency_hours: row.0,
            retention_count: row.1,
            enabled: row.2 != 0,
        })
    }

    pub async fn update_backup_config(
        &self,
        frequency_hours: i64,
        retention_count: i64,
        enabled: bool,
    ) -> eyre::Result<()> {
        let enabled_i = if enabled { 1i32 } else { 0 };
        sqlx::query(
            "UPDATE backup_config SET frequency_hours = ?, retention_count = ?, enabled = ? WHERE id = 1",
        )
        .bind(frequency_hours)
        .bind(retention_count)
        .bind(enabled_i)
        .execute(self.pool.as_ref())
        .await?;
        Ok(())
    }

    pub async fn start_backup_record(&self, trigger_source: &str) -> eyre::Result<i64> {
        let now = chrono::Utc::now().to_rfc3339();
        let result = sqlx::query(
            "INSERT INTO backup_history (started_at, status, trigger_source) VALUES (?, 'running', ?)",
        )
        .bind(&now)
        .bind(trigger_source)
        .execute(self.pool.as_ref())
        .await?;
        Ok(result.last_insert_rowid())
    }

    pub async fn complete_backup_record(
        &self,
        id: i64,
        size_bytes: i64,
        s3_key: &str,
        manifest: &str,
    ) -> eyre::Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE backup_history SET status = 'success', completed_at = ?, size_bytes = ?, s3_key = ?, manifest = ? WHERE id = ?",
        )
        .bind(&now)
        .bind(size_bytes)
        .bind(s3_key)
        .bind(manifest)
        .bind(id)
        .execute(self.pool.as_ref())
        .await?;
        Ok(())
    }

    pub async fn fail_backup_record(&self, id: i64, error_message: &str) -> eyre::Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE backup_history SET status = 'failed', completed_at = ?, error_message = ? WHERE id = ?",
        )
        .bind(&now)
        .bind(error_message)
        .bind(id)
        .execute(self.pool.as_ref())
        .await?;
        Ok(())
    }

    pub async fn latest_backup(&self) -> eyre::Result<Option<BackupRecord>> {
        let row: Option<(i64, String, Option<String>, String, String, Option<String>, Option<i64>, Option<String>, Option<String>)> =
            sqlx::query_as(
                "SELECT id, started_at, completed_at, status, trigger_source, error_message, size_bytes, s3_key, manifest FROM backup_history ORDER BY started_at DESC LIMIT 1",
            )
            .fetch_optional(self.pool.as_ref())
            .await?;
        Ok(row.map(
            |(
                id,
                started_at,
                completed_at,
                status,
                trigger_source,
                error_message,
                size_bytes,
                s3_key,
                manifest,
            )| {
                BackupRecord {
                    id,
                    started_at,
                    completed_at,
                    status,
                    trigger_source,
                    error_message,
                    size_bytes,
                    s3_key,
                    manifest,
                }
            },
        ))
    }

    pub async fn last_successful_backup(&self) -> eyre::Result<Option<BackupRecord>> {
        let row: Option<(i64, String, Option<String>, String, String, Option<String>, Option<i64>, Option<String>, Option<String>)> =
            sqlx::query_as(
                "SELECT id, started_at, completed_at, status, trigger_source, error_message, size_bytes, s3_key, manifest FROM backup_history WHERE status = 'success' ORDER BY started_at DESC LIMIT 1",
            )
            .fetch_optional(self.pool.as_ref())
            .await?;
        Ok(row.map(
            |(
                id,
                started_at,
                completed_at,
                status,
                trigger_source,
                error_message,
                size_bytes,
                s3_key,
                manifest,
            )| {
                BackupRecord {
                    id,
                    started_at,
                    completed_at,
                    status,
                    trigger_source,
                    error_message,
                    size_bytes,
                    s3_key,
                    manifest,
                }
            },
        ))
    }

    pub async fn list_backup_history(&self, limit: i64) -> eyre::Result<Vec<BackupRecord>> {
        let rows: Vec<(i64, String, Option<String>, String, String, Option<String>, Option<i64>, Option<String>, Option<String>)> =
            sqlx::query_as(
                "SELECT id, started_at, completed_at, status, trigger_source, error_message, size_bytes, s3_key, manifest FROM backup_history ORDER BY started_at DESC LIMIT ?",
            )
            .bind(limit)
            .fetch_all(self.pool.as_ref())
            .await?;
        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    started_at,
                    completed_at,
                    status,
                    trigger_source,
                    error_message,
                    size_bytes,
                    s3_key,
                    manifest,
                )| {
                    BackupRecord {
                        id,
                        started_at,
                        completed_at,
                        status,
                        trigger_source,
                        error_message,
                        size_bytes,
                        s3_key,
                        manifest,
                    }
                },
            )
            .collect())
    }

    pub async fn prune_backup_history(&self, keep: i64) -> eyre::Result<()> {
        sqlx::query(
            "DELETE FROM backup_history WHERE id NOT IN (SELECT id FROM backup_history ORDER BY started_at DESC LIMIT ?)",
        )
        .bind(keep)
        .execute(self.pool.as_ref())
        .await?;
        Ok(())
    }
}

pub fn validate_deployment_slug(slug: &str) -> Result<(), String> {
    if slug.is_empty() || slug.len() > 64 {
        return Err("Slug must be 1-64 characters".to_string());
    }
    if slug.starts_with('-') || slug.ends_with('-') {
        return Err("Slug cannot start or end with a hyphen".to_string());
    }
    if !slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err("Slug must contain only lowercase letters, numbers, and hyphens".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> SqlitePool {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .pragma("foreign_keys", "ON");
        let pool = SqlitePool::connect_with(options).await.unwrap();
        sqlx::migrate!("./audit_migrations")
            .run(&pool)
            .await
            .unwrap();
        pool
    }

    // ── Deployment CRUD tests ────────────────────────────────────

    #[tokio::test]
    async fn test_create_and_get_deployment() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        let dep = logger
            .create_deployment(
                1,
                "Production",
                "production",
                "https://example.com/hook",
                Some("secret123"),
                true,
                false,
                300,
            )
            .await
            .unwrap();
        assert_eq!(dep.name, "Production");
        assert_eq!(dep.slug, "production");
        assert_eq!(dep.webhook_url, "https://example.com/hook");
        assert_eq!(dep.webhook_auth_token.as_deref(), Some("secret123"));
        assert!(dep.include_drafts);
        assert!(!dep.auto_deploy);
        assert_eq!(dep.debounce_seconds, 300);

        let fetched = logger
            .get_deployment_by_slug("production")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.id, dep.id);
        assert_eq!(fetched.name, "Production");

        let fetched2 = logger.get_deployment_by_id(dep.id).await.unwrap().unwrap();
        assert_eq!(fetched2.slug, "production");

        assert!(
            logger
                .get_deployment_by_slug("nonexistent")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_list_deployments_sorted() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        logger
            .create_deployment(1, "Zulu", "zulu", "https://z.com", None, false, false, 300)
            .await
            .unwrap();
        logger
            .create_deployment(
                1,
                "Alpha",
                "alpha",
                "https://a.com",
                None,
                false,
                false,
                300,
            )
            .await
            .unwrap();
        let all = logger.list_deployments().await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "Alpha");
        assert_eq!(all[1].name, "Zulu");
    }

    #[tokio::test]
    async fn test_update_deployment() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        let dep = logger
            .create_deployment(
                1,
                "Staging",
                "staging",
                "https://old.com",
                None,
                false,
                false,
                300,
            )
            .await
            .unwrap();
        // Small sleep to ensure updated_at differs
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        logger
            .update_deployment(
                dep.id,
                "New Staging",
                "staging",
                "https://new.com",
                Some("token"),
                true,
                true,
                60,
            )
            .await
            .unwrap();
        let updated = logger
            .get_deployment_by_slug("staging")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.name, "New Staging");
        assert_eq!(updated.webhook_url, "https://new.com");
        assert_eq!(updated.webhook_auth_token.as_deref(), Some("token"));
        assert!(updated.include_drafts);
        assert!(updated.auto_deploy);
        assert_eq!(updated.debounce_seconds, 60);
        // updated_at should differ from created_at
        assert_ne!(updated.created_at, updated.updated_at);
    }

    #[tokio::test]
    async fn test_duplicate_slug_fails() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        logger
            .create_deployment(
                1,
                "First",
                "same-slug",
                "https://a.com",
                None,
                false,
                false,
                300,
            )
            .await
            .unwrap();
        let result = logger
            .create_deployment(
                1,
                "Second",
                "same-slug",
                "https://b.com",
                None,
                false,
                false,
                300,
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_validate_deployment_slug() {
        assert!(validate_deployment_slug("production").is_ok());
        assert!(validate_deployment_slug("my-deploy-1").is_ok());
        assert!(validate_deployment_slug("a").is_ok());

        assert!(validate_deployment_slug("").is_err());
        assert!(validate_deployment_slug("My Slug").is_err());
        assert!(validate_deployment_slug("UPPER").is_err());
        assert!(validate_deployment_slug("-leading").is_err());
        assert!(validate_deployment_slug("trailing-").is_err());
        assert!(validate_deployment_slug("has space").is_err());
        assert!(validate_deployment_slug("has_underscore").is_err());
        assert!(validate_deployment_slug(&"a".repeat(65)).is_err());
    }

    // ── Dirty detection tests ────────────────────────────────────

    #[tokio::test]
    async fn test_is_dirty_never_fired() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        let dep = logger
            .create_deployment(1, "D", "d", "https://d.com", None, false, false, 300)
            .await
            .unwrap();
        assert!(logger.is_dirty_for_deployment(dep.id).await.unwrap());
    }

    #[tokio::test]
    async fn test_is_dirty_after_mark_fired_no_mutations() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        let dep = logger
            .create_deployment(1, "D", "d", "https://d.com", None, false, false, 300)
            .await
            .unwrap();
        logger.mark_deployment_fired(dep.id).await.unwrap();
        assert!(!logger.is_dirty_for_deployment(dep.id).await.unwrap());
    }

    #[tokio::test]
    async fn test_is_dirty_after_mutation() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        let dep = logger
            .create_deployment(1, "D", "d", "https://d.com", None, false, false, 300)
            .await
            .unwrap();
        logger.mark_deployment_fired(dep.id).await.unwrap();
        // Insert a mutation with a future timestamp and matching app_id
        let future_ts = (chrono::Utc::now() + chrono::Duration::seconds(10)).to_rfc3339();
        let query = format!(
            "INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id, app_id) VALUES ('{future_ts}', 'test', 'content_create', 'content', 'test/1', 1)"
        );
        logger.execute_raw(&query).await.unwrap();
        assert!(logger.is_dirty_for_deployment(dep.id).await.unwrap());
    }

    #[tokio::test]
    async fn test_is_dirty_ignores_non_mutation_events() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        let dep = logger
            .create_deployment(1, "D", "d", "https://d.com", None, false, false, 300)
            .await
            .unwrap();
        logger.mark_deployment_fired(dep.id).await.unwrap();
        logger
            .execute_raw("INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id, app_id) VALUES (datetime('now', '+1 second'), 'test', 'login', 'session', '', 1)")
            .await
            .unwrap();
        assert!(!logger.is_dirty_for_deployment(dep.id).await.unwrap());
    }

    #[tokio::test]
    async fn test_is_dirty_detects_entry_published() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        let dep = logger
            .create_deployment(1, "D", "d", "https://d.com", None, false, false, 300)
            .await
            .unwrap();
        logger.mark_deployment_fired(dep.id).await.unwrap();
        let future_ts = (chrono::Utc::now() + chrono::Duration::seconds(10)).to_rfc3339();
        let query = format!(
            "INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id, app_id) VALUES ('{future_ts}', 'test', 'entry_published', 'content', 'posts/1', 1)"
        );
        logger.execute_raw(&query).await.unwrap();
        assert!(logger.is_dirty_for_deployment(dep.id).await.unwrap());
    }

    #[tokio::test]
    async fn test_mark_deployment_fired_upsert() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        let dep = logger
            .create_deployment(1, "D", "d", "https://d.com", None, false, false, 300)
            .await
            .unwrap();
        let ts1 = logger.mark_deployment_fired(dep.id).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let ts2 = logger.mark_deployment_fired(dep.id).await.unwrap();
        assert_ne!(ts1, ts2);
    }

    // ── Webhook history tests ────────────────────────────────────

    #[tokio::test]
    async fn test_record_webhook_attempt_with_deployment() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        let dep = logger
            .create_deployment(1, "D", "d", "https://d.com", None, false, false, 300)
            .await
            .unwrap();
        let id = logger
            .record_webhook_attempt(
                dep.id,
                "manual",
                "success",
                Some(200),
                None,
                Some(150),
                1,
                "g1",
            )
            .await
            .unwrap();
        assert!(id > 0);
    }

    #[tokio::test]
    async fn test_list_webhook_history_for_deployment() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        let dep1 = logger
            .create_deployment(
                1,
                "Alpha",
                "alpha",
                "https://a.com",
                None,
                false,
                false,
                300,
            )
            .await
            .unwrap();
        let dep2 = logger
            .create_deployment(1, "Beta", "beta", "https://b.com", None, false, false, 300)
            .await
            .unwrap();

        logger
            .record_webhook_attempt(
                dep1.id,
                "manual",
                "success",
                Some(200),
                None,
                Some(100),
                1,
                "g1",
            )
            .await
            .unwrap();
        logger
            .record_webhook_attempt(
                dep2.id,
                "manual",
                "failed",
                Some(500),
                Some("err"),
                Some(200),
                1,
                "g2",
            )
            .await
            .unwrap();

        // All
        let all = logger
            .list_webhook_history_for_deployment(None, None)
            .await
            .unwrap();
        assert_eq!(all.len(), 2);

        // Filter to dep1
        let filtered = logger
            .list_webhook_history_for_deployment(Some(dep1.id), None)
            .await
            .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].deployment_name, "Alpha");
        assert_eq!(filtered[0].deployment_slug, "alpha");

        // Filter by status
        let failed = logger
            .list_webhook_history_for_deployment(None, Some("failed"))
            .await
            .unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].deployment_slug, "beta");
    }

    #[tokio::test]
    async fn test_delete_deployment_cascades() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        let dep = logger
            .create_deployment(1, "D", "d", "https://d.com", None, false, false, 300)
            .await
            .unwrap();
        logger.mark_deployment_fired(dep.id).await.unwrap();
        logger
            .record_webhook_attempt(
                dep.id,
                "manual",
                "success",
                Some(200),
                None,
                Some(100),
                1,
                "g1",
            )
            .await
            .unwrap();
        logger.delete_deployment(dep.id).await.unwrap();

        assert!(logger.get_deployment_by_id(dep.id).await.unwrap().is_none());
        let history = logger
            .list_webhook_history_for_deployment(Some(dep.id), None)
            .await
            .unwrap();
        assert!(history.is_empty());
    }

    // ── Additional deployment tests ─────────────────────────────

    #[tokio::test]
    async fn test_is_dirty_detects_entry_unpublished() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        let dep = logger
            .create_deployment(1, "D", "d", "https://d.com", None, false, false, 300)
            .await
            .unwrap();
        logger.mark_deployment_fired(dep.id).await.unwrap();
        let future_ts = (chrono::Utc::now() + chrono::Duration::seconds(10)).to_rfc3339();
        let query = format!(
            "INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id, app_id) VALUES ('{future_ts}', 'test', 'entry_unpublished', 'content', 'posts/1', 1)"
        );
        logger.execute_raw(&query).await.unwrap();
        assert!(logger.is_dirty_for_deployment(dep.id).await.unwrap());
    }

    #[tokio::test]
    async fn test_list_auto_deploy_deployments() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        logger
            .create_deployment(
                1,
                "Manual",
                "manual",
                "https://m.com",
                None,
                false,
                false,
                300,
            )
            .await
            .unwrap();
        logger
            .create_deployment(1, "Auto", "auto", "https://a.com", None, false, true, 60)
            .await
            .unwrap();
        logger
            .create_deployment(
                1,
                "Also Auto",
                "also-auto",
                "https://aa.com",
                None,
                false,
                true,
                120,
            )
            .await
            .unwrap();

        let auto = logger.list_auto_deploy_deployments().await.unwrap();
        assert_eq!(auto.len(), 2);
        // Sorted by name
        assert_eq!(auto[0].slug, "also-auto");
        assert_eq!(auto[1].slug, "auto");
        // Manual deployment should not be included
        assert!(auto.iter().all(|d| d.auto_deploy));
    }

    #[tokio::test]
    async fn test_get_deployment_by_id_nonexistent() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        assert!(logger.get_deployment_by_id(999).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_deployment_no_auth_token() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        let dep = logger
            .create_deployment(
                1,
                "NoToken",
                "no-token",
                "https://n.com",
                None,
                false,
                false,
                300,
            )
            .await
            .unwrap();
        assert!(dep.webhook_auth_token.is_none());

        let fetched = logger
            .get_deployment_by_slug("no-token")
            .await
            .unwrap()
            .unwrap();
        assert!(fetched.webhook_auth_token.is_none());
    }

    #[tokio::test]
    async fn test_is_dirty_independent_per_deployment() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        let dep1 = logger
            .create_deployment(1, "D1", "d1", "https://d1.com", None, false, false, 300)
            .await
            .unwrap();
        let dep2 = logger
            .create_deployment(1, "D2", "d2", "https://d2.com", None, false, false, 300)
            .await
            .unwrap();

        // Both start dirty (never fired)
        assert!(logger.is_dirty_for_deployment(dep1.id).await.unwrap());
        assert!(logger.is_dirty_for_deployment(dep2.id).await.unwrap());

        // Fire dep1 only
        logger.mark_deployment_fired(dep1.id).await.unwrap();
        assert!(!logger.is_dirty_for_deployment(dep1.id).await.unwrap());
        // dep2 still dirty (never fired)
        assert!(logger.is_dirty_for_deployment(dep2.id).await.unwrap());

        // Fire dep2
        logger.mark_deployment_fired(dep2.id).await.unwrap();
        assert!(!logger.is_dirty_for_deployment(dep2.id).await.unwrap());

        // Add a mutation -- both become dirty
        let future_ts = (chrono::Utc::now() + chrono::Duration::seconds(10)).to_rfc3339();
        let query = format!(
            "INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id, app_id) VALUES ('{future_ts}', 'test', 'content_update', 'content', 'posts/1', 1)"
        );
        logger.execute_raw(&query).await.unwrap();
        assert!(logger.is_dirty_for_deployment(dep1.id).await.unwrap());
        assert!(logger.is_dirty_for_deployment(dep2.id).await.unwrap());
    }

    #[tokio::test]
    async fn test_update_deployment_slug_change() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        let dep = logger
            .create_deployment(
                1,
                "Staging",
                "staging",
                "https://s.com",
                None,
                false,
                false,
                300,
            )
            .await
            .unwrap();
        logger
            .update_deployment(
                dep.id,
                "Staging",
                "staging-v2",
                "https://s.com",
                None,
                false,
                false,
                300,
            )
            .await
            .unwrap();
        // Old slug no longer resolves
        assert!(
            logger
                .get_deployment_by_slug("staging")
                .await
                .unwrap()
                .is_none()
        );
        // New slug works
        let fetched = logger
            .get_deployment_by_slug("staging-v2")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.id, dep.id);
    }

    // ── Audit log tests ──────────────────────────────────────────

    #[tokio::test]
    async fn test_list_audit_log_order_and_basic() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);

        logger
            .execute_raw("INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id, details) VALUES ('2026-01-01T00:00:00Z', 'user1', 'content_create', 'content', 'posts/1', '{\"title\":\"Hello\"}')")
            .await
            .unwrap();
        logger
            .execute_raw("INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id, details) VALUES ('2026-01-02T00:00:00Z', 'user2', 'login', 'session', '', NULL)")
            .await
            .unwrap();

        let (entries, has_next) = logger
            .list_audit_log(None, None, None, None, None, 1)
            .await
            .unwrap();
        assert_eq!(entries.len(), 2);
        assert!(!has_next);
        assert_eq!(entries[0].action, "login");
        assert_eq!(entries[0].actor, "user2");
        assert_eq!(entries[1].action, "content_create");
        assert_eq!(entries[1].actor, "user1");
        assert_eq!(
            entries[1].details,
            Some("{\"title\":\"Hello\"}".to_string())
        );
        assert_eq!(entries[0].details, None);
    }

    #[tokio::test]
    async fn test_list_audit_log_action_filter() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);

        logger
            .execute_raw("INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id) VALUES ('2026-01-01T00:00:00Z', 'user1', 'content_create', 'content', 'posts/1')")
            .await
            .unwrap();
        logger
            .execute_raw("INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id) VALUES ('2026-01-02T00:00:00Z', 'user1', 'login', 'session', '')")
            .await
            .unwrap();

        let (entries, _) = logger
            .list_audit_log(Some("login"), None, None, None, None, 1)
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "login");
    }

    #[tokio::test]
    async fn test_list_audit_log_actor_filter() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);

        logger
            .execute_raw("INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id) VALUES ('2026-01-01T00:00:00Z', 'user1', 'content_create', 'content', 'posts/1')")
            .await
            .unwrap();
        logger
            .execute_raw("INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id) VALUES ('2026-01-02T00:00:00Z', 'user2', 'login', 'session', '')")
            .await
            .unwrap();

        let (entries, _) = logger
            .list_audit_log(None, Some("user1"), None, None, None, 1)
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].actor, "user1");
    }

    #[tokio::test]
    async fn test_list_audit_log_pagination() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);

        for i in 0..105 {
            let ts = format!("2026-01-01T{:02}:{:02}:00Z", i / 60, i % 60);
            let query = format!(
                "INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id) VALUES ('{ts}', 'user1', 'login', 'session', '')"
            );
            logger.execute_raw(&query).await.unwrap();
        }

        let (page1, has_next1) = logger
            .list_audit_log(None, None, None, None, None, 1)
            .await
            .unwrap();
        assert_eq!(page1.len(), 100);
        assert!(has_next1);

        let (page2, has_next2) = logger
            .list_audit_log(None, None, None, None, None, 2)
            .await
            .unwrap();
        assert_eq!(page2.len(), 5);
        assert!(!has_next2);
    }

    #[tokio::test]
    async fn test_list_audit_actors() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);

        logger
            .execute_raw("INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id) VALUES ('2026-01-01T00:00:00Z', 'zara', 'login', 'session', '')")
            .await
            .unwrap();
        logger
            .execute_raw("INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id) VALUES ('2026-01-02T00:00:00Z', 'alice', 'login', 'session', '')")
            .await
            .unwrap();
        logger
            .execute_raw("INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id) VALUES ('2026-01-03T00:00:00Z', 'alice', 'logout', 'session', '')")
            .await
            .unwrap();

        let actors = logger.list_audit_actors().await.unwrap();
        assert_eq!(actors, vec!["alice", "zara"]);
    }

    // ── Backup config & history tests ───────────────────────────

    #[tokio::test]
    async fn test_backup_config_defaults() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        let config = logger.get_backup_config().await.unwrap();
        assert_eq!(config.frequency_hours, 24);
        assert_eq!(config.retention_count, 7);
        assert!(!config.enabled);
    }

    #[tokio::test]
    async fn test_update_and_get_backup_config() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        logger.update_backup_config(12, 14, true).await.unwrap();
        let config = logger.get_backup_config().await.unwrap();
        assert_eq!(config.frequency_hours, 12);
        assert_eq!(config.retention_count, 14);
        assert!(config.enabled);
    }

    #[tokio::test]
    async fn test_start_backup_record() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        let id = logger.start_backup_record("manual").await.unwrap();
        assert!(id > 0);
        let latest = logger.latest_backup().await.unwrap().unwrap();
        assert_eq!(latest.id, id);
        assert_eq!(latest.status, "running");
        assert_eq!(latest.trigger_source, "manual");
    }

    #[tokio::test]
    async fn test_complete_backup_record() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        let id = logger.start_backup_record("scheduled").await.unwrap();
        logger
            .complete_backup_record(id, 1024, "backups/test.tar.gz", r#"{"version":1}"#)
            .await
            .unwrap();
        let latest = logger.latest_backup().await.unwrap().unwrap();
        assert_eq!(latest.status, "success");
        assert_eq!(latest.size_bytes, Some(1024));
        assert_eq!(latest.s3_key.as_deref(), Some("backups/test.tar.gz"));
        assert_eq!(latest.manifest.as_deref(), Some(r#"{"version":1}"#));
        assert!(latest.completed_at.is_some());
    }

    #[tokio::test]
    async fn test_fail_backup_record() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        let id = logger.start_backup_record("manual").await.unwrap();
        logger
            .fail_backup_record(id, "connection refused")
            .await
            .unwrap();
        let latest = logger.latest_backup().await.unwrap().unwrap();
        assert_eq!(latest.status, "failed");
        assert_eq!(latest.error_message.as_deref(), Some("connection refused"));
        assert!(latest.completed_at.is_some());
    }

    #[tokio::test]
    async fn test_last_successful_backup_skips_failed() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);

        // Create a successful backup
        let id1 = logger.start_backup_record("scheduled").await.unwrap();
        logger
            .complete_backup_record(id1, 512, "backups/1.tar.gz", "{}")
            .await
            .unwrap();

        // Create a failed backup (more recent)
        let id2 = logger.start_backup_record("scheduled").await.unwrap();
        logger
            .fail_backup_record(id2, "network error")
            .await
            .unwrap();

        let last_success = logger.last_successful_backup().await.unwrap().unwrap();
        assert_eq!(last_success.id, id1);
        assert_eq!(last_success.status, "success");
    }

    #[tokio::test]
    async fn test_list_backup_history_order() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);

        let id1 = logger.start_backup_record("scheduled").await.unwrap();
        logger
            .complete_backup_record(id1, 100, "backups/1.tar.gz", "{}")
            .await
            .unwrap();

        let id2 = logger.start_backup_record("manual").await.unwrap();
        logger
            .complete_backup_record(id2, 200, "backups/2.tar.gz", "{}")
            .await
            .unwrap();

        let id3 = logger.start_backup_record("scheduled").await.unwrap();
        logger
            .complete_backup_record(id3, 300, "backups/3.tar.gz", "{}")
            .await
            .unwrap();

        let history = logger.list_backup_history(10).await.unwrap();
        assert_eq!(history.len(), 3);
        // Most recent first
        assert_eq!(history[0].id, id3);
        assert_eq!(history[1].id, id2);
        assert_eq!(history[2].id, id1);
    }

    #[tokio::test]
    async fn test_prune_backup_history() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);

        for i in 0..5 {
            let id = logger.start_backup_record("scheduled").await.unwrap();
            logger
                .complete_backup_record(id, 100 * (i + 1), &format!("backups/{i}.tar.gz"), "{}")
                .await
                .unwrap();
        }

        logger.prune_backup_history(2).await.unwrap();
        let history = logger.list_backup_history(10).await.unwrap();
        assert_eq!(history.len(), 2);
        // Should keep the 2 most recent
        assert_eq!(history[0].size_bytes, Some(500));
        assert_eq!(history[1].size_bytes, Some(400));
    }

    #[tokio::test]
    async fn test_latest_backup_returns_none_when_empty() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);
        let latest = logger.latest_backup().await.unwrap();
        assert!(latest.is_none());
    }

    #[tokio::test]
    async fn test_last_successful_backup_returns_none_when_only_failed() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);

        let id = logger.start_backup_record("scheduled").await.unwrap();
        logger
            .fail_backup_record(id, "connection refused")
            .await
            .unwrap();

        let last_success = logger.last_successful_backup().await.unwrap();
        assert!(
            last_success.is_none(),
            "Should return None when no successful backups exist"
        );
    }

    #[tokio::test]
    async fn test_list_backup_history_respects_limit() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);

        for i in 0..5 {
            let id = logger.start_backup_record("scheduled").await.unwrap();
            logger
                .complete_backup_record(id, 100 * (i + 1), &format!("backups/{i}.tar.gz"), "{}")
                .await
                .unwrap();
        }

        let history = logger.list_backup_history(3).await.unwrap();
        assert_eq!(history.len(), 3, "Should return at most 3 records");
        // Most recent first
        assert_eq!(history[0].size_bytes, Some(500));
        assert_eq!(history[1].size_bytes, Some(400));
        assert_eq!(history[2].size_bytes, Some(300));
    }

    #[tokio::test]
    async fn test_prune_backup_history_noop_when_fewer_than_keep() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);

        let id = logger.start_backup_record("manual").await.unwrap();
        logger
            .complete_backup_record(id, 256, "backups/only.tar.gz", "{}")
            .await
            .unwrap();

        // Prune with keep=10, but only 1 record exists
        logger.prune_backup_history(10).await.unwrap();
        let history = logger.list_backup_history(10).await.unwrap();
        assert_eq!(history.len(), 1, "Should not delete when fewer than keep");
        assert_eq!(history[0].size_bytes, Some(256));
    }

    // ── Multi-app audit tests ────────────────────────────────────

    #[tokio::test]
    async fn test_log_with_app() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);

        // Log with app_id
        logger
            .execute_raw("INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id, app_id) VALUES ('2026-01-01T00:00:00Z', 'user1', 'content_create', 'content', 'posts/1', 1)")
            .await
            .unwrap();
        // Log without app_id (global event)
        logger
            .execute_raw("INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id) VALUES ('2026-01-02T00:00:00Z', 'user1', 'login', 'session', '')")
            .await
            .unwrap();
        // Log with different app_id
        logger
            .execute_raw("INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id, app_id) VALUES ('2026-01-03T00:00:00Z', 'user1', 'content_create', 'content', 'pages/1', 2)")
            .await
            .unwrap();

        // No filter: returns all
        let (entries, _) = logger
            .list_audit_log(None, None, None, None, None, 1)
            .await
            .unwrap();
        assert_eq!(entries.len(), 3);

        // Filter by app_id = 1
        let (entries, _) = logger
            .list_audit_log(None, None, Some("1"), None, None, 1)
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].app_id, Some(1));
        assert_eq!(entries[0].resource_id, "posts/1");

        // Filter by app_id = 2
        let (entries, _) = logger
            .list_audit_log(None, None, Some("2"), None, None, 1)
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].app_id, Some(2));
        assert_eq!(entries[0].resource_id, "pages/1");

        // Filter by global (no app_id)
        let (entries, _) = logger
            .list_audit_log(None, None, Some("global"), None, None, 1)
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].app_id, None);
        assert_eq!(entries[0].action, "login");
    }

    #[tokio::test]
    async fn test_deployment_scoped_by_app() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);

        // Create deployments for two different apps
        let dep1 = logger
            .create_deployment(
                1,
                "Prod App1",
                "prod",
                "https://a.com",
                None,
                false,
                false,
                300,
            )
            .await
            .unwrap();
        let dep2 = logger
            .create_deployment(
                2,
                "Prod App2",
                "prod",
                "https://b.com",
                None,
                false,
                false,
                300,
            )
            .await
            .unwrap();
        let dep3 = logger
            .create_deployment(
                1,
                "Staging App1",
                "staging",
                "https://c.com",
                None,
                false,
                false,
                300,
            )
            .await
            .unwrap();

        // list_deployments_for_app returns only correct app's deployments
        let app1_deps = logger.list_deployments_for_app(1).await.unwrap();
        assert_eq!(app1_deps.len(), 2);
        let slugs: Vec<&str> = app1_deps.iter().map(|d| d.slug.as_str()).collect();
        assert!(slugs.contains(&"prod"));
        assert!(slugs.contains(&"staging"));

        let app2_deps = logger.list_deployments_for_app(2).await.unwrap();
        assert_eq!(app2_deps.len(), 1);
        assert_eq!(app2_deps[0].slug, "prod");

        // get_deployment_by_slug_and_app returns correct one
        let found = logger
            .get_deployment_by_slug_and_app(1, "prod")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, dep1.id);

        let found = logger
            .get_deployment_by_slug_and_app(2, "prod")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, dep2.id);

        // Non-existent combination
        let found = logger
            .get_deployment_by_slug_and_app(2, "staging")
            .await
            .unwrap();
        assert!(found.is_none());

        // Same slug in different apps is allowed
        assert_ne!(dep1.id, dep2.id);
        // Verify app_ids
        assert_eq!(dep1.app_id, Some(1));
        assert_eq!(dep2.app_id, Some(2));
        assert_eq!(dep3.app_id, Some(1));
    }

    #[tokio::test]
    async fn test_dirty_detection_scoped() {
        let pool = test_pool().await;
        let logger = AuditLogger::new(pool);

        // Create deployments for two different apps
        let dep1 = logger
            .create_deployment(1, "D1", "d1", "https://d1.com", None, false, false, 300)
            .await
            .unwrap();
        let dep2 = logger
            .create_deployment(2, "D2", "d2", "https://d2.com", None, false, false, 300)
            .await
            .unwrap();

        // Mark both as fired
        logger.mark_deployment_fired(dep1.id).await.unwrap();
        logger.mark_deployment_fired(dep2.id).await.unwrap();

        // Neither should be dirty
        assert!(!logger.is_dirty_for_deployment(dep1.id).await.unwrap());
        assert!(!logger.is_dirty_for_deployment(dep2.id).await.unwrap());

        // Insert a mutation for app_id=1 only (with a future timestamp)
        let future_ts = (chrono::Utc::now() + chrono::Duration::seconds(10)).to_rfc3339();
        let query = format!(
            "INSERT INTO audit_log (timestamp, actor, action, resource_type, resource_id, app_id) VALUES ('{future_ts}', 'test', 'content_create', 'content', 'test/1', 1)"
        );
        logger.execute_raw(&query).await.unwrap();

        // dep1 (app_id=1) should be dirty, dep2 (app_id=2) should not
        assert!(logger.is_dirty_for_deployment(dep1.id).await.unwrap());
        assert!(!logger.is_dirty_for_deployment(dep2.id).await.unwrap());
    }
}
