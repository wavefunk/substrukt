use std::sync::atomic::Ordering;

use axum::{
    Extension, Form, Router,
    extract::State,
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Redirect},
    routing::get,
};
use axum_htmx::HxRequest;
use tower_sessions::Session;

use crate::auth;
use crate::backup;
use crate::routes::error_response_with_nav;
use crate::state::AppState;
use crate::templates::base_for_htmx;

/// Extract username string from allowthem User.
fn username_str(user: &allowthem_core::User) -> String {
    user.username
        .as_ref()
        .map(|u| u.as_str().to_string())
        .unwrap_or_default()
}

fn users_page_response(is_htmx: bool, html: String) -> axum::response::Response {
    if is_htmx {
        return (
            [(
                HeaderName::from_static("hx-push-url"),
                HeaderValue::from_static("/settings/users"),
            )],
            Html(html),
        )
            .into_response();
    }
    Html(html).into_response()
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/users", get(users_page))
        .route("/users/invite", axum::routing::post(invite_user))
        .route("/users/{id}/role", axum::routing::post(change_user_role))
        .route("/users/{id}/delete", axum::routing::post(delete_user))
        .route(
            "/users/invitations/{id}/delete",
            axum::routing::post(delete_invitation),
        )
        .route("/profile", get(profile_page).post(change_password))
        .route("/audit-log", get(audit_log_page))
        .route("/backups", get(backups_page).post(update_backup_config))
        .route("/backups/trigger", axum::routing::post(trigger_backup))
}

