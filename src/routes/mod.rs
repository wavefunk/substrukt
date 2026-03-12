pub mod api;
pub mod auth;
pub mod content;
pub mod schemas;
pub mod settings;
pub mod uploads;

use axum::{Router, extract::State, middleware, response::Html};

use crate::auth::require_auth;
use crate::state::AppState;

pub fn build_router(state: AppState) -> Router {
    let api_routes = api::routes();
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
        .layer(middleware::from_fn_with_state(state.clone(), require_auth))
        .nest("/api/v1", api_routes)
        .fallback(not_found)
        .with_state(state)
}

async fn not_found(State(state): State<AppState>) -> (axum::http::StatusCode, Html<String>) {
    let html = render_error(&state, 404, "Page not found").await;
    (axum::http::StatusCode::NOT_FOUND, Html(html))
}

pub async fn render_error(state: &AppState, status: u16, message: &str) -> String {
    let tmpl = state.templates.read().await;
    if let Ok(template) = tmpl.get_template("error.html") {
        if let Ok(html) = template.render(minijinja::context! {
            status => status,
            message => message,
        }) {
            return html;
        }
    }
    format!("<h1>{status}</h1><p>{message}</p>")
}

async fn dashboard(State(state): State<AppState>) -> axum::response::Result<Html<String>> {
    let schemas = crate::schema::list_schemas(&state.config.schemas_dir()).unwrap_or_default();
    let entry_count: usize = schemas
        .iter()
        .filter_map(|s| crate::content::list_entries(&state.config.content_dir(), s).ok())
        .map(|entries| entries.len())
        .sum();

    let tmpl = state.templates.read().await;
    let template = tmpl
        .get_template("dashboard.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            schema_count => schemas.len(),
            entry_count => entry_count,
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html))
}
