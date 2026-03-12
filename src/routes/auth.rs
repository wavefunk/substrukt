use axum::{
    Form, Router,
    extract::State,
    http::HeaderMap,
    response::{Html, IntoResponse, Redirect},
    routing::get,
};
use tower_sessions::Session;

use crate::auth::{self, ensure_csrf_token};
use crate::db::models;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/login", get(login_page).post(login_submit))
        .route("/logout", axum::routing::post(logout))
        .route("/setup", get(setup_page).post(setup_submit))
}

#[derive(serde::Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
}

#[derive(serde::Deserialize)]
pub struct SetupForm {
    username: String,
    password: String,
    confirm_password: String,
}

async fn login_page(
    State(state): State<AppState>,
    session: Session,
) -> axum::response::Result<Html<String>> {
    let csrf_token = ensure_csrf_token(&session).await;
    render_template(&state, "login.html", minijinja::context! { csrf_token => csrf_token }).await
}

async fn login_submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    session: Session,
    Form(form): Form<LoginForm>,
) -> impl IntoResponse {
    let ip = client_ip(&headers);
    if !state.login_limiter.check(&ip) {
        return (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "Too many login attempts. Please try again later.",
        )
            .into_response();
    }

    let user = models::find_user_by_username(&state.pool, &form.username).await;
    match user {
        Ok(Some(user)) if user.verify_password(&form.password) => {
            if let Err(e) = auth::login_user(&session, user.id).await {
                tracing::error!("Failed to create session: {e}");
                return Redirect::to("/login").into_response();
            }
            Redirect::to("/").into_response()
        }
        _ => {
            let csrf_token = ensure_csrf_token(&session).await;
            let html = render_template(
                &state,
                "login.html",
                minijinja::context! {
                    csrf_token => csrf_token,
                    error => "Invalid username or password",
                },
            )
            .await;
            match html {
                Ok(h) => h.into_response(),
                Err(_) => Redirect::to("/login").into_response(),
            }
        }
    }
}

async fn logout(session: Session) -> Redirect {
    let _ = auth::logout_user(&session).await;
    Redirect::to("/login")
}

async fn setup_page(
    State(state): State<AppState>,
    session: Session,
) -> axum::response::Result<impl IntoResponse> {
    // If users already exist, redirect to login
    let count = models::user_count(&state.pool)
        .await
        .map_err(|e| format!("DB error: {e}"))?;
    if count > 0 {
        return Ok(Redirect::to("/login").into_response());
    }
    let csrf_token = ensure_csrf_token(&session).await;
    let html = render_template(&state, "setup.html", minijinja::context! { csrf_token => csrf_token }).await?;
    Ok(html.into_response())
}

async fn setup_submit(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<SetupForm>,
) -> impl IntoResponse {
    // Check no users exist
    let count = models::user_count(&state.pool).await.unwrap_or(1);
    if count > 0 {
        return Redirect::to("/login").into_response();
    }

    if form.password != form.confirm_password {
        let csrf_token = ensure_csrf_token(&session).await;
        let html = render_template(
            &state,
            "setup.html",
            minijinja::context! {
                csrf_token => csrf_token,
                error => "Passwords do not match",
            },
        )
        .await;
        return match html {
            Ok(h) => h.into_response(),
            Err(_) => Redirect::to("/setup").into_response(),
        };
    }

    if form.username.is_empty() || form.password.len() < 8 {
        let csrf_token = ensure_csrf_token(&session).await;
        let html = render_template(
            &state,
            "setup.html",
            minijinja::context! {
                csrf_token => csrf_token,
                error => "Username required, password must be at least 8 characters",
            },
        )
        .await;
        return match html {
            Ok(h) => h.into_response(),
            Err(_) => Redirect::to("/setup").into_response(),
        };
    }

    match models::create_user(&state.pool, &form.username, &form.password).await {
        Ok(user) => {
            let _ = auth::login_user(&session, user.id).await;
            Redirect::to("/").into_response()
        }
        Err(e) => {
            tracing::error!("Failed to create user: {e}");
            Redirect::to("/setup").into_response()
        }
    }
}

async fn render_template(
    state: &AppState,
    name: &str,
    ctx: minijinja::Value,
) -> axum::response::Result<Html<String>> {
    let tmpl = state.templates.acquire_env().map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template(name)
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(ctx)
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html))
}

fn client_ip(headers: &HeaderMap) -> String {
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first_ip) = xff.split(',').next() {
            return first_ip.trim().to_string();
        }
    }
    "unknown".to_string()
}
