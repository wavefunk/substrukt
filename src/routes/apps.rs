use axum::{
    Extension, Form, Router,
    extract::State,
    response::{Html, IntoResponse, Redirect},
    routing::get,
};
use axum_htmx::HxRequest;
use tower_sessions::Session;

use crate::app_context::AppContext;
use crate::auth;
use crate::db::models;
use crate::state::AppState;
use crate::templates::base_for_htmx;

/// Extract username string from allowthem User.
fn username_str(user: &allowthem_core::User) -> String {
    user.username
        .as_ref()
        .map(|u| u.as_str().to_string())
        .unwrap_or_default()
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_apps).post(create_app))
        .route("/new", get(new_app_form))
        .route(
            "/{app_slug}/settings",
            get(app_settings).post(update_app_name),
        )
        .route(
            "/{app_slug}/settings/access",
            axum::routing::post(update_access),
        )
        .route(
            "/{app_slug}/settings/tokens",
            axum::routing::post(create_token),
        )
        .route(
            "/{app_slug}/settings/tokens/{token_id}/delete",
            axum::routing::post(delete_token),
        )
        .route("/{app_slug}/data", get(data_page))
        .route("/{app_slug}/data/import", axum::routing::post(import_data))
        .route("/{app_slug}/data/export", axum::routing::post(export_data))
        .route("/{app_slug}/delete", axum::routing::post(delete_app))
}

async fn list_apps(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
) -> axum::response::Result<axum::response::Response> {
    let user_role = role.0.clone();
    let current_username = username_str(&user);
    let csrf_token = auth::ensure_csrf_token(&session).await;
    let flash = auth::take_flash(&session).await;
    let echo = auth::flash_echo_trigger(&flash);

    let apps = if user_role == "admin" {
        models::list_apps(&state.pool)
            .await
            .map_err(|e| format!("DB error: {e}"))?
    } else {
        models::list_apps_for_user(&state.pool, &user.id.to_string())
            .await
            .map_err(|e| format!("DB error: {e}"))?
    };

    let app_data: Vec<minijinja::Value> = apps
        .iter()
        .map(|a| {
            let schemas_dir = state.config.app_schemas_dir(&a.slug);
            let schema_count = crate::schema::list_schemas(&schemas_dir)
                .map(|s| s.len())
                .unwrap_or(0);
            minijinja::context! {
                id => a.id,
                slug => a.slug,
                name => a.name,
                schema_count => schema_count,
            }
        })
        .collect();

    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("apps/list.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            user_role => user_role,
            current_username => current_username,
            apps => app_data,
            flash_kind => flash.as_ref().map(|(k, _)| k.as_str()),
            flash_message => flash.as_ref().map(|(_, m)| m.as_str()),
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok((echo, Html(html)).into_response())
}

async fn new_app_form(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
) -> axum::response::Result<Html<String>> {
    if !auth::has_min_role(&role.0, "admin") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }
    let csrf_token = auth::ensure_csrf_token(&session).await;
    let user_role = &role.0;
    let current_username = username_str(&user);

    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("apps/new.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            user_role => user_role,
            current_username => current_username,
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html))
}

#[derive(serde::Deserialize)]
struct CreateAppForm {
    name: String,
    slug: String,
}

