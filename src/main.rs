use std::net::SocketAddr;
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

    /// Enable public browser registration. Disabled by default; use create-admin for setup.
    #[arg(long, global = true)]
    enable_registrations: bool,
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
    /// Create the initial admin user from the CLI
    CreateAdmin {
        /// Admin email address
        #[arg(long)]
        email: String,
        /// Optional admin username
        #[arg(long)]
        username: Option<String>,
        /// Admin password
        #[arg(long)]
        password: String,
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
    config.registrations_enabled = cli.enable_registrations;
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
        Command::CreateAdmin {
            email,
            username,
            password,
        } => create_initial_admin(config, email, username, password).await,
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
            let ath = allowthem_core::AllowThemBuilder::with_pool(pool.clone())
                .build()
                .await
                .expect("Failed to init allowthem");
            let users = ath.db().list_users().await.unwrap_or_default();
            if users.is_empty() {
                eyre::bail!("No users exist. Run `substrukt create-admin` first.");
            }
            let app_record = models::find_app_by_slug(&pool, &app)
                .await?
                .ok_or_else(|| eyre::eyre!("App '{app}' not found"))?;
            let first_user = users.into_iter().next().unwrap();
            let (raw_token, info) = ath
                .db()
                .create_api_token(first_user.id, &name, None, None)
                .await
                .map_err(|e| eyre::eyre!("Failed to create token: {e}"))?;
            // Create app_tokens entry linking token to app
            use sha2::{Digest, Sha256};
            let token_hash = hex::encode(Sha256::digest(raw_token.as_bytes()));
            models::create_app_token(&pool, &info.id.to_string(), app_record.id, &token_hash)
                .await
                .map_err(|e| eyre::eyre!("Failed to create app token mapping: {e}"))?;
            println!("Token created: {raw_token}");
            println!("(Save this token — it won't be shown again)");
            Ok(())
        }
    }
}

async fn bootstrap_roles(ath: &allowthem_core::AllowThem) -> eyre::Result<()> {
    for role_name in ["admin", "editor", "viewer"] {
        let rn = allowthem_core::RoleName::new(role_name);
        if ath.db().get_role_by_name(&rn).await?.is_none() {
            ath.db().create_role(&rn, None).await?;
        }
    }
    Ok(())
}

async fn create_initial_admin(
    config: Config,
    email: String,
    username: Option<String>,
    password: String,
) -> eyre::Result<()> {
    let pool = db::init_pool(&config.db_path).await?;
    let ath = allowthem_core::AllowThemBuilder::with_pool(pool)
        .cookie_secure(config.secure_cookies)
        .build()
        .await
        .map_err(|e| eyre::eyre!("Failed to initialize allowthem: {e}"))?;

    bootstrap_roles(&ath).await?;

    let users = ath.db().list_users().await.unwrap_or_default();
    if !users.is_empty() {
        eyre::bail!("Users already exist; create-admin is only for initial setup.");
    }

    if password.len() < 8 {
        eyre::bail!("Password must be at least 8 characters.");
    }

    let email =
        allowthem_core::Email::new(email).map_err(|e| eyre::eyre!("Invalid email address: {e}"))?;
    let username = username
        .map(|username| username.trim().to_string())
        .filter(|username| !username.is_empty())
        .map(allowthem_core::Username::new);

    let user = ath
        .db()
        .create_user(email, &password, username, None)
        .await
        .map_err(|e| eyre::eyre!("Failed to create admin user: {e}"))?;
    ath.db()
        .set_email_verified(user.id, true)
        .await
        .map_err(|e| eyre::eyre!("Failed to mark admin email verified: {e}"))?;

    let role = ath
        .db()
        .get_role_by_name(&allowthem_core::RoleName::new("admin"))
        .await?
        .ok_or_else(|| eyre::eyre!("Admin role was not created"))?;
    ath.db()
        .assign_role(&user.id, &role.id)
        .await
        .map_err(|e| eyre::eyre!("Failed to assign admin role: {e}"))?;

    println!("Admin user created: {}", user.email.as_str());
    Ok(())
}

