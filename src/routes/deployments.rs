use axum::{
    Extension, Form, Router,
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect},
    routing::get,
};
use axum_htmx::HxRequest;
use sqlx;
use tower_sessions::Session;

use crate::app_context::AppContext;
use crate::audit::validate_deployment_slug;
use crate::auth;
use crate::state::AppState;
use crate::templates::base_for_htmx;
use crate::webhooks::{self, validate_webhook_url};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_deployments))
        .route("/new", get(new_deployment_form).post(create_deployment))
        .route("/{slug}/edit", get(edit_deployment_form))
        .route("/{slug}", axum::routing::post(update_deployment))
        .route("/{slug}/delete", axum::routing::post(delete_deployment))
        .route("/{slug}/fire", axum::routing::post(fire_deployment))
}

#[derive(serde::Deserialize)]
struct DeploymentForm {
    name: String,
    slug: String,
    webhook_url: String,
    #[serde(default)]
    webhook_auth_token: String,
    #[serde(default)]
    _token_action: String,
    #[serde(default)]
    include_drafts: Option<String>,
    #[serde(default)]
    auto_deploy: Option<String>,
    #[serde(default)]
    debounce_seconds: Option<i64>,
}

async fn list_deployments(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
) -> axum::response::Result<axum::response::Response> {
    if !auth::has_min_role(&role.0, "editor") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }
    let csrf_token = auth::ensure_csrf_token(&session).await;
    let user_role = &role.0;
    let current_username = user
        .username
        .as_ref()
        .map(|u| u.as_str().to_string())
        .unwrap_or_default();
    let flash = auth::take_flash(&session).await;
    let echo = auth::flash_echo_trigger(&flash);

    let deployments = state
        .audit
        .list_deployments_for_app(app.app.id)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    let mut deployment_data: Vec<minijinja::Value> = Vec::new();
    for dep in &deployments {
        let is_dirty = state
            .audit
            .is_dirty_for_deployment(dep.id)
            .await
            .unwrap_or(false);

        // Fetch last_fired_at for display
        let last_fired: Option<String> = {
            let row: Option<(Option<String>,)> = sqlx::query_as(
                "SELECT last_fired_at FROM deployment_state WHERE deployment_id = ?",
            )
            .bind(dep.id)
            .fetch_optional(state.audit.pool_ref())
            .await
            .unwrap_or(None);
            row.and_then(|(ts,)| ts)
        };

        deployment_data.push(minijinja::context! {
            id => dep.id,
            name => dep.name,
            slug => dep.slug,
            webhook_url => dep.webhook_url,
            include_drafts => dep.include_drafts,
            auto_deploy => dep.auto_deploy,
            debounce_seconds => dep.debounce_seconds,
            is_dirty => is_dirty,
            last_fired => last_fired,
        });
    }

    let history = state
        .audit
        .list_webhook_history_for_deployment(None, None)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    let history_data: Vec<minijinja::Value> = history
        .iter()
        .map(|h| {
            minijinja::context! {
                id => h.id,
                deployment_name => h.deployment_name,
                deployment_slug => h.deployment_slug,
                trigger_source => h.trigger_source,
                status => h.status,
                http_status => h.http_status,
                error_message => h.error_message,
                response_time_ms => h.response_time_ms,
                attempt_count => h.attempt_count,
                group_id => h.group_id,
                created_at => h.created_at,
            }
        })
        .collect();

    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("deployments/list.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            user_role => user_role,
            current_username => current_username,
            app => app.template_context(),
            nav_schemas => app.nav_schemas(&state.config),
            deployments => deployment_data,
            history => history_data,
            flash_kind => flash.as_ref().map(|(k, _)| k.as_str()),
            flash_message => flash.as_ref().map(|(_, m)| m.as_str()),
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok((echo, Html(html)).into_response())
}

async fn new_deployment_form(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
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
    let current_username = user
        .username
        .as_ref()
        .map(|u| u.as_str().to_string())
        .unwrap_or_default();

    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("deployments/form.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            user_role => user_role,
            current_username => current_username,
            app => app.template_context(),
            nav_schemas => app.nav_schemas(&state.config),
            editing => false,
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html))
}

