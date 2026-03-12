pub mod token;

use axum::{
    extract::{Request, State},
    http::Method,
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use tower_sessions::Session;

use crate::db::models;
use crate::state::AppState;

const USER_ID_KEY: &str = "user_id";
const FLASH_KEY: &str = "_flash";
const CSRF_KEY: &str = "_csrf";

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

/// Get or create a CSRF token for this session.
pub async fn ensure_csrf_token(session: &Session) -> String {
    if let Ok(Some(token)) = session.get::<String>(CSRF_KEY).await {
        return token;
    }
    let token = hex::encode(rand::random::<[u8; 32]>());
    let _ = session.insert(CSRF_KEY, &token).await;
    token
}

/// Verify a submitted CSRF token against the session.
pub async fn verify_csrf_token(session: &Session, submitted: &str) -> bool {
    if let Ok(Some(expected)) = session.get::<String>(CSRF_KEY).await {
        expected == submitted
    } else {
        false
    }
}

/// Middleware: verify CSRF token on mutating requests (POST/PUT/DELETE).
/// Checks X-CSRF-Token header first, then _csrf form field for urlencoded bodies.
/// Multipart forms are passed through — handlers must verify _csrf from parsed fields.
pub async fn verify_csrf(request: Request, next: Next) -> Response {
    if matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) {
        return next.run(request).await;
    }

    let session = match request.extensions().get::<Session>().cloned() {
        Some(s) => s,
        None => return next.run(request).await,
    };

    // Check X-CSRF-Token header (used by fetch/DELETE requests)
    if let Some(token) = request
        .headers()
        .get("X-CSRF-Token")
        .and_then(|v| v.to_str().ok())
    {
        if verify_csrf_token(&session, token).await {
            return next.run(request).await;
        }
        return (axum::http::StatusCode::FORBIDDEN, "Invalid CSRF token").into_response();
    }

    let content_type = request
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // For urlencoded forms, extract _csrf from body
    if content_type.starts_with("application/x-www-form-urlencoded") {
        let (parts, body) = request.into_parts();
        let bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
            Ok(b) => b,
            Err(_) => {
                return (axum::http::StatusCode::BAD_REQUEST, "Body too large").into_response();
            }
        };

        // CSRF tokens are hex — no URL decoding needed
        let body_str = std::str::from_utf8(&bytes).unwrap_or("");
        let csrf_value = body_str
            .split('&')
            .find_map(|pair| pair.strip_prefix("_csrf="));

        if let Some(token) = csrf_value
            && verify_csrf_token(&session, token).await
        {
            let request = Request::from_parts(parts, axum::body::Body::from(bytes));
            return next.run(request).await;
        }

        return (axum::http::StatusCode::FORBIDDEN, "Invalid CSRF token").into_response();
    }

    // Multipart: handler must verify _csrf from parsed fields
    if content_type.starts_with("multipart/form-data") {
        return next.run(request).await;
    }

    next.run(request).await
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