async fn create_app(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<CreateAppForm>,
) -> axum::response::Result<axum::response::Response> {
    if !auth::has_min_role(&role.0, "admin") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }
    let user_id_str = user.id.to_string();
    let slug = form.slug.trim().to_lowercase();
    let name = form.name.trim();

    if name.is_empty() || slug.is_empty() {
        auth::set_flash(&session, "error", "Name and slug are required").await;
        return Ok(Redirect::to("/apps/new").into_response());
    }

    if let Err(e) = models::validate_app_slug(&slug) {
        auth::set_flash(&session, "error", &e).await;
        return Ok(Redirect::to("/apps/new").into_response());
    }

    match models::create_app(&state.pool, &slug, name).await {
        Ok(app) => {
            if let Err(e) = state.config.ensure_app_dirs(&slug) {
                tracing::error!("Failed to create app dirs: {e}");
            }
            state.audit.log(
                &user_id_str,
                "app_create",
                "app",
                &app.slug,
                Some(&serde_json::json!({"name": app.name}).to_string()),
            );
            auth::set_flash(&session, "success", &format!("App '{}' created", app.name)).await;
            Ok(Redirect::to("/apps").into_response())
        }
        Err(e) => {
            let msg = if e.to_string().contains("UNIQUE") {
                "An app with this slug already exists".to_string()
            } else {
                e.to_string()
            };
            auth::set_flash(&session, "error", &msg).await;
            Ok(Redirect::to("/apps/new").into_response())
        }
    }
}

async fn app_settings(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
) -> axum::response::Result<axum::response::Response> {
    if !auth::has_min_role(&role.0, "admin") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }
    let csrf_token = auth::ensure_csrf_token(&session).await;
    let user_role = &role.0;
    let current_username = username_str(&user);
    let flash = auth::take_flash(&session).await;
    let echo = auth::flash_echo_trigger(&flash);

    // Build user list from allowthem users with app access info
    let all_users = state.ath.db().list_users().await.unwrap_or_default();
    let mut user_data: Vec<minijinja::Value> = Vec::new();
    for u in &all_users {
        let uid_str = u.id.to_string();
        let has_access = models::user_has_app_access(&state.pool, app.app.id, &uid_str)
            .await
            .unwrap_or(false);
        // Resolve role for this user
        let u_role = crate::auth::resolve_user_role(&state, &u.id).await;
        user_data.push(minijinja::context! {
            id => uid_str,
            username => u.username.as_ref().map(|un| un.as_str().to_string()).unwrap_or_else(|| u.email.as_str().to_string()),
            role => u_role,
            has_access => has_access,
        });
    }

    // Build token list from app_tokens + allowthem token info
    // We collect all allowthem tokens, then match them against our app_tokens table
    let token_ids = models::list_app_tokens(&state.pool, app.app.id)
        .await
        .map_err(|e| format!("DB error: {e}"))?;
    // Collect all tokens for all users -- for a settings page this is acceptable
    let all_ath_users = state.ath.db().list_users().await.unwrap_or_default();
    let mut all_tokens: Vec<allowthem_core::ApiTokenInfo> = Vec::new();
    for u in &all_ath_users {
        if let Ok(toks) = state.ath.db().list_api_tokens(u.id).await {
            all_tokens.extend(toks);
        }
    }
    let mut token_data: Vec<minijinja::Value> = Vec::new();
    for tid in &token_ids {
        // Find matching token info
        let info = all_tokens.iter().find(|t| t.id.to_string() == *tid);
        let name = info.map(|t| t.name.clone()).unwrap_or_else(|| tid.clone());
        let created_at = info.map(|t| t.created_at.to_string()).unwrap_or_default();
        token_data.push(minijinja::context! {
            id => tid,
            name => name,
            created_at => created_at,
        });
    }

    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("apps/settings.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            user_role => user_role,
            current_username => current_username,
            app => app.template_context(),
            nav_schemas => app.nav_schemas(&state.config),
            users => user_data,
            tokens => token_data,
            flash_kind => flash.as_ref().map(|(k, _)| k.as_str()),
            flash_message => flash.as_ref().map(|(_, m)| m.as_str()),
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok((echo, Html(html)).into_response())
}

#[derive(serde::Deserialize)]
struct UpdateNameForm {
    name: String,
}