async fn create_deployment(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Form(form): Form<DeploymentForm>,
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
    let current_username = user
        .username
        .as_ref()
        .map(|u| u.as_str().to_string())
        .unwrap_or_default();

    let name = form.name.trim();
    let slug = form.slug.trim();
    let webhook_url = form.webhook_url.trim();

    if name.is_empty() || webhook_url.is_empty() {
        return render_form_with_error(
            &state,
            &app,
            &session,
            is_htmx,
            "Name and Webhook URL are required",
            None,
            &user_role,
            &current_username,
        )
        .await;
    }

    if let Err(e) = validate_deployment_slug(slug) {
        return render_form_with_error(
            &state,
            &app,
            &session,
            is_htmx,
            &e,
            None,
            &user_role,
            &current_username,
        )
        .await;
    }

    if let Err(e) = validate_webhook_url(webhook_url, state.config.allow_private_webhooks) {
        return render_form_with_error(
            &state,
            &app,
            &session,
            is_htmx,
            &e,
            None,
            &user_role,
            &current_username,
        )
        .await;
    }

    let auth_token = if form.webhook_auth_token.trim().is_empty() {
        None
    } else {
        Some(form.webhook_auth_token.trim())
    };

    let include_drafts = form.include_drafts.as_deref() == Some("on");
    let auto_deploy = form.auto_deploy.as_deref() == Some("on");
    let debounce_seconds = form.debounce_seconds.unwrap_or(300).max(10);

    match state
        .audit
        .create_deployment(
            app.app.id,
            name,
            slug,
            webhook_url,
            auth_token,
            include_drafts,
            auto_deploy,
            debounce_seconds,
        )
        .await
    {
        Ok(dep) => {
            state.audit.log_with_app(
                &user_id_str,
                "deployment_create",
                "deployment",
                &dep.slug,
                Some(&serde_json::json!({"name": dep.name}).to_string()),
                Some(app.app.id),
            );
            if dep.auto_deploy {
                webhooks::spawn_auto_deploy_task(&state, dep);
            }
            auth::set_flash(
                &session,
                "success",
                &format!("Deployment '{}' created", name),
            )
            .await;
            Ok(Redirect::to(&format!("/apps/{}/deployments", app.app.slug)).into_response())
        }
        Err(e) => {
            let msg = if e.to_string().contains("UNIQUE") {
                "A deployment with this slug already exists".to_string()
            } else {
                e.to_string()
            };
            render_form_with_error(
                &state,
                &app,
                &session,
                is_htmx,
                &msg,
                None,
                &user_role,
                &current_username,
            )
            .await
        }
    }
}

async fn edit_deployment_form(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, slug)): Path<(String, String)>,
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
    let current_username = user
        .username
        .as_ref()
        .map(|u| u.as_str().to_string())
        .unwrap_or_default();

    let dep = state
        .audit
        .get_deployment_by_slug_and_app(app.app.id, &slug)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    let Some(dep) = dep else {
        return Ok((
            axum::http::StatusCode::NOT_FOUND,
            Html("Not found".to_string()),
        )
            .into_response());
    };

    let has_token = dep.webhook_auth_token.is_some();

    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("deployments/form.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            user_role => user_role,
            current_username => current_username,
            app => app.template_context(),
            nav_schemas => app.nav_schemas(&state.config),
            editing => true,
            has_token => has_token,
            deployment => minijinja::context! {
                id => dep.id,
                name => dep.name,
                slug => dep.slug,
                webhook_url => dep.webhook_url,
                include_drafts => dep.include_drafts,
                auto_deploy => dep.auto_deploy,
                debounce_seconds => dep.debounce_seconds,
            },
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html).into_response())
}

async fn update_deployment(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, slug)): Path<(String, String)>,
    Form(form): Form<DeploymentForm>,
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
    let current_username = user
        .username
        .as_ref()
        .map(|u| u.as_str().to_string())
        .unwrap_or_default();

    let dep = state
        .audit
        .get_deployment_by_slug_and_app(app.app.id, &slug)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    let Some(dep) = dep else {
        return Ok((
            axum::http::StatusCode::NOT_FOUND,
            Html("Not found".to_string()),
        )
            .into_response());
    };

    let name = form.name.trim();
    let new_slug = form.slug.trim();
    let webhook_url = form.webhook_url.trim();

    if name.is_empty() || webhook_url.is_empty() {
        return render_form_with_error(
            &state,
            &app,
            &session,
            is_htmx,
            "Name and Webhook URL are required",
            Some(&dep),
            &user_role,
            &current_username,
        )
        .await;
    }

    if let Err(e) = validate_deployment_slug(new_slug) {
        return render_form_with_error(
            &state,
            &app,
            &session,
            is_htmx,
            &e,
            Some(&dep),
            &user_role,
            &current_username,
        )
        .await;
    }

    if let Err(e) = validate_webhook_url(webhook_url, state.config.allow_private_webhooks) {
        return render_form_with_error(
            &state,
            &app,
            &session,
            is_htmx,
            &e,
            Some(&dep),
            &user_role,
            &current_username,
        )
        .await;
    }

    // Handle auth token: keep, clear, or update
    let auth_token = match form._token_action.as_str() {
        "clear" => None,
        "update" => {
            if form.webhook_auth_token.trim().is_empty() {
                None
            } else {
                Some(form.webhook_auth_token.trim().to_string())
            }
        }
        _ => dep.webhook_auth_token.clone(), // "keep" or default
    };

    let include_drafts = form.include_drafts.as_deref() == Some("on");
    let auto_deploy = form.auto_deploy.as_deref() == Some("on");
    let debounce_seconds = form.debounce_seconds.unwrap_or(300).max(10);

    if let Err(e) = state
        .audit
        .update_deployment(
            dep.id,
            name,
            new_slug,
            webhook_url,
            auth_token.as_deref(),
            include_drafts,
            auto_deploy,
            debounce_seconds,
        )
        .await
    {
        let msg = if e.to_string().contains("UNIQUE") {
            "A deployment with this slug already exists".to_string()
        } else {
            e.to_string()
        };
        return render_form_with_error(
            &state,
            &app,
            &session,
            is_htmx,
            &msg,
            Some(&dep),
            &user_role,
            &current_username,
        )
        .await;
    }

    state.audit.log_with_app(
        &user_id_str,
        "deployment_updated",
        "deployment",
        new_slug,
        Some(&serde_json::json!({"name": name}).to_string()),
        Some(app.app.id),
    );

    // Cancel old auto-deploy task, optionally spawn new one
    webhooks::cancel_auto_deploy_task(&state, dep.id);
    if auto_deploy && let Ok(Some(updated_dep)) = state.audit.get_deployment_by_id(dep.id).await {
        webhooks::spawn_auto_deploy_task(&state, updated_dep);
    }

    auth::set_flash(
        &session,
        "success",
        &format!("Deployment '{}' updated", name),
    )
    .await;
    Ok(Redirect::to(&format!("/apps/{}/deployments", app.app.slug)).into_response())
}

