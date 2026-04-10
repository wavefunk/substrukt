use axum::{
    Form, Router,
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
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
) -> axum::response::Result<Html<String>> {
    let user_id = auth::current_user_id(&session).await.unwrap_or(0);
    let user_role = auth::current_user_role(&session).await.unwrap_or_default();
    let current_username = auth::current_username(&session).await.unwrap_or_default();
    let csrf_token = auth::ensure_csrf_token(&session).await;
    let flash = auth::take_flash(&session).await;

    let apps = if user_role == "admin" {
        models::list_apps(&state.pool)
            .await
            .map_err(|e| format!("DB error: {e}"))?
    } else {
        models::list_apps_for_user(&state.pool, user_id)
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
    Ok(Html(html))
}

async fn new_app_form(
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
) -> axum::response::Result<Html<String>> {
    auth::require_role(&session, "admin").await?;
    let csrf_token = auth::ensure_csrf_token(&session).await;
    let user_role = auth::current_user_role(&session).await.unwrap_or_default();
    let current_username = auth::current_username(&session).await.unwrap_or_default();

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
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<CreateAppForm>,
) -> axum::response::Result<axum::response::Response> {
    let user_id = auth::require_role(&session, "admin").await?;
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
                &user_id.to_string(),
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
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
) -> axum::response::Result<Html<String>> {
    auth::require_role(&session, "admin").await?;
    let csrf_token = auth::ensure_csrf_token(&session).await;
    let user_role = auth::current_user_role(&session).await.unwrap_or_default();
    let current_username = auth::current_username(&session).await.unwrap_or_default();
    let flash = auth::take_flash(&session).await;

    let users = models::list_app_users(&state.pool, app.app.id)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    let user_data: Vec<minijinja::Value> = users
        .iter()
        .map(|(u, has_access)| {
            minijinja::context! {
                id => u.id,
                username => u.username,
                role => u.role,
                has_access => has_access,
            }
        })
        .collect();

    let tokens = models::list_api_tokens_for_app(&state.pool, app.app.id)
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
    Ok(Html(html))
}

#[derive(serde::Deserialize)]
struct UpdateNameForm {
    name: String,
}

async fn update_app_name(
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Form(form): Form<UpdateNameForm>,
) -> axum::response::Result<axum::response::Response> {
    let user_id = auth::require_role(&session, "admin").await?;
    let name = form.name.trim();
    if name.is_empty() {
        auth::set_flash(&session, "error", "Name cannot be empty").await;
    } else {
        models::update_app_name(&state.pool, app.app.id, name)
            .await
            .map_err(|e| format!("DB error: {e}"))?;
        state.audit.log(
            &user_id.to_string(),
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
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Form(form): Form<AccessForm>,
) -> axum::response::Result<axum::response::Response> {
    let user_id = auth::require_role(&session, "admin").await?;

    let all_users = models::list_app_users(&state.pool, app.app.id)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    let granted_ids: std::collections::HashSet<i64> = form
        .access
        .iter()
        .filter_map(|s| s.parse::<i64>().ok())
        .collect();

    for (user, currently_has) in &all_users {
        let should_have = granted_ids.contains(&user.id);
        if should_have && !currently_has {
            let _ = models::grant_app_access(&state.pool, app.app.id, user.id).await;
            state.audit.log_with_app(
                &user_id.to_string(),
                "app_access_granted",
                "app",
                &app.app.slug,
                Some(
                    &serde_json::json!({"user_id": user.id, "username": user.username}).to_string(),
                ),
                Some(app.app.id),
            );
        } else if !should_have && *currently_has {
            let _ = models::revoke_app_access(&state.pool, app.app.id, user.id).await;
            state.audit.log_with_app(
                &user_id.to_string(),
                "app_access_revoked",
                "app",
                &app.app.slug,
                Some(
                    &serde_json::json!({"user_id": user.id, "username": user.username}).to_string(),
                ),
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
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Form(form): Form<CreateTokenForm>,
) -> axum::response::Result<axum::response::Response> {
    let user_id = auth::require_role(&session, "editor").await?;
    let name = form.name.trim();
    if name.is_empty() {
        auth::set_flash(&session, "error", "Token name is required").await;
        return Ok(Redirect::to(&format!("/apps/{}/settings", app.app.slug)).into_response());
    }

    let raw_token = crate::auth::token::generate_token();
    let token_hash = crate::auth::token::hash_token(&raw_token);
    match models::create_api_token(&state.pool, user_id, app.app.id, name, &token_hash).await {
        Ok(_) => {
            state.audit.log_with_app(
                &user_id.to_string(),
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
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    axum::extract::Path((_app_slug, token_id)): axum::extract::Path<(String, i64)>,
) -> axum::response::Result<axum::response::Response> {
    let user_id = auth::require_role(&session, "editor").await?;
    models::delete_api_token(&state.pool, token_id, app.app.id)
        .await
        .map_err(|e| format!("DB error: {e}"))?;
    state.audit.log_with_app(
        &user_id.to_string(),
        "token_delete",
        "token",
        &token_id.to_string(),
        None,
        Some(app.app.id),
    );
    auth::set_flash(&session, "success", "Token deleted").await;
    Ok(Redirect::to(&format!("/apps/{}/settings", app.app.slug)).into_response())
}

async fn data_page(
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
) -> axum::response::Result<Html<String>> {
    auth::require_role(&session, "admin").await?;
    let csrf_token = auth::ensure_csrf_token(&session).await;
    let user_role = auth::current_user_role(&session).await.unwrap_or_default();
    let current_username = auth::current_username(&session).await.unwrap_or_default();
    let flash = auth::take_flash(&session).await;

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
    Ok(Html(html))
}

async fn import_data(
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    mut multipart: axum::extract::Multipart,
) -> axum::response::Result<axum::response::Response> {
    let user_id = auth::require_role(&session, "admin").await?;
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
            state.audit.log_with_app(
                &user_id.to_string(),
                "import",
                "bundle",
                "",
                None,
                Some(app.app.id),
            );

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
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Form(_form): Form<std::collections::HashMap<String, String>>,
) -> axum::response::Result<axum::response::Response> {
    let user_id = auth::require_role(&session, "admin").await?;
    let app_dir = state.config.app_dir(&app.app.slug);
    let tmp =
        std::env::temp_dir().join(format!("substrukt-export-{}.tar.gz", uuid::Uuid::new_v4()));

    Ok(
        match crate::sync::export_bundle(&app_dir, &state.pool, app.app.id, &tmp).await {
            Ok(()) => match std::fs::read(&tmp) {
                Ok(data) => {
                    let _ = std::fs::remove_file(&tmp);
                    state.audit.log_with_app(
                        &user_id.to_string(),
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
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
) -> axum::response::Result<axum::response::Response> {
    let user_id = auth::require_role(&session, "admin").await?;

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
        &user_id.to_string(),
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
