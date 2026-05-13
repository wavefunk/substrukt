pub mod token;

use axum::{
    extract::{Request, State},
    http::Method,
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
};
use tower_sessions::Session;

use crate::state::AppState;

/// Return a redirect response that works correctly with htmx.
/// If the request came from htmx (has HX-Request header), returns a 200 with
/// HX-Redirect header so htmx performs a full page navigation instead of
/// swapping the response into the content area.
/// Otherwise, returns a standard HTTP redirect.
fn htmx_aware_redirect(request: &Request, location: &str) -> Response {
    let is_htmx = request.headers().get("HX-Request").is_some();
    if is_htmx {
        (
            [(
                axum::http::header::HeaderName::from_static("hx-redirect"),
                axum::http::header::HeaderValue::from_str(location)
                    .expect("redirect location is valid header value"),
            )],
            "",
        )
            .into_response()
    } else {
        Redirect::to(location).into_response()
    }
}

const FLASH_KEY: &str = "_flash";
const CSRF_KEY: &str = "_csrf";
const ATH_CSRF_KEY: &str = "_ath_csrf";

/// Get the current authenticated user from request extensions.
pub fn current_user(request: &Request) -> Option<&allowthem_core::User> {
    request.extensions().get::<allowthem_core::User>()
}

/// Get cached user role from request extensions.
pub fn current_user_role_from_ext(extensions: &axum::http::Extensions) -> Option<String> {
    extensions.get::<CurrentUserRole>().map(|r| r.0.clone())
}

/// Newtype to store the user's primary role in request extensions.
#[derive(Clone)]
pub struct CurrentUserRole(pub String);

/// AllowThem's session-bound CSRF token, derived in require_auth and used by
/// templates for POSTing to AllowThem routes (e.g. logout).
#[derive(Clone)]
pub struct AllowThemCsrf(pub String);

/// Check that the current user has at least the given role level.
/// Role hierarchy: admin > editor > viewer.
/// Returns the user's UUID on success, or a 403 response on failure.
pub fn require_role(
    extensions: &axum::http::Extensions,
    min_role: &str,
) -> axum::response::Result<allowthem_core::UserId> {
    let user =
        extensions
            .get::<allowthem_core::User>()
            .ok_or(axum::response::ErrorResponse::from(
                Redirect::to("/login").into_response(),
            ))?;
    let role = extensions
        .get::<CurrentUserRole>()
        .map(|r| r.0.as_str())
        .unwrap_or("");

    let role_level = |r: &str| -> u8 {
        match r {
            "admin" => 3,
            "editor" => 2,
            "viewer" => 1,
            _ => 0,
        }
    };

    if role_level(role) >= role_level(min_role) {
        Ok(user.id)
    } else {
        Err(axum::response::ErrorResponse::from(
            (
                axum::http::StatusCode::FORBIDDEN,
                "Insufficient permissions",
            )
                .into_response(),
        ))
    }
}

/// Check if a role string meets a minimum role level.
/// Role hierarchy: admin > editor > viewer.
pub fn has_min_role(role: &str, min_role: &str) -> bool {
    let level = |r: &str| -> u8 {
        match r {
            "admin" => 3,
            "editor" => 2,
            "viewer" => 1,
            _ => 0,
        }
    };
    level(role) >= level(min_role)
}

// --- Flash / CSRF (unchanged, still use tower-sessions) ---

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

pub fn flash_echo_trigger(
    flash: &Option<(String, String)>,
) -> Option<axum_htmx::HxResponseTrigger> {
    let (kind, msg) = flash.as_ref()?;
    let echo_kind = match kind.as_str() {
        "success" => "ok",
        "error" => "err",
        _ => return None,
    };
    let event = axum_htmx::HxEvent::new_with_data(
        "wfEcho",
        serde_json::json!({"kind": echo_kind, "msg": msg}),
    )
    .ok()?;
    Some(axum_htmx::HxResponseTrigger::after_settle([event]))
}

