pub mod token;

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use tower_sessions::Session;

use crate::db::models;
use crate::state::AppState;

const USER_ID_KEY: &str = "user_id";
const FLASH_KEY: &str = "_flash";

pub async fn login_user(session: &Session, user_id: i64) -> eyre::Result<()> {
    session
        .insert(USER_ID_KEY, user_id)
        .await
        .map_err(|e| eyre::eyre!("Session insert error: {e}"))?;
    Ok(())
}

pub async fn logout_user(session: &Session) -> eyre::Result<()> {
    session
        .flush()
        .await
        .map_err(|e| eyre::eyre!("Session flush error: {e}"))?;
    Ok(())
}

pub async fn current_user_id(session: &Session) -> Option<i64> {
    session.get::<i64>(USER_ID_KEY).await.ok().flatten()
}

/// Store a flash message in the session. It will be consumed on next page load.
pub async fn set_flash(session: &Session, kind: &str, message: &str) {
    let flash = serde_json::json!({"kind": kind, "message": message});
    let _ = session.insert(FLASH_KEY, flash).await;
}

/// Consume and return the flash message from the session, if any.
pub async fn take_flash(session: &Session) -> Option<(String, String)> {
    if let Ok(Some(flash)) = session.get::<serde_json::Value>(FLASH_KEY).await {
        let _ = session.remove::<serde_json::Value>(FLASH_KEY).await;
        let kind = flash["kind"].as_str().unwrap_or("info").to_string();
        let message = flash["message"].as_str().unwrap_or("").to_string();
        Some((kind, message))
    } else {
        None
    }
}

/// Middleware: redirect to /setup if no users exist, or to /login if not authenticated.
/// Session is extracted from request extensions (set by the session layer below this).
pub async fn require_auth(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();

    // Allow public paths
    if path.starts_with("/login")
        || path.starts_with("/setup")
        || path.starts_with("/api/")
        || path.starts_with("/uploads/file/")
    {
        return next.run(request).await;
    }

    // Check if any users exist
    let user_count = models::user_count(&state.pool).await.unwrap_or(0);
    if user_count == 0 {
        return Redirect::to("/setup").into_response();
    }

    // Check session - get from request extensions
    let session = request.extensions().get::<Session>().cloned();
    match session {
        Some(session) => {
            if current_user_id(&session).await.is_none() {
                return Redirect::to("/login").into_response();
            }
        }
        None => {
            return Redirect::to("/login").into_response();
        }
    }

    next.run(request).await
}
