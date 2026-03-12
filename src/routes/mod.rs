pub mod api;
pub mod auth;
pub mod content;
pub mod schemas;
pub mod settings;
pub mod uploads;

use axum::{Router, extract::State, middleware, response::{Html, IntoResponse}};
use axum_htmx::HxRequest;
use tower_http::catch_panic::CatchPanicLayer;
use tower_sessions::Session;

use crate::auth::{require_auth, verify_csrf};
use crate::state::AppState;
use crate::templates::base_for_htmx;

pub fn build_router(state: AppState) -> Router {
    let api_routes = api::routes(state.clone());
    let auth_routes = auth::routes();
    let schema_routes = schemas::routes();
    let content_routes = content::routes();
    let upload_routes = uploads::routes();
    let settings_routes = settings::routes();

    Router::new()
        .merge(auth_routes)
        .nest("/schemas", schema_routes)
        .nest("/content", content_routes)
        .nest("/uploads", upload_routes)
        .nest("/settings", settings_routes)
        .route("/", axum::routing::get(dashboard))
        .layer(middleware::from_fn(verify_csrf))
        .layer(middleware::from_fn_with_state(state.clone(), require_auth))
        .nest("/api/v1", api_routes)
        .fallback(not_found)
        .layer(CatchPanicLayer::custom(handle_panic))
        .with_state(state)
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
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
) -> (axum::http::StatusCode, Html<String>) {
    let html = render_error(&state, 404, "Page not found", is_htmx);
    (axum::http::StatusCode::NOT_FOUND, Html(html))
}

pub fn render_error(state: &AppState, status: u16, message: &str, is_htmx: bool) -> String {
    let Ok(tmpl) = state.templates.acquire_env() else {
        return format!("<h1>{status}</h1><p>{message}</p>");
    };
    if let Ok(template) = tmpl.get_template("error.html") {
        if let Ok(html) = template.render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            status => status,
            message => message,
        }) {
            return html;
        }
    }
    format!("<h1>{status}</h1><p>{message}</p>")
}

async fn dashboard(
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
) -> axum::response::Result<Html<String>> {
    let csrf_token = crate::auth::ensure_csrf_token(&session).await;
    let schemas = crate::schema::list_schemas(&state.config.schemas_dir()).unwrap_or_default();
    let entry_count: usize = schemas
        .iter()
        .filter_map(|s| crate::content::list_entries(&state.config.content_dir(), s).ok())
        .map(|entries| entries.len())
        .sum();

    let tmpl = state.templates.acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("dashboard.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            schema_count => schemas.len(),
            entry_count => entry_count,
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html))
}