async fn update_app_name(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Form(form): Form<UpdateNameForm>,
) -> axum::response::Result<axum::response::Response> {
    if !auth::has_min_role(&role.0, "admin") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }
    let user_id_str = user.id.to_string();
    let name = form.name.trim();
    if name.is_empty() {
        auth::set_flash(&session, "error", "Name cannot be empty").await;
    } else {
        models::update_app_name(&state.pool, app.app.id, name)
            .await
            .map_err(|e| format!("DB error: {e}"))?;
        state.audit.log(
            &user_id_str,
            "app_updated",
            "app",
            &app.app.slug,
            Some(&serde_json::json!({"name": name}).to_string()),
        );
        auth::set_flash(&session, "success", "App name updated").await;
    }
    Ok(Redirect::to(&format!("/apps/{}/settings", app.app.slug)).into_response())
}

#[derive(serde::Deserialize)]
struct AccessForm {
    #[serde(default)]
    access: Vec<String>,
}

async fn update_access(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Form(form): Form<AccessForm>,
) -> axum::response::Result<axum::response::Response> {
    if !auth::has_min_role(&role.0, "admin") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }
    let user_id_str = user.id.to_string();

    // Get all users from allowthem and check/update access
    let all_users = state.ath.db().list_users().await.unwrap_or_default();
    let granted_ids: std::collections::HashSet<String> = form.access.into_iter().collect();

    for u in &all_users {
        let uid_str = u.id.to_string();
        let currently_has = models::user_has_app_access(&state.pool, app.app.id, &uid_str)
            .await
            .unwrap_or(false);
        let should_have = granted_ids.contains(&uid_str);
        if should_have && !currently_has {
            let _ = models::grant_app_access(&state.pool, app.app.id, &uid_str).await;
            let uname = u
                .username
                .as_ref()
                .map(|un| un.as_str().to_string())
                .unwrap_or_default();
            state.audit.log_with_app(
                &user_id_str,
                "app_access_granted",
                "app",
                &app.app.slug,
                Some(&serde_json::json!({"user_id": uid_str, "username": uname}).to_string()),
                Some(app.app.id),
            );
        } else if !should_have && currently_has {
            let _ = models::revoke_app_access(&state.pool, app.app.id, &uid_str).await;
            let uname = u
                .username
                .as_ref()
                .map(|un| un.as_str().to_string())
                .unwrap_or_default();
            state.audit.log_with_app(
                &user_id_str,
                "app_access_revoked",
                "app",
                &app.app.slug,
                Some(&serde_json::json!({"user_id": uid_str, "username": uname}).to_string()),
                Some(app.app.id),
            );
        }
    }

    auth::set_flash(&session, "success", "Access updated").await;
    Ok(Redirect::to(&format!("/apps/{}/settings", app.app.slug)).into_response())
}

#[derive(serde::Deserialize)]
struct CreateTokenForm {
    name: String,
}

async fn create_token(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Form(form): Form<CreateTokenForm>,
) -> axum::response::Result<axum::response::Response> {
    if !auth::has_min_role(&role.0, "editor") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }
    let user_id_str = user.id.to_string();
    let name = form.name.trim();
    if name.is_empty() {
        auth::set_flash(&session, "error", "Token name is required").await;
        return Ok(Redirect::to(&format!("/apps/{}/settings", app.app.slug)).into_response());
    }

    // Create token via allowthem
    match state
        .ath
        .db()
        .create_api_token(user.id, name, None, None)
        .await
    {
        Ok((raw_token, info)) => {
            // Hash the raw token for app_tokens lookup
            use sha2::{Digest, Sha256};
            let token_hash = hex::encode(Sha256::digest(raw_token.as_bytes()));
            let token_id_str = info.id.to_string();
            if let Err(e) =
                models::create_app_token(&state.pool, &token_id_str, app.app.id, &token_hash).await
            {
                tracing::error!("Failed to create app_token mapping: {e}");
            }
            state.audit.log_with_app(
                &user_id_str,
                "token_create",
                "token",
                name,
                None,
                Some(app.app.id),
            );
            auth::set_flash(&session, "token", &format!("Token created: {raw_token}")).await;
        }
        Err(e) => {
            auth::set_flash(&session, "error", &format!("Failed to create token: {e}")).await;
        }
    }
    Ok(Redirect::to(&format!("/apps/{}/settings", app.app.slug)).into_response())
}