async fn delete_deployment(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, slug)): Path<(String, String)>,
) -> axum::response::Result<axum::response::Response> {
    if !auth::has_min_role(&role.0, "admin") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }
    let user_id_str = user.id.to_string();

    let dep = state
        .audit
        .get_deployment_by_slug_and_app(app.app.id, &slug)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    let Some(dep) = dep else {
        return Ok((
            axum::http::StatusCode::NOT_FOUND,
            Html("Not found".to_string()),
        )
            .into_response());
    };

    webhooks::cancel_auto_deploy_task(&state, dep.id);
    state
        .audit
        .delete_deployment(dep.id)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    state.audit.log_with_app(
        &user_id_str,
        "deployment_delete",
        "deployment",
        &dep.slug,
        Some(&serde_json::json!({"name": dep.name}).to_string()),
        Some(app.app.id),
    );

    auth::set_flash(
        &session,
        "success",
        &format!("Deployment '{}' deleted", dep.name),
    )
    .await;
    Ok(Redirect::to(&format!("/apps/{}/deployments", app.app.slug)).into_response())
}

async fn fire_deployment(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, slug)): Path<(String, String)>,
) -> axum::response::Result<axum::response::Response> {
    if !auth::has_min_role(&role.0, "editor") {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into());
    }
    let user_id_str = user.id.to_string();

    let dep = state
        .audit
        .get_deployment_by_slug_and_app(app.app.id, &slug)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    let Some(dep) = dep else {
        return Ok((
            axum::http::StatusCode::NOT_FOUND,
            Html("Not found".to_string()),
        )
            .into_response());
    };

    match webhooks::fire_webhook(
        &state.http_client,
        &state.audit,
        &dep,
        webhooks::TriggerSource::Manual,
        &app.app.slug,
    )
    .await
    {
        Ok(_) => {
            state.audit.log_with_app(
                &user_id_str,
                "deployment_fired",
                "deployment",
                &dep.slug,
                None,
                Some(app.app.id),
            );
            auth::set_flash(
                &session,
                "success",
                &format!("Deployment '{}' triggered", dep.name),
            )
            .await;
        }
        Err(e) => {
            tracing::warn!("Webhook failed for deployment {}: {e}", dep.slug);
            state.audit.log_with_app(
                &user_id_str,
                "deployment_fired",
                "deployment",
                &dep.slug,
                None,
                Some(app.app.id),
            );
            auth::set_flash(
                &session,
                "error",
                "Webhook failed \u{2014} retries in progress",
            )
            .await;
        }
    }

    Ok(Redirect::to(&format!("/apps/{}/deployments", app.app.slug)).into_response())
}

async fn render_form_with_error(
    state: &AppState,
    app: &AppContext,
    session: &Session,
    is_htmx: bool,
    error: &str,
    dep: Option<&crate::audit::Deployment>,
    user_role: &str,
    current_username: &str,
) -> axum::response::Result<axum::response::Response> {
    let csrf_token = auth::ensure_csrf_token(session).await;

    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("deployments/form.html")
        .map_err(|e| format!("Template error: {e}"))?;

    let editing = dep.is_some();
    let has_token = dep.map(|d| d.webhook_auth_token.is_some()).unwrap_or(false);

    let deployment_ctx = dep.map(|d| {
        minijinja::context! {
            id => d.id,
            name => d.name,
            slug => d.slug,
            webhook_url => d.webhook_url,
            include_drafts => d.include_drafts,
            auto_deploy => d.auto_deploy,
            debounce_seconds => d.debounce_seconds,
        }
    });

    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            user_role => user_role,
            current_username => current_username,
            app => app.template_context(),
            nav_schemas => app.nav_schemas(&state.config),
            editing => editing,
            has_token => has_token,
            deployment => deployment_ctx,
            error => error,
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html).into_response())
}
