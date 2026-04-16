use axum::{
    extract::State,
    response::{Html, IntoResponse, Redirect, Response},
    Form,
};
use tower_sessions::Session;

use crate::auth::ensure_csrf_token;
use crate::state::AppState;

pub fn routes() -> axum::Router<AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/login", get(login_page).post(login_submit))
        .route("/logout", post(logout))
        .route("/setup", get(setup_page).post(setup_submit))
        .route("/signup", get(signup_page).post(signup_submit))
}

#[derive(serde::Deserialize)]
struct LoginForm {
    username: String,
    password: String,
}

#[derive(serde::Deserialize)]
struct SetupForm {
    username: String,
    password: String,
    confirm_password: String,
}

async fn login_page(session: Session, State(state): State<AppState>) -> impl IntoResponse {
    let csrf_token = ensure_csrf_token(&session).await;
    render_template(
        &state,
        "login.html",
        minijinja::context! { csrf_token => csrf_token },
    )
}

async fn login_submit(
    session: Session,
    State(state): State<AppState>,
    Form(form): Form<LoginForm>,
) -> Response {
    let ip = "direct".to_string();
    if !state.login_limiter.check(&ip) {
        let csrf_token = ensure_csrf_token(&session).await;
        return render_template(
            &state,
            "login.html",
            minijinja::context! {
                csrf_token => csrf_token,
                error => "Too many login attempts. Please try again later.",
            },
        )
        .into_response();
    }

    // Find user by email or username
    let user = match state.ath.db().find_for_login(&form.username).await {
        Ok(u) => u,
        Err(_) => {
            let csrf_token = ensure_csrf_token(&session).await;
            return render_template(
                &state,
                "login.html",
                minijinja::context! {
                    csrf_token => csrf_token,
                    error => "Invalid username or password.",
                },
            )
            .into_response();
        }
    };

    // Verify password
    let hash = match &user.password_hash {
        Some(h) => h,
        None => {
            let csrf_token = ensure_csrf_token(&session).await;
            return render_template(
                &state,
                "login.html",
                minijinja::context! {
                    csrf_token => csrf_token,
                    error => "Invalid username or password.",
                },
            )
            .into_response();
        }
    };

    match allowthem_core::password::verify_password(&form.password, hash) {
        Ok(true) => {}
        _ => {
            let csrf_token = ensure_csrf_token(&session).await;
            return render_template(
                &state,
                "login.html",
                minijinja::context! {
                    csrf_token => csrf_token,
                    error => "Invalid username or password.",
                },
            )
            .into_response();
        }
    }

    // Create allowthem session
    let token = allowthem_core::generate_token();
    let token_hash = allowthem_core::hash_token(&token);
    let expires = chrono::Utc::now() + state.ath.session_config().ttl;
    if let Err(e) = state
        .ath
        .db()
        .create_session(user.id, token_hash, None, None, expires)
        .await
    {
        tracing::error!("Failed to create session: {e}");
        let csrf_token = ensure_csrf_token(&session).await;
        return render_template(
            &state,
            "login.html",
            minijinja::context! {
                csrf_token => csrf_token,
                error => "Login failed. Please try again.",
            },
        )
        .into_response();
    }

    let _ = state
        .ath
        .db()
        .log_audit(
            allowthem_core::AuditEvent::Login,
            Some(&user.id),
            None,
            None,
            None,
            None,
        )
        .await;

    // Set cookie and redirect
    let cookie = state.ath.session_cookie(&token);
    let mut resp = Redirect::to("/apps").into_response();
    resp.headers_mut().insert(
        axum::http::header::SET_COOKIE,
        cookie.parse().unwrap(),
    );
    resp
}

async fn logout(
    State(state): State<AppState>,
    session: Session,
    request: axum::extract::Request,
) -> Response {
    // Parse and invalidate allowthem session
    if let Some(cookie_header) = request
        .headers()
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(token) = state.ath.parse_session_cookie(cookie_header) {
            let _ = state.auth_client.logout(&token).await;
        }
    }

    // Flush tower-session (flash/CSRF)
    let _ = session.flush().await;

    // Clear allowthem cookie
    let clear_cookie = format!(
        "{}=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax",
        state.auth_client.session_cookie_name()
    );
    let mut resp = Redirect::to("/login").into_response();
    resp.headers_mut().insert(
        axum::http::header::SET_COOKIE,
        clear_cookie.parse().unwrap(),
    );
    resp
}