fn spawn_registration_event_handler(
    ath: allowthem_core::AllowThem,
    pool: sqlx::SqlitePool,
    has_users: Arc<AtomicBool>,
    mut events_rx: allowthem_core::LifecycleEventReceiver,
) {
    tokio::spawn(async move {
        while let Some(event) = events_rx.recv().await {
            let allowthem_core::LifecycleEvent::Registered(e) = event else {
                continue;
            };
            has_users.store(true, std::sync::atomic::Ordering::Relaxed);
            match e.source {
                allowthem_core::RegistrationSource::Password => {
                    let user_count = ath.db().list_users().await.map(|u| u.len()).unwrap_or(0);
                    if user_count == 1 {
                        assign_role(&ath, &e.user.id, "admin").await;
                        tracing::info!(
                            user_id = %e.user.id,
                            "auto-assigned admin role to first registered user"
                        );
                    }
                }
                allowthem_core::RegistrationSource::Invitation { metadata, .. } => {
                    let role = match metadata.as_deref().unwrap_or("viewer") {
                        "admin" => "admin",
                        "editor" => "editor",
                        "viewer" => "viewer",
                        other => {
                            tracing::warn!(role = other, "invalid invitation role metadata");
                            "viewer"
                        }
                    };
                    if role != "admin"
                        && let Ok(apps) = models::list_apps(&pool).await
                    {
                        let user_id = e.user.id.to_string();
                        for app in apps {
                            if let Err(error) =
                                models::grant_app_access(&pool, app.id, &user_id).await
                            {
                                tracing::error!(
                                    %error,
                                    app_id = app.id,
                                    user_id,
                                    "failed to grant invited user app access"
                                );
                            }
                        }
                    }
                    assign_role(&ath, &e.user.id, role).await;
                }
                _ => {}
            }
        }
    });
}

async fn assign_role(
    ath: &allowthem_core::AllowThem,
    user_id: &allowthem_core::UserId,
    role: &str,
) {
    let role_name = allowthem_core::RoleName::new(role);
    if let Ok(Some(role)) = ath.db().get_role_by_name(&role_name).await {
        let _ = ath.db().assign_role(user_id, &role.id).await;
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

    // Email sender — SMTP when env is configured, LogEmailSender otherwise.
    let smtp_cfg = substrukt::email::SmtpConfig::from_env()?;
    let email_sender = substrukt::email::build_sender(smtp_cfg)?;

    // allowthem auth system (shares substrukt's pool)
    let csrf_key: [u8; 32] = rand::random();
    let ath = allowthem_core::AllowThemBuilder::with_pool(pool.clone())
        .cookie_secure(config.secure_cookies)
        .csrf_key(csrf_key)
        .email_sender(Box::new(email_sender.clone()))
        .build()
        .await
        .expect("Failed to initialize allowthem");

    bootstrap_roles(&ath).await?;

    // One-time data migration: move old users into allowthem
    let id_map = db::migration::migrate_users_to_allowthem(&pool, &ath).await?;
    db::migration::finalize_schema(&pool, &id_map).await?;

    // Grandfather existing users so the hard-block login policy does not lock them out.
    db::migration::grandfather_email_verification(&pool).await?;

    // Check if any users exist — after migration so migrated users count.
    let has_users = Arc::new(AtomicBool::new(
        !ath.db().list_users().await.unwrap_or_default().is_empty(),
    ));

    let auth_client: Arc<dyn allowthem_core::AuthClient> = Arc::new(
        allowthem_core::EmbeddedAuthClient::new(ath.clone(), "/login"),
    );

    // AllowThem built-in auth routes. Public browser registration is config
    // gated; invites still use the shared AllowThem register page.
    let scheme = if config.secure_cookies {
        "https"
    } else {
        "http"
    };
    let base_url = format!("{scheme}://localhost:{}", config.listen_port);
    let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel();
    let allowthem_auth_router = allowthem_server::AllRoutesBuilder::new()
        .login()
        .register()
        .logout()
        .password_reset()
        .base_url(base_url)
        .is_production(config.secure_cookies)
        .public_registration(config.registrations_enabled)
        .events(events_tx)
        .default_branding(
            allowthem_core::applications::BrandingConfig::new("substrukt")
                .with_accent("#f59e0b", allowthem_core::AccentInk::Black),
        )
        .build(&ath)
        .expect("Failed to build allowthem auth routes");
    spawn_registration_event_handler(ath.clone(), pool.clone(), has_users.clone(), events_rx);

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
        email_sender,
        has_users,
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

    let app = routes::build_router(state, allowthem_auth_router)
        .layer(axum::extract::DefaultBodyLimit::max(config.max_body_size))
        .layer(session_layer)
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let addr = format!("{}:{}", config.listen_addr, config.listen_port);
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("Listening on {addr}");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
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