async fn users_page(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
) -> axum::response::Result<axum::response::Response> {
    if !auth::has_min_role(&role.0, "admin") {
        let csrf_token = auth::ensure_csrf_token(&session).await;
        let ath_csrf = auth::ath_csrf(&session).await;
        return Ok(error_response_with_nav(
            &state,
            StatusCode::FORBIDDEN,
            "Insufficient permissions",
            is_htmx,
            &role.0,
            &username_str(&user),
            &csrf_token,
            &ath_csrf,
        ));
    }

    let invitations = state
        .ath
        .db()
        .list_pending_invitations()
        .await
        .unwrap_or_default();
    let users = state.ath.db().list_users().await.unwrap_or_default();

    let csrf_token = auth::ensure_csrf_token(&session).await;
    let ath_csrf = auth::ath_csrf(&session).await;
    let flash = auth::take_flash(&session).await;
    let echo = auth::flash_echo_trigger(&flash);

    let inv_data: Vec<minijinja::Value> = invitations
        .iter()
        .map(|i| {
            minijinja::context! {
                id => i.id.to_string(),
                email => i.email.as_ref().map(|e: &allowthem_core::Email| e.as_str().to_string()).unwrap_or_default(),
                role => i.metadata.as_deref().unwrap_or("editor"),
                created_at => i.created_at.to_string(),
                expires_at => i.expires_at.to_string(),
            }
        })
        .collect();

    let mut user_data: Vec<minijinja::Value> = Vec::new();
    for u in &users {
        let u_role = crate::auth::resolve_user_role(&state, &u.id).await;
        user_data.push(minijinja::context! {
            id => u.id.to_string(),
            username => u.username.as_ref().map(|un| un.as_str().to_string()).unwrap_or_else(|| u.email.as_str().to_string()),
            role => u_role,
            created_at => u.created_at.to_string(),
        });
    }

    let user_role = &role.0;
    let current_username = username_str(&user);
    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("settings/users.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            ath_csrf => ath_csrf,
            user_role => user_role,
            current_username => current_username,
            invitations => inv_data,
            users => user_data,
            flash_kind => flash.as_ref().map(|(k, _)| k.as_str()),
            flash_message => flash.as_ref().map(|(_, m)| m.as_str()),
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok((echo, Html(html)).into_response())
}

#[derive(serde::Deserialize)]
pub struct InviteForm {
    email: String,
    role: String,
}

async fn invite_user(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    headers: HeaderMap,
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<InviteForm>,
) -> axum::response::Result<axum::response::Response> {
    if !auth::has_min_role(&role.0, "admin") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }
    let user_id_str = user.id.to_string();
    let user_role = role.0.clone();
    let current_username = username_str(&user);

    // Basic email validation
    if !form.email.contains('@') || form.email.len() < 3 {
        return render_users_with_error(
            &state,
            &session,
            is_htmx,
            "Invalid email address",
            &user_role,
            &current_username,
        )
        .await;
    }

    // Check if email already has an account
    let email = match allowthem_core::Email::new(form.email.clone()) {
        Ok(e) => e,
        Err(_) => {
            return render_users_with_error(
                &state,
                &session,
                is_htmx,
                "Invalid email address",
                &user_role,
                &current_username,
            )
            .await;
        }
    };
    if state.ath.db().get_user_by_email(&email).await.is_ok() {
        return render_users_with_error(
            &state,
            &session,
            is_htmx,
            "A user with this email already exists",
            &user_role,
            &current_username,
        )
        .await;
    }

    // Check if there's already a pending invitation for this email
    if let Ok(pending) = state.ath.db().list_pending_invitations().await {
        if pending
            .iter()
            .any(|inv| inv.email.as_ref().map(|e| e.as_str()) == Some(form.email.trim()))
        {
            return render_users_with_error(
                &state,
                &session,
                is_htmx,
                "An invitation for this email already exists",
                &user_role,
                &current_username,
            )
            .await;
        }
    }

    // Validate role
    let role_str = match form.role.as_str() {
        "admin" | "editor" | "viewer" => &form.role,
        _ => {
            return render_users_with_error(
                &state,
                &session,
                is_htmx,
                "Invalid role",
                &user_role,
                &current_username,
            )
            .await;
        }
    };

    // Create invitation via allowthem
    let expires_at = chrono::Utc::now() + chrono::Duration::days(7);
    let (raw_token, invitation) = match state
        .ath
        .db()
        .create_invitation(Some(&email), Some(role_str), Some(user.id), expires_at)
        .await
    {
        Ok(inv) => inv,
        Err(e) => {
            return render_users_with_error(
                &state,
                &session,
                is_htmx,
                &format!("Failed to create invitation: {e}"),
                &user_role,
                &current_username,
            )
            .await;
        }
    };

    state.audit.log(
        &user_id_str,
        "invite_create",
        "invitation",
        &invitation.id.to_string(),
        Some(&serde_json::json!({"email": form.email}).to_string()),
    );

    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");
    let scheme = if state.config.secure_cookies {
        "https"
    } else {
        "http"
    };
    let invite_url = format!("{scheme}://{host}/register?token={raw_token}");

    // Re-fetch lists for display
    let invitations = state
        .ath
        .db()
        .list_pending_invitations()
        .await
        .unwrap_or_default();
    let users = state.ath.db().list_users().await.unwrap_or_default();

    let inv_data: Vec<minijinja::Value> = invitations
        .iter()
        .map(|i| {
            minijinja::context! {
                id => i.id.to_string(),
                email => i.email.as_ref().map(|e: &allowthem_core::Email| e.as_str().to_string()).unwrap_or_default(),
                role => i.metadata.as_deref().unwrap_or("editor"),
                created_at => i.created_at.to_string(),
                expires_at => i.expires_at.to_string(),
            }
        })
        .collect();

    let mut user_data_list: Vec<minijinja::Value> = Vec::new();
    for u in &users {
        let u_role = crate::auth::resolve_user_role(&state, &u.id).await;
        user_data_list.push(minijinja::context! {
            id => u.id.to_string(),
            username => u.username.as_ref().map(|un| un.as_str().to_string()).unwrap_or_else(|| u.email.as_str().to_string()),
            role => u_role,
            created_at => u.created_at.to_string(),
        });
    }

    let csrf_token = auth::ensure_csrf_token(&session).await;
    let ath_csrf = auth::ath_csrf(&session).await;
    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("settings/users.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            ath_csrf => ath_csrf,
            user_role => user_role,
            current_username => current_username,
            invitations => inv_data,
            users => user_data_list,
            invite_url => invite_url,
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(users_page_response(is_htmx, html))
}

async fn render_users_with_error(
    state: &AppState,
    session: &Session,
    is_htmx: bool,
    error: &str,
    user_role: &str,
    current_username: &str,
) -> axum::response::Result<axum::response::Response> {
    let invitations = state
        .ath
        .db()
        .list_pending_invitations()
        .await
        .unwrap_or_default();
    let users = state.ath.db().list_users().await.unwrap_or_default();

    let inv_data: Vec<minijinja::Value> = invitations
        .iter()
        .map(|i| {
            minijinja::context! {
                id => i.id.to_string(),
                email => i.email.as_ref().map(|e: &allowthem_core::Email| e.as_str().to_string()).unwrap_or_default(),
                role => i.metadata.as_deref().unwrap_or("editor"),
                created_at => i.created_at.to_string(),
                expires_at => i.expires_at.to_string(),
            }
        })
        .collect();

    let mut user_data: Vec<minijinja::Value> = Vec::new();
    for u in &users {
        let u_role = crate::auth::resolve_user_role(state, &u.id).await;
        user_data.push(minijinja::context! {
            id => u.id.to_string(),
            username => u.username.as_ref().map(|un| un.as_str().to_string()).unwrap_or_else(|| u.email.as_str().to_string()),
            role => u_role,
            created_at => u.created_at.to_string(),
        });
    }

    let csrf_token = auth::ensure_csrf_token(session).await;
    let ath_csrf = auth::ath_csrf(session).await;
    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("settings/users.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            ath_csrf => ath_csrf,
            user_role => user_role,
            current_username => current_username,
            invitations => inv_data,
            users => user_data,
            error => error,
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(users_page_response(is_htmx, html))
}

async fn profile_page(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
) -> axum::response::Result<axum::response::Response> {
    let csrf_token = auth::ensure_csrf_token(&session).await;
    let ath_csrf = auth::ath_csrf(&session).await;
    let flash = auth::take_flash(&session).await;
    let echo = auth::flash_echo_trigger(&flash);
    let (flash_kind, flash_message) = flash.unwrap_or_default();
    let user_role = &role.0;
    let current_username = username_str(&user);

    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("settings/profile.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            ath_csrf => ath_csrf,
            user_role => user_role,
            current_username => current_username,
            username => current_username,
            flash_kind => flash_kind,
            flash_message => flash_message,
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok((echo, Html(html)).into_response())
}

#[derive(serde::Deserialize)]
struct ChangePasswordForm {
    current_password: String,
    new_password: String,
    confirm_password: String,
}

async fn change_password(
    Extension(user): Extension<allowthem_core::User>,
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<ChangePasswordForm>,
) -> axum::response::Result<axum::response::Redirect> {
    // Validate new password length
    if form.new_password.len() < 8 {
        auth::set_flash(
            &session,
            "error",
            "New password must be at least 8 characters",
        )
        .await;
        return Ok(axum::response::Redirect::to("/settings/profile"));
    }

    // Confirm passwords match
    if form.new_password != form.confirm_password {
        auth::set_flash(&session, "error", "New passwords do not match").await;
        return Ok(axum::response::Redirect::to("/settings/profile"));
    }

    // Fetch the user with password hash (the Extension user has None for security)
    let username_str = user.username.as_ref().map(|u| u.as_str()).unwrap_or("");
    let login_user = match state.ath.db().find_for_login(username_str).await {
        Ok(u) => u,
        Err(_) => {
            auth::set_flash(&session, "error", "Current password is incorrect").await;
            return Ok(axum::response::Redirect::to("/settings/profile"));
        }
    };

    // Verify current password
    let hash = match &login_user.password_hash {
        Some(h) => h,
        None => {
            auth::set_flash(&session, "error", "Current password is incorrect").await;
            return Ok(axum::response::Redirect::to("/settings/profile"));
        }
    };

    match allowthem_core::password::verify_password(&form.current_password, hash) {
        Ok(true) => {}
        _ => {
            auth::set_flash(&session, "error", "Current password is incorrect").await;
            return Ok(axum::response::Redirect::to("/settings/profile"));
        }
    }

    // Update password via allowthem
    if let Err(e) = state
        .ath
        .db()
        .update_user_password(user.id, &form.new_password)
        .await
    {
        tracing::error!("Password update failed: {e}");
        auth::set_flash(
            &session,
            "error",
            &format!("Failed to update password: {e}"),
        )
        .await;
        return Ok(axum::response::Redirect::to("/settings/profile"));
    }

    let user_id_str = user.id.to_string();
    state
        .audit
        .log(&user_id_str, "password_changed", "user", &user_id_str, None);

    auth::set_flash(&session, "success", "Password updated successfully").await;
    Ok(axum::response::Redirect::to("/settings/profile"))
}

async fn delete_invitation(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> axum::response::Result<axum::response::Response> {
    if !auth::has_min_role(&role.0, "admin") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }
    let user_id_str = user.id.to_string();

    if let Ok(uuid) = uuid::Uuid::parse_str(&id) {
        let inv_id = allowthem_core::InvitationId::from_uuid(uuid);
        let _ = state.ath.db().delete_invitation(inv_id).await;
    }
    state
        .audit
        .log(&user_id_str, "invite_delete", "invitation", &id, None);
    auth::set_flash(&session, "success", "Invitation revoked").await;
    Ok(Redirect::to("/settings/users").into_response())
}

#[derive(serde::Deserialize)]
struct ChangeRoleForm {
    role: String,
}

async fn change_user_role(
    Extension(current_user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    axum::extract::Path(id): axum::extract::Path<String>,
    Form(form): Form<ChangeRoleForm>,
) -> axum::response::Result<axum::response::Response> {
    if !auth::has_min_role(&role.0, "admin") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }
    let current_user_id_str = current_user.id.to_string();

    let target_user_id = match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => allowthem_core::UserId::from_uuid(uuid),
        Err(_) => {
            auth::set_flash(&session, "error", "Invalid user ID").await;
            return Ok(Redirect::to("/settings/users").into_response());
        }
    };

    // Prevent self-demotion
    if target_user_id == current_user.id {
        auth::set_flash(&session, "error", "Cannot change your own role").await;
        return Ok(Redirect::to("/settings/users").into_response());
    }

    // Validate role
    let new_role = match form.role.as_str() {
        "admin" | "editor" | "viewer" => &form.role,
        _ => {
            auth::set_flash(&session, "error", "Invalid role").await;
            return Ok(Redirect::to("/settings/users").into_response());
        }
    };

    // Remove all existing roles
    let current_roles = state
        .ath
        .db()
        .get_user_roles(&target_user_id)
        .await
        .unwrap_or_default();
    for r in &current_roles {
        let _ = state.ath.db().unassign_role(&target_user_id, &r.id).await;
    }

    // Assign new role
    let role_name = allowthem_core::RoleName::new(new_role);
    if let Ok(Some(r)) = state.ath.db().get_role_by_name(&role_name).await {
        let _ = state.ath.db().assign_role(&target_user_id, &r.id).await;
    }

    state.audit.log(
        &current_user_id_str,
        "user_role_changed",
        "user",
        &id,
        Some(&serde_json::json!({"new_role": new_role}).to_string()),
    );
    auth::set_flash(&session, "success", "User role updated").await;
    Ok(Redirect::to("/settings/users").into_response())
}

async fn delete_user(
    Extension(current_user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> axum::response::Result<axum::response::Response> {
    if !auth::has_min_role(&role.0, "admin") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }
    let current_user_id_str = current_user.id.to_string();

    let target_user_id = match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => allowthem_core::UserId::from_uuid(uuid),
        Err(_) => {
            auth::set_flash(&session, "error", "Invalid user ID").await;
            return Ok(Redirect::to("/settings/users").into_response());
        }
    };

    // Prevent self-deletion
    if target_user_id == current_user.id {
        auth::set_flash(&session, "error", "Cannot delete your own account").await;
        return Ok(Redirect::to("/settings/users").into_response());
    }

    // Delete sessions first, then user
    let _ = state.ath.db().delete_user_sessions(&target_user_id).await;
    if let Err(e) = state.ath.db().delete_user(target_user_id).await {
        auth::set_flash(&session, "error", &format!("Failed to delete user: {e}")).await;
        return Ok(Redirect::to("/settings/users").into_response());
    }

    state
        .audit
        .log(&current_user_id_str, "user_deleted", "user", &id, None);
    auth::set_flash(&session, "success", "User deleted").await;
    Ok(Redirect::to("/settings/users").into_response())
}

#[derive(serde::Deserialize, Default)]
pub struct AuditLogFilter {
    #[serde(default)]
    action: String,
    #[serde(default)]
    actor: String,
    #[serde(default)]
    date_from: String,
    #[serde(default)]
    date_to: String,
    #[serde(default)]
    page: String,
}

async fn audit_log_page(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    axum::extract::Query(filter): axum::extract::Query<AuditLogFilter>,
) -> axum::response::Result<Html<String>> {
    if !auth::has_min_role(&role.0, "admin") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }

    let page: u32 = filter.page.parse().unwrap_or(1).max(1);

    let action_filter = if filter.action.is_empty() {
        None
    } else {
        Some(filter.action.as_str())
    };
    let actor_filter = if filter.actor.is_empty() {
        None
    } else {
        Some(filter.actor.as_str())
    };
    let date_from = if filter.date_from.is_empty() {
        None
    } else {
        Some(filter.date_from.as_str())
    };
    let date_to = if filter.date_to.is_empty() {
        None
    } else {
        Some(filter.date_to.as_str())
    };

    let (entries, has_next) = state
        .audit
        .list_audit_log(action_filter, actor_filter, None, date_from, date_to, page)
        .await
        .map_err(|e| format!("DB error: {e}"))?;
    let page_start = if entries.is_empty() {
        0
    } else {
        (page as usize - 1) * 100 + 1
    };
    let page_end = if entries.is_empty() {
        0
    } else {
        page_start + entries.len() - 1
    };

    let actors = state
        .audit
        .list_audit_actors()
        .await
        .map_err(|e| format!("DB error: {e}"))?;
    let actions = state
        .audit
        .list_audit_actions()
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    // Build a map from user ID (string) -> username for display
    let all_users = state.ath.db().list_users().await.unwrap_or_default();
    let username_map: std::collections::HashMap<String, String> = all_users
        .iter()
        .map(|u| {
            let uid = u.id.to_string();
            let uname = u
                .username
                .as_ref()
                .map(|un| un.as_str().to_string())
                .unwrap_or_else(|| u.email.as_str().to_string());
            (uid, uname)
        })
        .collect();

    let resolve_actor = |actor: &str| -> String {
        username_map
            .get(actor)
            .cloned()
            .unwrap_or_else(|| actor.to_string())
    };

    let entry_data: Vec<minijinja::Value> = entries
        .iter()
        .map(|e| {
            minijinja::context! {
                id => e.id,
                timestamp => e.timestamp,
                actor => resolve_actor(&e.actor),
                action => e.action,
                resource_type => e.resource_type,
                resource_id => e.resource_id,
                details => e.details,
            }
        })
        .collect();

    // Build actor filter options with display names, keeping raw IDs as values
    let actor_options: Vec<minijinja::Value> = actors
        .iter()
        .map(|a| {
            minijinja::context! {
                value => a,
                label => resolve_actor(a),
            }
        })
        .collect();

    let mut pagination_params = Vec::new();
    if !filter.action.is_empty() {
        pagination_params.push(format!("action={}", filter.action));
    }
    if !filter.actor.is_empty() {
        pagination_params.push(format!("actor={}", filter.actor));
    }
    if !filter.date_from.is_empty() {
        pagination_params.push(format!("date_from={}", filter.date_from));
    }
    if !filter.date_to.is_empty() {
        pagination_params.push(format!("date_to={}", filter.date_to));
    }
    let pagination_qs = if pagination_params.is_empty() {
        String::new()
    } else {
        format!("{}&", pagination_params.join("&"))
    };

    let csrf_token = auth::ensure_csrf_token(&session).await;
    let ath_csrf = auth::ath_csrf(&session).await;
    let user_role = &role.0;
    let current_username = username_str(&user);
    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("settings/audit_log.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            ath_csrf => ath_csrf,
            user_role => user_role,
            current_username => current_username,
            entries => entry_data,
            actions => actions,
            actors => actor_options,
            filter_action => filter.action,
            filter_actor => filter.actor,
            filter_date_from => filter.date_from,
            filter_date_to => filter.date_to,
            pagination_qs => pagination_qs,
            page => page,
            page_start => page_start,
            page_end => page_end,
            has_next => has_next,
            has_prev => page > 1,
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html))
}

