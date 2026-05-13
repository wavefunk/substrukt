pub mod api;
pub mod apps;
pub mod auth;
pub mod content;
pub mod deployments;
pub mod schemas;
pub mod settings;
pub mod uploads;

use axum::{
    Router,
    extract::{OriginalUri, Request, State},
    http::StatusCode,
    middleware,
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_htmx::HxRequest;
use tower_http::catch_panic::CatchPanicLayer;

#[cfg(debug_assertions)]
use tower_http::services::ServeDir;

#[cfg(not(debug_assertions))]
use rust_embed::Embed;

#[cfg(not(debug_assertions))]
#[derive(Embed)]
#[folder = "static/"]
struct StaticAssets;

use crate::auth::{require_auth, verify_csrf};
use crate::metrics;
use crate::state::AppState;
use crate::templates::base_for_htmx;

pub fn build_router(state: AppState, allowthem_auth_router: Router) -> Router {
    let auth_routes = auth::routes();
    let settings_routes = settings::routes();
    let apps_management = apps::routes();
    let app_content = Router::new()
        .nest("/schemas", schemas::routes())
        .nest("/content", content::routes())
        .nest("/uploads", uploads::routes())
        .nest("/deployments", deployments::routes());

    let api_global = api::api_global_routes();
    let api_app_scoped = api::api_app_routes();
    let api_routes = Router::new()
        .merge(api_global)
        .nest("/apps/{app_slug}", api_app_scoped)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            api::api_rate_limit,
        ));

    let core = Router::new()
        .merge(auth_routes)
        .nest("/apps", apps_management)
        .nest("/apps/{app_slug}", app_content)
        .nest("/settings", settings_routes)
        .route("/", axum::routing::get(|| async { Redirect::to("/apps") }))
        .layer(middleware::from_fn_with_state(state.clone(), verify_csrf))
        .layer(middleware::from_fn_with_state(state.clone(), require_auth))
        .nest("/api/v1", api_routes)
        .route("/healthz", axum::routing::get(healthz))
        .route("/metrics", axum::routing::get(metrics::metrics_handler));

    #[cfg(debug_assertions)]
    let core = core.nest_service("/static", ServeDir::new("static"));

    #[cfg(not(debug_assertions))]
    let core = core.route("/static/{*path}", axum::routing::get(serve_embedded_asset));

    let core = core.fallback(not_found).with_state(state);

    core.merge(allowthem_auth_router)
        .layer(middleware::from_fn(metrics::track_metrics))
        .layer(CatchPanicLayer::custom(handle_panic))
        .layer(middleware::from_fn(security_headers))
}

/// Middleware that sets HTTP security headers on every response.
async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert("X-Frame-Options", "DENY".parse().unwrap());
    headers.insert("X-Content-Type-Options", "nosniff".parse().unwrap());
    headers.insert(
        "Referrer-Policy",
        "strict-origin-when-cross-origin".parse().unwrap(),
    );
    headers.insert("X-XSS-Protection", "1; mode=block".parse().unwrap());
    headers.insert(
        "Permissions-Policy",
        "camera=(), microphone=(), geolocation=()".parse().unwrap(),
    );
    response
}

fn handle_panic(_err: Box<dyn std::any::Any + Send + 'static>) -> axum::response::Response {
    let html = "<h1>500</h1><p>Internal server error</p>";
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        Html(html.to_string()),
    )
        .into_response()
}

async fn not_found(
    OriginalUri(uri): OriginalUri,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
) -> Response {
    // API routes get a plain 404 — no redirect, no HTML layout.
    if uri.path().starts_with("/api/") {
        return (axum::http::StatusCode::NOT_FOUND, "Not found").into_response();
    }

    // The fallback handler runs outside the require_auth middleware, so
    // user extensions may not be set. Render a simple 404 without nav.
    let html = render_error(&state, 404, "Page not found", is_htmx);
    (axum::http::StatusCode::NOT_FOUND, Html(html)).into_response()
}

pub fn render_error(state: &AppState, status: u16, message: &str, is_htmx: bool) -> String {
    render_error_with_nav(state, status, message, is_htmx, "", "", "", "")
}

pub fn error_response_with_nav(
    state: &AppState,
    status: StatusCode,
    message: &str,
    is_htmx: bool,
    user_role: &str,
    current_username: &str,
    csrf_token: &str,
    ath_csrf: &str,
) -> Response {
    let html = render_error_with_nav(
        state,
        status.as_u16(),
        message,
        is_htmx,
        user_role,
        current_username,
        csrf_token,
        ath_csrf,
    );
    (status, Html(html)).into_response()
}

pub fn render_error_with_nav(
    state: &AppState,
    status: u16,
    message: &str,
    is_htmx: bool,
    user_role: &str,
    current_username: &str,
    csrf_token: &str,
    ath_csrf: &str,
) -> String {
    let Ok(tmpl) = state.templates.acquire_env() else {
        return format!("<h1>{status}</h1><p>{message}</p>");
    };
    if let Ok(template) = tmpl.get_template("error.html")
        && let Ok(html) = template.render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            status => status,
            message => message,
            user_role => user_role,
            current_username => current_username,
            csrf_token => csrf_token,
            ath_csrf => ath_csrf,
        })
    {
        return html;
    }
    format!("<h1>{status}</h1><p>{message}</p>")
}

async fn healthz() -> &'static str {
    "ok"
}

#[cfg(not(debug_assertions))]
async fn serve_embedded_asset(axum::extract::Path(path): axum::extract::Path<String>) -> Response {
    use axum::http::{StatusCode, header};

    match StaticAssets::get(&path) {
        Some(file) => {
            let mime = mime_guess::from_path(&path)
                .first_or_octet_stream()
                .to_string();
            ([(header::CONTENT_TYPE, mime)], file.data).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