async fn setup_page(session: Session, State(state): State<AppState>) -> Response {
    if state
        .has_users
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        return Redirect::to("/login").into_response();
    }
    let csrf_token = ensure_csrf_token(&session).await;
    render_template(
        &state,
        "setup.html",
        minijinja::context! { csrf_token => csrf_token },
    )
    .into_response()
}

async fn setup_submit(
    session: Session,
    State(state): State<AppState>,
    Form(form): Form<SetupForm>,
) -> Response {
    if state
        .has_users
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        return Redirect::to("/login").into_response();
    }

    if form.password != form.confirm_password {
        let csrf_token = ensure_csrf_token(&session).await;
        return render_template(
            &state,
            "setup.html",
            minijinja::context! {
                csrf_token => csrf_token,
                error => "Passwords do not match.",
            },
        )
        .into_response();
    }
    if form.password.len() < 8 {
        let csrf_token = ensure_csrf_token(&session).await;
        return render_template(
            &state,
            "setup.html",
            minijinja::context! {
                csrf_token => csrf_token,
                error => "Password must be at least 8 characters.",
            },
        )
        .into_response();
    }

    let email = match allowthem_core::Email::new(format!("{}@setup.local", form.username)) {
        Ok(e) => e,
        Err(_) => {
            let csrf_token = ensure_csrf_token(&session).await;
            return render_template(
                &state,
                "setup.html",
                minijinja::context! {
                    csrf_token => csrf_token,
                    error => "Invalid username.",
                },
            )
            .into_response();
        }
    };
    let username = allowthem_core::Username::new(form.username.clone());

    let user = match state
        .ath
        .db()
        .create_user(email, &form.password, Some(username))
        .await
    {
        Ok(u) => u,
        Err(e) => {
            let csrf_token = ensure_csrf_token(&session).await;
            return render_template(
                &state,
                "setup.html",
                minijinja::context! {
                    csrf_token => csrf_token,
                    error => format!("Failed to create user: {e}"),
                },
            )
            .into_response();
        }
    };

    // Assign admin role
    let admin_role_name = allowthem_core::RoleName::new("admin");
    if let Ok(Some(role)) = state.ath.db().get_role_by_name(&admin_role_name).await {
        let _ = state.ath.db().assign_role(&user.id, &role.id).await;
    }

    // Mark that users exist
    state
        .has_users
        .store(true, std::sync::atomic::Ordering::Relaxed);

    // Create session and set cookie
    let token = allowthem_core::generate_token();
    let token_hash = allowthem_core::hash_token(&token);
    let expires = chrono::Utc::now() + state.ath.session_config().ttl;
    let _ = state
        .ath
        .db()
        .create_session(user.id, token_hash, None, None, expires)
        .await;

    let _ = state
        .ath
        .db()
        .log_audit(
            allowthem_core::AuditEvent::Register,
            Some(&user.id),
            None,
            None,
            None,
            None,
        )
        .await;

    let cookie = state.ath.session_cookie(&token);
    let mut resp = Redirect::to("/apps").into_response();
    resp.headers_mut().insert(
        axum::http::header::SET_COOKIE,
        cookie.parse().unwrap(),
    );
    resp
}

#[derive(serde::Deserialize)]
struct SignupQuery {
    token: Option<String>,
}

#[derive(serde::Deserialize)]
struct SignupForm {
    token: String,
    username: String,
    password: String,
    confirm_password: String,
}

async fn signup_page(
    session: Session,
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<SignupQuery>,
) -> Response {
    let raw_token = match query.token {
        Some(t) => t,
        None => return render_error_page(&state, "Invalid signup link."),
    };

    let invitation = match state.ath.db().validate_invitation(&raw_token).await {
        Ok(Some(inv)) => inv,
        _ => return render_error_page(&state, "This invitation link is invalid or has expired."),
    };

    let csrf_token = ensure_csrf_token(&session).await;
    let email = invitation.email.as_ref().map(|e| e.as_str()).unwrap_or("");
    render_template(
        &state,
        "signup.html",
        minijinja::context! {
            csrf_token => csrf_token,
            token => raw_token,
            email => email,
        },
    )
    .into_response()
}