async fn delete_token(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    axum::extract::Path((_app_slug, token_id)): axum::extract::Path<(String, String)>,
) -> axum::response::Result<axum::response::Response> {
    if !auth::has_min_role(&role.0, "editor") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }
    let user_id_str = user.id.to_string();

    // Delete from allowthem (parse token_id to ApiTokenId)
    if let Ok(uuid) = uuid::Uuid::parse_str(&token_id) {
        let ath_token_id = allowthem_core::ApiTokenId::from_uuid(uuid);
        let _ = state.ath.db().delete_api_token(ath_token_id).await;
    }
    // Delete from app_tokens
    let _ = models::delete_app_token(&state.pool, &token_id).await;

    state.audit.log_with_app(
        &user_id_str,
        "token_delete",
        "token",
        &token_id,
        None,
        Some(app.app.id),
    );
    auth::set_flash(&session, "success", "Token deleted").await;
    Ok(Redirect::to(&format!("/apps/{}/settings", app.app.slug)).into_response())
}

async fn data_page(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
) -> axum::response::Result<axum::response::Response> {
    if !auth::has_min_role(&role.0, "admin") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }
    let csrf_token = auth::ensure_csrf_token(&session).await;
    let user_role = &role.0;
    let current_username = username_str(&user);
    let flash = auth::take_flash(&session).await;
    let echo = auth::flash_echo_trigger(&flash);

    let data_result = flash.as_ref().and_then(|(k, v)| {
        if k == "data_result" {
            serde_json::from_str::<serde_json::Value>(v).ok()
        } else {
            None
        }
    });

    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("apps/data.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            user_role => user_role,
            current_username => current_username,
            app => app.template_context(),
            nav_schemas => app.nav_schemas(&state.config),
            data_result => data_result,
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok((echo, Html(html)).into_response())
}

async fn import_data(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    mut multipart: axum::extract::Multipart,
) -> axum::response::Result<axum::response::Response> {
    if !auth::has_min_role(&role.0, "admin") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }
    let user_id_str = user.id.to_string();
    let app_dir = state.config.app_dir(&app.app.slug);

    let mut csrf_token = None;
    let data = loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                if field.name() == Some("_csrf") {
                    csrf_token = field.text().await.ok();
                    continue;
                }
                if field.name() != Some("bundle") {
                    continue;
                }
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| format!("Upload error: {e}"))?;
                if !bytes.is_empty() {
                    break bytes;
                }
            }
            Ok(None) => {
                auth::set_flash(
                    &session,
                    "data_result",
                    &serde_json::json!({"status": "error", "message": "No file provided", "warnings": []}).to_string(),
                ).await;
                return Ok(Redirect::to(&format!("/apps/{}/data", app.app.slug)).into_response());
            }
            Err(e) => {
                auth::set_flash(
                    &session,
                    "data_result",
                    &serde_json::json!({"status": "error", "message": e.to_string(), "warnings": []}).to_string(),
                ).await;
                return Ok(Redirect::to(&format!("/apps/{}/data", app.app.slug)).into_response());
            }
        }
    };

    if !matches!(csrf_token.as_deref(), Some(token) if auth::verify_csrf_token(&session, token).await)
    {
        return Ok((axum::http::StatusCode::FORBIDDEN, "Invalid CSRF token").into_response());
    }

    match crate::sync::import_bundle_from_bytes(&app_dir, &state.pool, app.app.id, &data).await {
        Ok(warnings) => {
            crate::cache::rebuild(&state.cache, &state.etag_cache, &state.config.data_dir);
            state
                .audit
                .log_with_app(&user_id_str, "import", "bundle", "", None, Some(app.app.id));

            let (status, message) = if warnings.is_empty() {
                (
                    "success".to_string(),
                    "Bundle imported successfully".to_string(),
                )
            } else {
                (
                    "warning".to_string(),
                    format!("Bundle imported with {} warnings", warnings.len()),
                )
            };

            auth::set_flash(
                &session,
                "data_result",
                &serde_json::json!({
                    "status": status,
                    "message": message,
                    "warnings": warnings,
                })
                .to_string(),
            )
            .await;
        }
        Err(e) => {
            auth::set_flash(
                &session,
                "data_result",
                &serde_json::json!({"status": "error", "message": e.to_string(), "warnings": []})
                    .to_string(),
            )
            .await;
        }
    }

    Ok(Redirect::to(&format!("/apps/{}/data", app.app.slug)).into_response())
}

