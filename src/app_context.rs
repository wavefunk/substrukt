use std::collections::HashMap;

use axum::extract::{FromRequestParts, Path};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{Html, IntoResponse, Json, Response};
use axum_htmx::HxRequest;
use tower_sessions::Session;

use crate::auth::{CurrentUserRole, ensure_csrf_token};
use crate::config::Config;
use crate::db::models::{self, App};
use crate::routes::render_error_with_nav;
use crate::schema;
use crate::state::AppState;

pub struct AppContext {
    pub app: App,
}

impl AppContext {
    pub fn nav_schemas(&self, config: &Config) -> Vec<minijinja::Value> {
        let schemas_dir = config.app_schemas_dir(&self.app.slug);
        match schema::list_schemas(&schemas_dir) {
            Ok(schemas) => schemas
                .iter()
                .map(|s| {
                    minijinja::context! {
                        title => s.meta.title,
                        slug => s.meta.slug,
                    }
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn template_context(&self) -> minijinja::Value {
        minijinja::context! {
            id => self.app.id,
            slug => self.app.slug,
            name => self.app.name,
        }
    }
}

impl FromRequestParts<AppState> for AppContext {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let HxRequest(is_htmx) = HxRequest::from_request_parts(parts, state)
            .await
            .unwrap_or(HxRequest(false));

        // Get user info from extensions (set by require_auth middleware)
        let user = parts.extensions.get::<allowthem_core::User>().cloned();
        let role = parts
            .extensions
            .get::<CurrentUserRole>()
            .map(|r| r.0.clone())
            .unwrap_or_default();
        let current_username = user
            .as_ref()
            .and_then(|u| u.username.as_ref())
            .map(|u| u.as_str().to_string())
            .unwrap_or_default();

        // CSRF from tower-session
        let session = parts.extensions.get::<Session>().cloned();
        let csrf_token = if let Some(ref s) = session {
            ensure_csrf_token(s).await
        } else {
            String::new()
        };

        let err_nav = |status: u16, msg: &str| {
            let html = render_error_with_nav(
                state, status, msg, is_htmx, &role, &current_username, &csrf_token,
            );
            (
                StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Html(html),
            )
                .into_response()
        };

        let params: HashMap<String, String> =
            match Path::<HashMap<String, String>>::from_request_parts(parts, state).await {
                Ok(Path(params)) => params,
                Err(_) => return Err(err_nav(404, "Not found")),
            };

        let slug = params
            .get("app_slug")
            .ok_or_else(|| err_nav(404, "Not found"))?;

        let app = models::find_app_by_slug(&state.pool, slug)
            .await
            .map_err(|_| err_nav(500, "Internal error"))?
            .ok_or_else(|| err_nav(404, "App not found"))?;

        // Auth check
        let user = user.ok_or_else(|| err_nav(403, "Not authenticated"))?;

        // Admins have access to all apps; others need explicit access
        if role != "admin" {
            let has_access =
                models::user_has_app_access(&state.pool, app.id, &user.id.to_string())
                    .await
                    .map_err(|_| err_nav(500, "Internal error"))?;
            if !has_access {
                return Err(err_nav(403, "You do not have access to this app"));
            }
        }

        Ok(AppContext { app })
    }
}

/// API route extractor — resolves app, no auth check (bearer does that).
pub struct ApiAppContext {
    pub app: App,
}

impl FromRequestParts<AppState> for ApiAppContext {
    type Rejection = (StatusCode, Json<serde_json::Value>);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let params: HashMap<String, String> =
            match Path::<HashMap<String, String>>::from_request_parts(parts, state).await {
                Ok(Path(params)) => params,
                Err(_) => {
                    return Err((
                        StatusCode::NOT_FOUND,
                        Json(serde_json::json!({"error": "Not found"})),
                    ));
                }
            };

        let slug = params.get("app_slug").ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Not found"})),
            )
        })?;

        let app = models::find_app_by_slug(&state.pool, slug)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": "Internal error"})),
                )
            })?
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "App not found"})),
                )
            })?;

        Ok(ApiAppContext { app })
    }
}