async fn signup_submit(
    session: Session,
    State(state): State<AppState>,
    Form(form): Form<SignupForm>,
) -> Response {
    let ip = "direct".to_string();
    if !state.login_limiter.check(&ip) {
        return render_error_page(&state, "Too many attempts. Please try again later.");
    }

    // Validate invitation
    let invitation = match state.ath.db().validate_invitation(&form.token).await {
        Ok(Some(inv)) => inv,
        _ => return render_error_page(&state, "This invitation link is invalid or has expired."),
    };

    // Validate form fields
    if form.username.trim().is_empty() {
        return render_signup_error(&state, &session, &form.token, &invitation, "Username is required.").await;
    }
    if form.password.len() < 8 {
        return render_signup_error(&state, &session, &form.token, &invitation, "Password must be at least 8 characters.").await;
    }
    if form.password != form.confirm_password {
        return render_signup_error(&state, &session, &form.token, &invitation, "Passwords do not match.").await;
    }

    // Check username uniqueness
    let username = allowthem_core::Username::new(form.username.clone());
    if state.ath.db().get_user_by_username(&username).await.is_ok() {
        return render_signup_error(&state, &session, &form.token, &invitation, "Username is already taken.").await;
    }

    // Determine email: use invitation email if provided, otherwise derive from username
    let email = match invitation.email.clone() {
        Some(e) => e,
        None => match allowthem_core::Email::new(format!("{}@signup.local", form.username)) {
            Ok(e) => e,
            Err(_) => {
                return render_signup_error(
                    &state,
                    &session,
                    &form.token,
                    &invitation,
                    "Invalid username — cannot derive email.",
                )
                .await;
            }
        },
    };

    // Create user
    let user = match state.ath.db().create_user(email, &form.password, Some(username)).await {
        Ok(u) => u,
        Err(e) => {
            return render_signup_error(
                &state,
                &session,
                &form.token,
                &invitation,
                &format!("Failed to create account: {e}"),
            )
            .await;
        }
    };

    // Consume invitation (best-effort — race is unlikely, but we don't fail on it)
    let _ = state.ath.db().consume_invitation(invitation.id).await;

    // Assign role from invitation metadata (default: editor)
    let role_str = invitation.metadata.as_deref().unwrap_or("editor");
    let role_name = allowthem_core::RoleName::new(role_str);
    if let Ok(Some(role)) = state.ath.db().get_role_by_name(&role_name).await {
        let _ = state.ath.db().assign_role(&user.id, &role.id).await;
    }

    // Auto-grant access to all apps for non-admin users
    if role_str != "admin" {
        if let Ok(apps) = crate::db::models::list_apps(&state.pool).await {
            let uid = user.id.to_string();
            for app in apps {
                let _ = crate::db::models::grant_app_access(&state.pool, app.id, &uid).await;
            }
        }
    }

    state.has_users.store(true, std::sync::atomic::Ordering::Relaxed);

    // Create session
    let token = allowthem_core::generate_token();
    let token_hash = allowthem_core::hash_token(&token);
    let expires = chrono::Utc::now() + state.ath.session_config().ttl;
    let _ = state.ath.db().create_session(user.id, token_hash, None, None, expires).await;

    let _ = state
        .ath
        .db()
        .log_audit(
            allowthem_core::AuditEvent::Register,
            Some(&user.id),
            None,
            None,
            None,
            None,
        )
        .await;

    // Set cookie and redirect
    let cookie = state.ath.session_cookie(&token);
    let mut resp = Redirect::to("/apps").into_response();
    resp.headers_mut().insert(
        axum::http::header::SET_COOKIE,
        cookie.parse().unwrap(),
    );
    resp
}

async fn render_signup_error(
    state: &AppState,
    session: &Session,
    token: &str,
    invitation: &allowthem_core::Invitation,
    error: &str,
) -> Response {
    let csrf_token = ensure_csrf_token(session).await;
    let email = invitation.email.as_ref().map(|e| e.as_str()).unwrap_or("");
    render_template(
        state,
        "signup.html",
        minijinja::context! {
            csrf_token => csrf_token,
            token => token,
            email => email,
            error => error,
        },
    )
    .into_response()
}

fn render_error_page(state: &AppState, message: &str) -> Response {
    let html = crate::routes::render_error(state, 400, message, false);
    (axum::http::StatusCode::BAD_REQUEST, Html(html)).into_response()
}

fn render_template(state: &AppState, template: &str, ctx: minijinja::Value) -> Html<String> {
    let Ok(env) = state.templates.acquire_env() else {
        return Html("<h1>500</h1><p>Template error</p>".to_string());
    };
    match env.get_template(template) {
        Ok(tmpl) => Html(tmpl.render(ctx).unwrap_or_else(|e| {
            format!("<h1>500</h1><p>Render error: {e}</p>")
        })),
        Err(e) => Html(format!("<h1>500</h1><p>Template not found: {e}</p>")),
    }
}

pub fn client_ip(headers: &axum::http::HeaderMap, trust_proxy: bool) -> String {
    if trust_proxy {
        if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            if let Some(first) = xff.split(',').next() {
                return first.trim().to_string();
            }
        }
    }
    "direct".to_string()
}
