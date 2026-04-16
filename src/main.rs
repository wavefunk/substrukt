use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use clap::{Parser, Subcommand};
use dashmap::DashMap;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_sessions::MemoryStore;
use tower_sessions::SessionManagerLayer;

use substrukt::audit;
use substrukt::auth;
use substrukt::cache;
use substrukt::config::Config;
use substrukt::db;
use substrukt::db::models;
use substrukt::metrics;
use substrukt::rate_limit::RateLimiter;
use substrukt::routes;
use substrukt::state::AppStateInner;
use substrukt::sync;
use substrukt::templates;

#[derive(Parser)]
#[command(name = "substrukt", about = "Schema-driven CMS")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Data directory path
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,

    /// Database file path
    #[arg(long, global = true)]
    db_path: Option<PathBuf>,

    /// Port to listen on
    #[arg(long, short, global = true)]
    port: Option<u16>,

    /// Enable secure (HTTPS-only) session cookies
    #[arg(long, global = true)]
    secure_cookies: bool,

    /// Max API requests per IP per minute (rate limit)
    #[arg(long, global = true, default_value = "100")]
    api_rate_limit: usize,

    /// Maximum number of content versions to keep per entry
    #[arg(long, global = true, default_value = "10")]
    version_history_count: usize,

    /// Maximum request body size in megabytes
    #[arg(long, global = true, default_value = "50")]
    max_body_size: usize,

    /// Trust X-Forwarded-For headers for rate limiting (enable only behind a trusted reverse proxy)
    #[arg(long, global = true)]
    trust_proxy_headers: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Start the web server (default)
    Serve,
    /// Import a bundle tar.gz
    Import {
        /// Path to the bundle tar.gz file
        path: PathBuf,
        /// App slug to import into
        #[arg(long)]
        app: String,
    },
    /// Export a bundle tar.gz
    Export {
        /// Output path for the bundle tar.gz file
        path: PathBuf,
        /// App slug to export from
        #[arg(long)]
        app: String,
    },
    /// Create an API token
    CreateToken {
        /// Name for the token
        name: String,
        /// App slug to create the token for
        #[arg(long)]
        app: String,
    },
    /// Output AI-optimized workflow context for LLM agents
    Prime,
    /// Output a minimal snippet for AGENTS.md / CLAUDE.md
    Onboard,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "substrukt=info,tower_http=info".into()),
        )
        .init();

    let cli = Cli::parse();
    let api_rate_limit = cli.api_rate_limit;
    let mut config = Config::new(
        cli.data_dir,
        cli.db_path,
        cli.port,
        cli.secure_cookies,
        cli.version_history_count,
        cli.max_body_size,
    );
    config.trust_proxy_headers = cli.trust_proxy_headers;
    config.ensure_dirs()?;

    match cli.command.unwrap_or(Command::Serve) {
        Command::Prime => {
            print!("{}", substrukt::prime::prime_output(&config));
            Ok(())
        }
        Command::Onboard => {
            print!("{}", substrukt::prime::onboard_output());
            Ok(())
        }
        Command::Serve => run_server(config, api_rate_limit).await,
        Command::Import { path, app } => {
            let pool = db::init_pool(&config.db_path).await?;
            let app_record = models::find_app_by_slug(&pool, &app)
                .await?
                .ok_or_else(|| eyre::eyre!("App '{app}' not found"))?;
            let app_dir = config.app_dir(&app);
            let warnings = sync::import_bundle(&app_dir, &pool, app_record.id, &path).await?;
            if warnings.is_empty() {
                tracing::info!("Import complete, no validation warnings");
            } else {
                tracing::warn!("Import complete with {} warnings:", warnings.len());
                for w in &warnings {
                    tracing::warn!("  {w}");
                }
            }
            Ok(())
        }
        Command::Export { path, app } => {
            let pool = db::init_pool(&config.db_path).await?;
            let app_record = models::find_app_by_slug(&pool, &app)
                .await?
                .ok_or_else(|| eyre::eyre!("App '{app}' not found"))?;
            let app_dir = config.app_dir(&app);
            sync::export_bundle(&app_dir, &pool, app_record.id, &path).await?;
            tracing::info!("Exported to {}", path.display());
            Ok(())
        }
        Command::CreateToken { name, app } => {
            let pool = db::init_pool(&config.db_path).await?;
            let count = models::user_count(&pool).await?;
            if count == 0 {
                eyre::bail!("No users exist. Run the server and set up an admin user first.");
            }
            let app_record = models::find_app_by_slug(&pool, &app)
                .await?
                .ok_or_else(|| eyre::eyre!("App '{app}' not found"))?;
            let raw_token = auth::token::generate_token();
            let token_hash = auth::token::hash_token(&raw_token);
            models::create_api_token(&pool, 1, app_record.id, &name, &token_hash).await?;
            println!("Token created: {raw_token}");
            println!("(Save this token — it won't be shown again)");
            Ok(())
        }
    }
}

