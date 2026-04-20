use axum::{
    Form,
    extract::State,
    response::{Html, IntoResponse, Redirect, Response},
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
        .route(
            "/forgot-password",
            get(forgot_password_page).post(forgot_password_submit),
        )
        .route(
            "/reset-password",
            get(reset_password_page).post(reset_password_submit),
        )
        .route("/verify-email", get(verify_email))
        .route("/verify-pending", get(verify_pending_page))
        .route("/verify-resend", post(verify_resend))
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

    // Hard-block unverified users. Do not create a session.
    if !user.email_verified {
        let csrf_token = ensure_csrf_token(&session).await;
        return render_template(
            &state,
            "login.html",
            minijinja::context! {
                csrf_token => csrf_token,
                username => form.username,
                error => "Please verify your email address before logging in.",
                show_resend => true,
            },
        )
        .into_response();
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
    resp.headers_mut()
        .insert(axum::http::header::SET_COOKIE, cookie.parse().unwrap());
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
    if state.has_users.load(std::sync::atomic::Ordering::Relaxed) {
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
    if state.has_users.load(std::sync::atomic::Ordering::Relaxed) {
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
        .create_user(email, &form.password, Some(username), None)
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

    // Bootstrap admin is auto-verified — no external mail bounce to check.
    auto_verify(&state, user.id).await;

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
    resp.headers_mut()
        .insert(axum::http::header::SET_COOKIE, cookie.parse().unwrap());
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
        return render_signup_error(
            &state,
            &session,
            &form.token,
            &invitation,
            "Username is required.",
        )
        .await;
    }
    if form.password.len() < 8 {
        return render_signup_error(
            &state,
            &session,
            &form.token,
            &invitation,
            "Password must be at least 8 characters.",
        )
        .await;
    }
    if form.password != form.confirm_password {
        return render_signup_error(
            &state,
            &session,
            &form.token,
            &invitation,
            "Passwords do not match.",
        )
        .await;
    }

    // Check username uniqueness
    let username = allowthem_core::Username::new(form.username.clone());
    if state.ath.db().get_user_by_username(&username).await.is_ok() {
        return render_signup_error(
            &state,
            &session,
            &form.token,
            &invitation,
            "Username is already taken.",
        )
        .await;
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
    let user = match state
        .ath
        .db()
        .create_user(email, &form.password, Some(username), None)
        .await
    {
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

    // Invitation flow proves email ownership (user received + clicked the link).
    auto_verify(&state, user.id).await;

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

    state
        .has_users
        .store(true, std::sync::atomic::Ordering::Relaxed);

    // Create session
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

    // Set cookie and redirect
    let cookie = state.ath.session_cookie(&token);
    let mut resp = Redirect::to("/apps").into_response();
    resp.headers_mut()
        .insert(axum::http::header::SET_COOKIE, cookie.parse().unwrap());
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

// ── Password reset ───────────────────────────────────────────

#[derive(serde::Deserialize)]
struct ForgotPasswordForm {
    email: String,
}

#[derive(serde::Deserialize)]
struct ResetTokenQuery {
    token: Option<String>,
}

#[derive(serde::Deserialize)]
struct ResetPasswordForm {
    token: String,
    password: String,
    confirm_password: String,
}

async fn forgot_password_page(
    session: Session,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let csrf_token = ensure_csrf_token(&session).await;
    render_template(
        &state,
        "forgot_password.html",
        minijinja::context! { csrf_token => csrf_token },
    )
}

async fn forgot_password_submit(
    session: Session,
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Form(form): Form<ForgotPasswordForm>,
) -> Response {
    // Rate-limit to deter enumeration and abuse.
    let ip = client_ip(&headers, state.config.trust_proxy_headers);
    if !state.login_limiter.check(&ip) {
        let csrf_token = ensure_csrf_token(&session).await;
        return render_template(
            &state,
            "forgot_password.html",
            minijinja::context! {
                csrf_token => csrf_token,
                error => "Too many attempts. Please try again later.",
            },
        )
        .into_response();
    }

    // Always show the same "check your email" page — do not reveal whether the
    // address has an account.
    if let Ok(email) = allowthem_core::Email::new(form.email.trim().to_string()) {
        match state.ath.db().create_password_reset(&email).await {
            Ok(Some(raw_token)) => {
                let host = headers
                    .get("host")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("localhost");
                let scheme = if state.config.secure_cookies {
                    "https"
                } else {
                    "http"
                };
                let reset_url = format!("{scheme}://{host}/reset-password?token={raw_token}");
                let body = format!(
                    "You requested a password reset for your Substrukt account.\n\n\
                     Click the link below to set a new password:\n\n{reset_url}\n\n\
                     This link expires in 30 minutes. If you did not request this, ignore this email."
                );
                let html = format!(
                    "<p>You requested a password reset for your Substrukt account.</p>\
                     <p><a href=\"{reset_url}\">Click here to set a new password</a>.</p>\
                     <p>This link expires in 30 minutes. If you did not request this, ignore this email.</p>"
                );
                let message = allowthem_core::EmailMessage {
                    to: email.as_str(),
                    subject: "Reset your Substrukt password",
                    body: &body,
                    html: Some(&html),
                };
                if let Err(e) = state.email_sender.send(message).await {
                    tracing::error!("password reset email failed: {e}");
                }
            }
            Ok(None) => {
                // No such user — stay silent.
            }
            Err(e) => {
                tracing::error!("create_password_reset failed: {e}");
            }
        }
    }

    render_template(
        &state,
        "forgot_password.html",
        minijinja::context! { sent => true },
    )
    .into_response()
}

async fn reset_password_page(
    session: Session,
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<ResetTokenQuery>,
) -> Response {
    let raw_token = match query.token {
        Some(t) if !t.is_empty() => t,
        _ => return render_error_page(&state, "Invalid or missing reset token."),
    };

    match state.ath.db().validate_reset_token(&raw_token).await {
        Ok(Some(_)) => {}
        _ => {
            return render_error_page(
                &state,
                "This password reset link is invalid or has expired.",
            );
        }
    }

    let csrf_token = ensure_csrf_token(&session).await;
    render_template(
        &state,
        "reset_password.html",
        minijinja::context! {
            csrf_token => csrf_token,
            token => raw_token,
        },
    )
    .into_response()
}

async fn reset_password_submit(
    session: Session,
    State(state): State<AppState>,
    Form(form): Form<ResetPasswordForm>,
) -> Response {
    let validation_error: Option<&'static str> = if form.password.len() < 8 {
        Some("Password must be at least 8 characters.")
    } else if form.password != form.confirm_password {
        Some("Passwords do not match.")
    } else {
        None
    };

    if let Some(msg) = validation_error {
        let csrf_token = ensure_csrf_token(&session).await;
        return render_template(
            &state,
            "reset_password.html",
            minijinja::context! {
                csrf_token => csrf_token,
                token => form.token,
                error => msg,
            },
        )
        .into_response();
    }

    match state
        .ath
        .db()
        .execute_reset(&form.token, &form.password)
        .await
    {
        Ok(true) => {
            let _ = state
                .ath
                .db()
                .log_audit(
                    allowthem_core::AuditEvent::PasswordReset,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .await;
            Redirect::to("/login").into_response()
        }
        Ok(false) => render_error_page(
            &state,
            "This password reset link is invalid or has expired.",
        ),
        Err(e) => {
            tracing::error!("execute_reset failed: {e}");
            render_error_page(&state, "Failed to reset password. Please try again.")
        }
    }
}

// ── Email verification ───────────────────────────────────────

#[derive(serde::Deserialize)]
struct VerifyEmailQuery {
    token: Option<String>,
}

#[derive(serde::Deserialize)]
struct VerifyResendForm {
    email: String,
}

/// Mint a verification token and immediately consume it. Used to auto-verify
/// users whose email ownership is already established by another means
/// (bootstrap admin, invitation flow).
async fn auto_verify(state: &AppState, user_id: allowthem_core::UserId) {
    let raw = match state.ath.db().create_email_verification(user_id).await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("auto_verify create failed: {e}");
            return;
        }
    };
    if let Err(e) = state.ath.db().verify_email(&raw).await {
        tracing::error!("auto_verify consume failed: {e}");
    }
}

async fn verify_email(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<VerifyEmailQuery>,
) -> Response {
    let raw_token = match query.token {
        Some(t) if !t.is_empty() => t,
        _ => {
            return render_template(
                &state,
                "verify_result.html",
                minijinja::context! {
                    success => false,
                    message => "Invalid or missing verification token.",
                },
            )
            .into_response();
        }
    };

    match state.ath.db().verify_email(&raw_token).await {
        Ok(true) => render_template(
            &state,
            "verify_result.html",
            minijinja::context! {
                success => true,
                message => "Your email has been verified. You can now log in.",
            },
        )
        .into_response(),
        _ => render_template(
            &state,
            "verify_result.html",
            minijinja::context! {
                success => false,
                message => "This verification link is invalid or has expired.",
            },
        )
        .into_response(),
    }
}

async fn verify_pending_page(session: Session, State(state): State<AppState>) -> impl IntoResponse {
    let csrf_token = ensure_csrf_token(&session).await;
    render_template(
        &state,
        "verify_pending.html",
        minijinja::context! { csrf_token => csrf_token },
    )
}

async fn verify_resend(
    session: Session,
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Form(form): Form<VerifyResendForm>,
) -> Response {
    let ip = client_ip(&headers, state.config.trust_proxy_headers);
    if !state.login_limiter.check(&ip) {
        let csrf_token = ensure_csrf_token(&session).await;
        return render_template(
            &state,
            "verify_pending.html",
            minijinja::context! {
                csrf_token => csrf_token,
                error => "Too many attempts. Please try again later.",
            },
        )
        .into_response();
    }

    // Silent on unknown / already-verified to avoid enumeration.
    if let Ok(email) = allowthem_core::Email::new(form.email.trim().to_string())
        && let Ok(user) = state.ath.db().get_user_by_email(&email).await
        && !user.email_verified
    {
        match state.ath.db().create_email_verification(user.id).await {
            Ok(raw_token) => {
                let host = headers
                    .get("host")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("localhost");
                let scheme = if state.config.secure_cookies {
                    "https"
                } else {
                    "http"
                };
                let verify_url = format!("{scheme}://{host}/verify-email?token={raw_token}");
                let body = format!(
                    "Please verify your Substrukt email by clicking the link below:\n\n{verify_url}\n\n\
                     This link expires in 24 hours."
                );
                let html = format!(
                    "<p>Please verify your Substrukt email address.</p>\
                     <p><a href=\"{verify_url}\">Click here to verify</a>.</p>\
                     <p>This link expires in 24 hours.</p>"
                );
                let message = allowthem_core::EmailMessage {
                    to: email.as_str(),
                    subject: "Verify your Substrukt email",
                    body: &body,
                    html: Some(&html),
                };
                if let Err(e) = state.email_sender.send(message).await {
                    tracing::error!("verification email failed: {e}");
                }
            }
            Err(e) => tracing::error!("create_email_verification failed: {e}"),
        }
    }

    render_template(
        &state,
        "verify_pending.html",
        minijinja::context! { sent => true },
    )
    .into_response()
}

fn render_template(state: &AppState, template: &str, ctx: minijinja::Value) -> Html<String> {
    let Ok(env) = state.templates.acquire_env() else {
        return Html("<h1>500</h1><p>Template error</p>".to_string());
    };
    match env.get_template(template) {
        Ok(tmpl) => Html(
            tmpl.render(ctx)
                .unwrap_or_else(|e| format!("<h1>500</h1><p>Render error: {e}</p>")),
        ),
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