// -- Backups --

async fn backups_page(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
) -> axum::response::Result<axum::response::Response> {
    if !auth::has_min_role(&role.0, "admin") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }

    let config = state
        .audit
        .get_backup_config()
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    let latest_backup = state
        .audit
        .latest_backup()
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    let history = state
        .audit
        .list_backup_history(10)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    let s3_configured = state.s3_config.is_some();
    let backup_running = state.backup_running.load(Ordering::SeqCst);

    // Next backup info
    let next_backup_info = if config.enabled && s3_configured {
        let last_success = state.audit.last_successful_backup().await.ok().flatten();
        let delay =
            backup::calculate_next_backup_delay(last_success.as_ref(), config.frequency_hours);
        if delay.is_zero() {
            "Imminent".to_string()
        } else {
            let hours = delay.as_secs() / 3600;
            let mins = (delay.as_secs() % 3600) / 60;
            if hours > 0 {
                format!("In {} hours {} minutes", hours, mins)
            } else {
                format!("In {} minutes", mins)
            }
        }
    } else {
        String::new()
    };

    // Credential status
    let credential_status: Vec<minijinja::Value> = [
        (
            "SUBSTRUKT_S3_ENDPOINT",
            std::env::var("SUBSTRUKT_S3_ENDPOINT").is_ok(),
        ),
        (
            "SUBSTRUKT_S3_BUCKET",
            std::env::var("SUBSTRUKT_S3_BUCKET").is_ok(),
        ),
        (
            "SUBSTRUKT_S3_ACCESS_KEY",
            std::env::var("SUBSTRUKT_S3_ACCESS_KEY").is_ok(),
        ),
        (
            "SUBSTRUKT_S3_SECRET_KEY",
            std::env::var("SUBSTRUKT_S3_SECRET_KEY").is_ok(),
        ),
        (
            "SUBSTRUKT_S3_REGION",
            std::env::var("SUBSTRUKT_S3_REGION").is_ok(),
        ),
        (
            "SUBSTRUKT_S3_PATH_STYLE",
            std::env::var("SUBSTRUKT_S3_PATH_STYLE").is_ok(),
        ),
    ]
    .iter()
    .map(|(name, present)| {
        minijinja::context! {
            name => *name,
            present => *present,
        }
    })
    .collect();

    let flash = auth::take_flash(&session).await;
    let echo = auth::flash_echo_trigger(&flash);
    let (flash_kind, flash_message) = flash.unwrap_or_default();

    let csrf_token = auth::ensure_csrf_token(&session).await;
    let ath_csrf = auth::ath_csrf(&session).await;
    let user_role = &role.0;
    let current_username = username_str(&user);

    let latest_ctx = latest_backup.as_ref().map(|b| {
        minijinja::context! {
            status => b.status,
            started_at => b.started_at,
            error_message => b.error_message,
            size_bytes => b.size_bytes,
        }
    });

    let history_ctx: Vec<minijinja::Value> = history
        .iter()
        .map(|b| {
            minijinja::context! {
                started_at => b.started_at,
                status => b.status,
                trigger_source => b.trigger_source,
                size_bytes => b.size_bytes,
                s3_key => b.s3_key,
                error_message => b.error_message,
            }
        })
        .collect();

    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("settings/backups.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            ath_csrf => ath_csrf,
            user_role => user_role,
            current_username => current_username,
            config => minijinja::context! {
                frequency_hours => config.frequency_hours,
                retention_count => config.retention_count,
                enabled => config.enabled,
            },
            latest_backup => latest_ctx,
            history => history_ctx,
            s3_configured => s3_configured,
            backup_running => backup_running,
            next_backup_info => next_backup_info,
            credential_status => credential_status,
            flash_kind => flash_kind,
            flash_message => flash_message,
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok((echo, Html(html)).into_response())
}

