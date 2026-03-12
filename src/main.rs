use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use dashmap::DashMap;
use tokio::net::TcpListener;
use tower_sessions::SessionManagerLayer;
use tower_sessions_sqlx_store::SqliteStore;

use substrukt::audit;
use substrukt::auth;
use substrukt::cache;
use substrukt::config::Config;
use substrukt::db;
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
}

#[derive(Subcommand)]
enum Command {
    /// Start the web server (default)
    Serve,
    /// Import a bundle tar.gz
    Import {
        /// Path to the bundle tar.gz file
        path: PathBuf,
    },
    /// Export a bundle tar.gz
    Export {
        /// Output path for the bundle tar.gz file
        path: PathBuf,
    },
    /// Create an API token
    CreateToken {
        /// Name for the token
        name: String,
    },
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
    let config = Config::new(cli.data_dir, cli.db_path, cli.port, cli.secure_cookies);
    config.ensure_dirs()?;

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => run_server(config).await,
        Command::Import { path } => {
            let warnings = sync::import_bundle(&config.data_dir, &path)?;
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
        Command::Export { path } => {
            sync::export_bundle(&config.data_dir, &path)?;
            tracing::info!("Exported to {}", path.display());
            Ok(())
        }
        Command::CreateToken { name } => {
            let pool = db::init_pool(&config.db_path).await?;
            let count = db::models::user_count(&pool).await?;
            if count == 0 {
                eyre::bail!("No users exist. Run the server and set up an admin user first.");
            }
            let raw_token = auth::token::generate_token();
            let token_hash = auth::token::hash_token(&raw_token);
            db::models::create_api_token(&pool, 1, &name, &token_hash).await?;
            println!("Token created: {raw_token}");
            println!("(Save this token — it won't be shown again)");
            Ok(())
        }
    }
}

async fn run_server(config: Config) -> eyre::Result<()> {
    let pool = db::init_pool(&config.db_path).await?;

    // Session store
    let session_store = SqliteStore::new(pool.clone());
    session_store.migrate().await?;
    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(config.secure_cookies);

    // Template environment (auto-reloads on file changes)
    let reloader = templates::create_reloader(config.schemas_dir());

    // Content cache
    let content_cache = DashMap::new();
    cache::populate(&content_cache, &config.schemas_dir(), &config.content_dir());

    // Prometheus metrics
    let metrics_handle = metrics::setup_recorder();

    // Audit logging (separate database)
    let audit_db_path = config.data_dir.join("audit.db");
    let audit_pool = audit::init_pool(&audit_db_path).await?;
    let audit_logger = audit::AuditLogger::new(audit_pool);

    let state = Arc::new(AppStateInner {
        pool,
        config: config.clone(),
        templates: reloader,
        cache: content_cache,
        login_limiter: RateLimiter::new(10, std::time::Duration::from_secs(60)),
        api_limiter: RateLimiter::new(100, std::time::Duration::from_secs(60)),
        metrics_handle,
        audit: audit_logger,
    });

    // File watcher for cache invalidation
    let _watcher = cache::spawn_watcher(
        Arc::new(state.cache.clone()),
        config.schemas_dir(),
        config.content_dir(),
    );

    let app = routes::build_router(state)
        .layer(session_layer)
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let addr = format!("{}:{}", config.listen_addr, config.listen_port);
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("Listening on {addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install Ctrl+C handler");
    tracing::info!("Shutdown signal received");
}
