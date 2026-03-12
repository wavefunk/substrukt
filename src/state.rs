use std::sync::Arc;

use dashmap::DashMap;
use minijinja_autoreload::AutoReloader;
use sqlx::SqlitePool;

use crate::config::Config;
use crate::rate_limit::RateLimiter;

pub type ContentCache = DashMap<String, serde_json::Value>;

pub struct AppStateInner {
    pub pool: SqlitePool,
    pub config: Config,
    pub templates: AutoReloader,
    pub cache: ContentCache,
    pub login_limiter: RateLimiter,
    pub api_limiter: RateLimiter,
}

pub type AppState = Arc<AppStateInner>;