async fn export_data(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Form(_form): Form<std::collections::HashMap<String, String>>,
) -> axum::response::Result<axum::response::Response> {
    if !auth::has_min_role(&role.0, "admin") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }
    let user_id_str = user.id.to_string();
    let app_dir = state.config.app_dir(&app.app.slug);
    let tmp =
        std::env::temp_dir().join(format!("substrukt-export-{}.tar.gz", uuid::Uuid::new_v4()));

    Ok(
        match crate::sync::export_bundle(&app_dir, &state.pool, app.app.id, &tmp).await {
            Ok(()) => match std::fs::read(&tmp) {
                Ok(data) => {
                    let _ = std::fs::remove_file(&tmp);
                    state.audit.log_with_app(
                        &user_id_str,
                        "export",
                        "bundle",
                        "",
                        None,
                        Some(app.app.id),
                    );

                    let date = chrono::Utc::now().format("%Y-%m-%d");
                    let filename = format!("substrukt-{}-{date}.tar.gz", app.app.slug);
                    let disposition = format!("attachment; filename=\"{filename}\"");

                    let mut response = axum::body::Body::from(data).into_response();
                    response.headers_mut().insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("application/gzip"),
                    );
                    if let Ok(val) = axum::http::HeaderValue::from_str(&disposition) {
                        response
                            .headers_mut()
                            .insert(axum::http::header::CONTENT_DISPOSITION, val);
                    }
                    response
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    auth::set_flash(&session, "error", &format!("Export failed: {e}")).await;
                    Redirect::to(&format!("/apps/{}/data", app.app.slug)).into_response()
                }
            },
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                auth::set_flash(&session, "error", &format!("Export failed: {e}")).await;
                Redirect::to(&format!("/apps/{}/data", app.app.slug)).into_response()
            }
        },
    )
}

async fn delete_app(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
) -> axum::response::Result<axum::response::Response> {
    if !auth::has_min_role(&role.0, "admin") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }
    let user_id_str = user.id.to_string();

    // Prevent deleting the default app
    if app.app.slug == "default" {
        auth::set_flash(&session, "error", "Cannot delete the default app").await;
        return Ok(Redirect::to(&format!("/apps/{}/settings", app.app.slug)).into_response());
    }

    // Cancel auto-deploy tasks for this app's deployments
    if let Ok(deployments) = state.audit.list_deployments_for_app(app.app.id).await {
        for dep in &deployments {
            crate::webhooks::cancel_auto_deploy_task(&state, dep.id);
            let _ = state.audit.delete_deployment(dep.id).await;
        }
    }

    // Delete app from DB (CASCADE deletes tokens, access, uploads, references)
    models::delete_app(&state.pool, app.app.id)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    // Remove app directory from disk
    let app_dir = state.config.app_dir(&app.app.slug);
    if app_dir.exists() {
        let _ = std::fs::remove_dir_all(&app_dir);
    }

    // Clear cache for this app
    crate::cache::remove_app(&state.cache, &app.app.slug);

    state.audit.log(
        &user_id_str,
        "app_delete",
        "app",
        &app.app.slug,
        Some(&serde_json::json!({"name": app.app.name}).to_string()),
    );

    auth::set_flash(
        &session,
        "success",
        &format!("App '{}' deleted", app.app.name),
    )
    .await;
    Ok(Redirect::to("/apps").into_response())
}
