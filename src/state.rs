use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use allowthem_core::{AllowThem, AuthClient, EmailSender};
use dashmap::DashMap;
use metrics_exporter_prometheus::PrometheusHandle;
use minijinja_autoreload::AutoReloader;
use sqlx::SqlitePool;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::audit::AuditLogger;
use crate::backup::S3Config;
use crate::config::Config;
use crate::rate_limit::RateLimiter;

pub type ContentCache = DashMap<String, serde_json::Value>;
pub type EtagCache = DashMap<String, String>;
pub type OpenApiCache = Arc<std::sync::RwLock<Option<serde_json::Value>>>;

pub struct AppStateInner {
    pub pool: SqlitePool,
    pub config: Config,
    pub templates: AutoReloader,
    pub cache: ContentCache,
    pub etag_cache: EtagCache,
    pub login_limiter: RateLimiter,
    pub api_limiter: RateLimiter,
    pub metrics_handle: PrometheusHandle,
    pub audit: AuditLogger,
    pub http_client: reqwest::Client,
    pub deploy_tasks: DashMap<i64, CancellationToken>,
    pub s3_config: Option<S3Config>,
    pub backup_trigger: Option<mpsc::Sender<()>>,
    pub backup_running: AtomicBool,
    pub backup_cancel: Option<CancellationToken>,
    pub openapi_cache: OpenApiCache,
    pub ath: AllowThem,
    pub auth_client: Arc<dyn AuthClient>,
    pub email_sender: Arc<dyn EmailSender>,
    pub has_users: Arc<AtomicBool>,
}

pub type AppState = Arc<AppStateInner>;