#[derive(serde::Deserialize)]
struct BackupConfigForm {
    frequency_hours: i64,
    retention_count: i64,
    enabled: Option<String>,
}

async fn update_backup_config(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<BackupConfigForm>,
) -> axum::response::Result<Redirect> {
    if !auth::has_min_role(&role.0, "admin") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }
    let user_id_str = user.id.to_string();

    // Validate
    let valid_frequencies = [1, 6, 12, 24, 48, 168];
    if !valid_frequencies.contains(&form.frequency_hours) {
        auth::set_flash(&session, "error", "Invalid frequency").await;
        return Ok(Redirect::to("/settings/backups"));
    }
    if form.retention_count < 1 || form.retention_count > 100 {
        auth::set_flash(&session, "error", "Retention count must be 1-100").await;
        return Ok(Redirect::to("/settings/backups"));
    }

    let enabled = form.enabled.is_some();

    state
        .audit
        .update_backup_config(form.frequency_hours, form.retention_count, enabled)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    state.audit.log(
        &user_id_str,
        "backup_config_changed",
        "backup_config",
        "1",
        Some(
            &serde_json::json!({
                "frequency_hours": form.frequency_hours,
                "retention_count": form.retention_count,
                "enabled": enabled,
            })
            .to_string(),
        ),
    );

    auth::set_flash(&session, "success", "Backup configuration updated").await;
    Ok(Redirect::to("/settings/backups"))
}

async fn trigger_backup(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
) -> axum::response::Result<Redirect> {
    if !auth::has_min_role(&role.0, "admin") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }
    let user_id_str = user.id.to_string();

    if state.s3_config.is_none() {
        auth::set_flash(&session, "error", "S3 not configured").await;
        return Ok(Redirect::to("/settings/backups"));
    }

    if state.backup_running.load(Ordering::SeqCst) {
        auth::set_flash(&session, "error", "Backup already in progress").await;
        return Ok(Redirect::to("/settings/backups"));
    }

    if let Some(tx) = &state.backup_trigger {
        if tx.try_send(()).is_err() {
            auth::set_flash(&session, "error", "Backup trigger channel full").await;
            return Ok(Redirect::to("/settings/backups"));
        }
    } else {
        auth::set_flash(&session, "error", "Backup not available").await;
        return Ok(Redirect::to("/settings/backups"));
    }

    state.audit.log(
        &user_id_str,
        "backup_triggered",
        "backup",
        "",
        Some(&serde_json::json!({"trigger": "manual"}).to_string()),
    );

    auth::set_flash(&session, "success", "Backup triggered").await;
    Ok(Redirect::to("/settings/backups"))
}
