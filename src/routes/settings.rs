use axum::{
    Form, Router,
    extract::State,
    response::{Html, IntoResponse, Redirect},
    routing::get,
};
use axum_htmx::HxRequest;
use tower_sessions::Session;

use crate::auth;
use crate::auth::token;
use crate::db::models;
use crate::state::AppState;
use crate::templates::base_for_htmx;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/tokens", get(tokens_page).post(create_token))
        .route(
            "/tokens/{token_id}/delete",
            axum::routing::post(delete_token),
        )
}

async fn tokens_page(
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
) -> axum::response::Result<Html<String>> {
    let user_id = auth::current_user_id(&session)
        .await
        .ok_or("Not authenticated")?;

    let tokens = models::list_api_tokens(&state.pool, user_id)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    let csrf_token = auth::ensure_csrf_token(&session).await;

    let token_data: Vec<minijinja::Value> = tokens
        .iter()
        .map(|t| {
            minijinja::context! {
                id => t.id,
                name => t.name,
                created_at => t.created_at,
            }
        })
        .collect();

    let tmpl = state.templates.acquire_env().map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("settings/tokens.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            tokens => token_data,
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html))
}

#[derive(serde::Deserialize)]
pub struct CreateTokenForm {
    name: String,
}

async fn create_token(
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<CreateTokenForm>,
) -> axum::response::Result<Html<String>> {
    let user_id = auth::current_user_id(&session)
        .await
        .ok_or("Not authenticated")?;

    let raw_token = token::generate_token();
    let token_hash = token::hash_token(&raw_token);

    let api_token = models::create_api_token(&state.pool, user_id, &form.name, &token_hash)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    state.audit.log(&user_id.to_string(), "token_create", "api_token", &api_token.id.to_string(), None);

    let tokens = models::list_api_tokens(&state.pool, user_id)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    let token_data: Vec<minijinja::Value> = tokens
        .iter()
        .map(|t| {
            minijinja::context! {
                id => t.id,
                name => t.name,
                created_at => t.created_at,
            }
        })
        .collect();

    let tmpl = state.templates.acquire_env().map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("settings/tokens.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            tokens => token_data,
            new_token => raw_token,
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html))
}

async fn delete_token(
    State(state): State<AppState>,
    session: Session,
    axum::extract::Path(token_id): axum::extract::Path<i64>,
) -> impl IntoResponse {
    let user_id = match auth::current_user_id(&session).await {
        Some(id) => id,
        None => return Redirect::to("/login"),
    };

    let _ = models::delete_api_token(&state.pool, token_id, user_id).await;
    state.audit.log(&user_id.to_string(), "token_delete", "api_token", &token_id.to_string(), None);
    Redirect::to("/settings/tokens")
}