async fn run_server(config: Config, api_rate_limit: usize) -> eyre::Result<()> {
    let pool = db::init_pool(&config.db_path).await?;

    // Session store
    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store).with_secure(config.secure_cookies);

    // Audit logging (separate database) — must be before template reloader
    let audit_db_path = config.data_dir.join("audit.db");
    let audit_pool = audit::init_pool(&audit_db_path).await?;
    let audit_logger = audit::AuditLogger::new(audit_pool);

    // allowthem auth system (shares substrukt's pool)
    let ath = allowthem_core::AllowThemBuilder::with_pool(pool.clone())
        .cookie_secure(config.secure_cookies)
        .build()
        .await
        .expect("Failed to initialize allowthem");

    // Bootstrap roles (idempotent)
    for role_name in ["admin", "editor", "viewer"] {
        let rn = allowthem_core::RoleName::new(role_name);
        if ath.db().get_role_by_name(&rn).await.unwrap_or(None).is_none() {
            ath.db()
                .create_role(&rn, None)
                .await
                .expect("Failed to create role");
        }
    }

    // Check if any users exist (for setup redirect)
    let has_users = !ath.db().list_users().await.unwrap_or_default().is_empty();

    let auth_client: Arc<dyn allowthem_core::AuthClient> =
        Arc::new(allowthem_core::EmbeddedAuthClient::new(ath.clone(), "/login"));

    // Migrate old single-app layout to multi-app
    substrukt::migrate_single_app_layout(&config.data_dir)?;

    // Ensure default app dirs exist
    config.ensure_app_dirs("default")?;

    // Template environment (auto-reloads on file changes)
    let reloader = templates::create_reloader();

    // Migrate .meta.json sidecars to SQLite (one-time, idempotent)
    // Migrate .meta.json sidecars to SQLite (one-time, idempotent, iterates app dirs)
    substrukt::uploads::migrate_meta_sidecars(&config.data_dir, &pool).await?;

    // Content cache
    let content_cache = DashMap::new();
    cache::populate(&content_cache, &config.data_dir);

    // Prometheus metrics
    let metrics_handle = metrics::setup_recorder();

    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .connect_timeout(std::time::Duration::from_secs(5))
        .user_agent("Substrukt/0.1")
        .build()?;

    // S3 backup configuration
    let s3_config = substrukt::backup::S3Config::from_env();
    let (backup_trigger_tx, backup_trigger_rx) = if s3_config.is_some() {
        let (tx, rx) = tokio::sync::mpsc::channel::<()>(1);
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };
    let backup_cancel = s3_config.as_ref().map(|_| CancellationToken::new());

    let state = Arc::new(AppStateInner {
        pool,
        config: config.clone(),
        templates: reloader,
        cache: content_cache,
        etag_cache: DashMap::new(),
        login_limiter: RateLimiter::new(10, std::time::Duration::from_secs(60)),
        api_limiter: RateLimiter::new(api_rate_limit, std::time::Duration::from_secs(60)),
        metrics_handle,
        audit: audit_logger,
        http_client,
        deploy_tasks: DashMap::new(),
        s3_config,
        backup_trigger: backup_trigger_tx,
        backup_running: AtomicBool::new(false),
        backup_cancel: backup_cancel.clone(),
        openapi_cache: Arc::new(std::sync::RwLock::new(None)),
        ath,
        auth_client,
        has_users: AtomicBool::new(has_users),
    });

    // Spawn auto-deploy tasks for all enabled deployments
    if let Ok(deployments) = state.audit.list_auto_deploy_deployments().await {
        for deployment in deployments {
            substrukt::webhooks::spawn_auto_deploy_task(&state, deployment);
        }
    }

    // Spawn backup task if S3 is configured
    if let (Some(s3_cfg), Some(rx), Some(cancel)) =
        (state.s3_config.clone(), backup_trigger_rx, &backup_cancel)
    {
        substrukt::backup::spawn_backup_task(state.clone(), s3_cfg, rx, cancel.child_token());
    }

    // File watcher for cache invalidation (content + openapi spec)
    let _watcher = cache::spawn_watcher(
        Arc::new(state.cache.clone()),
        Arc::new(state.etag_cache.clone()),
        state.openapi_cache.clone(),
        config.data_dir.clone(),
    );

    let app = routes::build_router(state)
        .layer(axum::extract::DefaultBodyLimit::max(config.max_body_size))
        .layer(session_layer)
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let addr = format!("{}:{}", config.listen_addr, config.listen_port);
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("Listening on {addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    if let Some(ref token) = backup_cancel {
        token.cancel();
    }

    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install Ctrl+C handler");
    tracing::info!("Shutdown signal received");
}