/// Read the AllowThem session-bound CSRF token stashed by require_auth.
/// Used in template contexts for forms that POST to AllowThem routes.
pub async fn ath_csrf(session: &Session) -> String {
    session
        .get::<String>(ATH_CSRF_KEY)
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
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
        if expected.len() != submitted.len() {
            return false;
        }
        use subtle::ConstantTimeEq;
        expected.as_bytes().ct_eq(submitted.as_bytes()).into()
    } else {
        false
    }
}

/// Middleware: verify CSRF token on mutating requests (POST/PUT/DELETE).
/// Checks X-CSRF-Token header first, then _csrf form field for urlencoded bodies.
/// Multipart forms are passed through — handlers must verify _csrf from parsed fields.
pub async fn verify_csrf(State(state): State<AppState>, request: Request, next: Next) -> Response {
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
        return csrf_error_response(&state);
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

        return csrf_error_response(&state);
    }

    // Multipart: handler must verify _csrf from parsed fields
    if content_type.starts_with("multipart/form-data") {
        return next.run(request).await;
    }

    next.run(request).await
}

fn csrf_error_response(state: &AppState) -> Response {
    use axum::response::Html;
    let html = crate::routes::render_error(
        state,
        403,
        "Your session may have expired. Please go back and try again.",
        false,
    );
    (axum::http::StatusCode::FORBIDDEN, Html(html)).into_response()
}

/// Middleware: validate allowthem session cookie. Redirect to /register or /login if needed.
/// On success, inserts `allowthem_core::User` and `CurrentUserRole` into request extensions.
pub async fn require_auth(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();

    // Allow public paths (AllowThem handles all auth UI)
    if path.starts_with("/login")
        || path.starts_with("/register")
        || path.starts_with("/logout")
        || path.starts_with("/forgot-password")
        || path.starts_with("/auth/reset-password")
        || path.starts_with("/__allowthem/")
        || path.starts_with("/api/")
    {
        return next.run(request).await;
    }

    if !state.has_users.load(std::sync::atomic::Ordering::Relaxed) {
        if state.config.registrations_enabled {
            return htmx_aware_redirect(&request, "/register");
        }
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Html("No admin user configured. Run `substrukt create-admin` from the server CLI."),
        )
            .into_response();
    }

    // Parse allowthem session cookie
    let cookie_header = request
        .headers()
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok());

    let token = cookie_header.and_then(|h| state.ath.parse_session_cookie(h));

    let token = match token {
        Some(t) => t,
        None => return htmx_aware_redirect(&request, "/login"),
    };

    // Validate session
    let user = match state.auth_client.validate_session(&token).await {
        Ok(Some(u)) => u,
        _ => return htmx_aware_redirect(&request, "/login"),
    };

    // Resolve primary role
    let role = resolve_user_role(&state, &user.id).await;

    // Derive AllowThem's session-bound CSRF token and stash in the
    // tower-session so templates can include it in POST forms that target
    // AllowThem routes (e.g. /logout).
    if let Ok(key) = state.ath.csrf_key() {
        let csrf = allowthem_core::derive_csrf_token(&token, key);
        if let Some(sess) = request.extensions().get::<Session>().cloned() {
            let _ = sess.insert(ATH_CSRF_KEY, &csrf).await;
        }
    }

    request.extensions_mut().insert(CurrentUserRole(role));
    request.extensions_mut().insert(user);
    next.run(request).await
}

/// Determine the user's highest role. Checks admin > editor > viewer.
pub(crate) async fn resolve_user_role(
    state: &AppState,
    user_id: &allowthem_core::UserId,
) -> String {
    for role_name in ["admin", "editor", "viewer"] {
        let rn = allowthem_core::RoleName::new(role_name);
        if state
            .auth_client
            .check_role(user_id, &rn)
            .await
            .unwrap_or(false)
        {
            return role_name.to_string();
        }
    }
    String::new()
}
