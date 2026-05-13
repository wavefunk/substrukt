use axum::{
    Extension, Form, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
    routing::get,
};
use axum_htmx::HxRequest;
use tower_sessions::Session;

use crate::app_context::AppContext;
use crate::auth;
use crate::routes::error_response_with_nav;
use crate::schema;
use crate::state::AppState;
use crate::templates::base_for_htmx;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_schemas))
        .route("/new", get(new_schema_page).post(create_schema))
        .route("/{slug}/edit", get(edit_schema_page))
        .route(
            "/{slug}",
            axum::routing::post(update_schema)
                .put(update_schema)
                .delete(delete_schema),
        )
}

async fn list_schemas(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
) -> axum::response::Result<axum::response::Response> {
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let schemas =
        schema::list_schemas(&schemas_dir).map_err(|e| format!("Failed to list schemas: {e}"))?;

    let content_dir = state.config.app_content_dir(&app.app.slug);
    let schema_data: Vec<minijinja::Value> = schemas
        .iter()
        .map(|s| {
            let entry_count = crate::content::list_entries(&content_dir, s)
                .map(|e| e.len())
                .unwrap_or(0);
            minijinja::context! {
                title => s.meta.title,
                slug => s.meta.slug,
                storage => s.meta.storage.to_string(),
                kind => s.meta.kind.to_string(),
                field_count => schema::property_count(&s.schema),
                entry_count => entry_count,
                is_single => s.meta.kind == crate::schema::models::Kind::Single,
            }
        })
        .collect();

    let csrf_token = auth::ensure_csrf_token(&session).await;
    let ath_csrf = auth::ath_csrf(&session).await;
    let flash = auth::take_flash(&session).await;
    let echo = auth::flash_echo_trigger(&flash);
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
        .get_template("schemas/list.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            ath_csrf => ath_csrf,
            user_role => user_role,
            current_username => current_username,
            app => app.template_context(),
            nav_schemas => app.nav_schemas(&state.config),
            schemas => schema_data,
            flash_kind => flash.as_ref().map(|(k, _)| k.as_str()),
            flash_message => flash.as_ref().map(|(_, m)| m.as_str()),
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok((echo, Html(html)).into_response())
}

async fn new_schema_page(
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
    let ath_csrf = auth::ath_csrf(&session).await;
    let user_role = &role.0;
    let current_username = user
        .username
        .as_ref()
        .map(|u| u.as_str().to_string())
        .unwrap_or_default();
    let default_schema = serde_json::json!({
        "x-substrukt": {
            "title": "",
            "slug": "",
            "storage": "directory"
        },
        "type": "object",
        "properties": {},
        "required": []
    });

    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("schemas/edit.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            ath_csrf => ath_csrf,
            user_role => user_role,
            current_username => current_username,
            app => app.template_context(),
            nav_schemas => app.nav_schemas(&state.config),
            is_new => true,
            schema_json => serde_json::to_string_pretty(&default_schema).unwrap_or_default(),
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html))
}

#[derive(serde::Deserialize)]
pub struct SchemaForm {
    schema_json: String,
}

async fn create_schema(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Form(form): Form<SchemaForm>,
) -> impl IntoResponse {
    if !auth::has_min_role(&role.0, "admin") {
        return (
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into_response();
    }
    let user_id_str = user.id.to_string();
    let user_role = role.0.clone();
    let current_username = user
        .username
        .as_ref()
        .map(|u| u.as_str().to_string())
        .unwrap_or_default();
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let schema_value: serde_json::Value = match serde_json::from_str(&form.schema_json) {
        Ok(v) => v,
        Err(e) => {
            return render_schema_edit(
                &state,
                &session,
                &app,
                is_htmx,
                true,
                &form.schema_json,
                &format!("Invalid JSON: {e}"),
                &user_role,
                &current_username,
            )
            .await
            .into_response();
        }
    };

    if let Err(e) = schema::validate_schema(&schema_value) {
        return render_schema_edit(
            &state,
            &session,
            &app,
            is_htmx,
            true,
            &form.schema_json,
            &format!("{e}"),
            &user_role,
            &current_username,
        )
        .await
        .into_response();
    }

    let meta = schema_value
        .get("x-substrukt")
        .and_then(|v| serde_json::from_value::<schema::models::SubstruktMeta>(v.clone()).ok());

    let slug = match meta {
        Some(ref m) if !m.slug.is_empty() => m.slug.clone(),
        Some(ref m) if !m.title.is_empty() => slug::slugify(&m.title),
        _ => {
            return render_schema_edit(
                &state,
                &session,
                &app,
                is_htmx,
                true,
                &form.schema_json,
                "Schema must have a title and slug in x-substrukt",
                &user_role,
                &current_username,
            )
            .await
            .into_response();
        }
    };

    if let Err(e) = schema::save_schema(&schemas_dir, &slug, &schema_value) {
        return render_schema_edit(
            &state,
            &session,
            &app,
            is_htmx,
            true,
            &form.schema_json,
            &format!("Save error: {e}"),
            &user_role,
            &current_username,
        )
        .await
        .into_response();
    }

    state.audit.log_with_app(
        &user_id_str,
        "schema_create",
        "schema",
        &slug,
        None,
        Some(app.app.id),
    );
    auth::set_flash(&session, "success", "Schema created").await;
    Redirect::to(&format!("/apps/{}/schemas", app.app.slug)).into_response()
}

async fn edit_schema_page(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, slug)): Path<(String, String)>,
) -> axum::response::Result<axum::response::Response> {
    if !auth::has_min_role(&role.0, "admin") {
        let csrf_token = auth::ensure_csrf_token(&session).await;
        let ath_csrf = auth::ath_csrf(&session).await;
        let current_username = user
            .username
            .as_ref()
            .map(|u| u.as_str().to_string())
            .unwrap_or_default();
        return Ok(error_response_with_nav(
            &state,
            StatusCode::FORBIDDEN,
            "Insufficient permissions",
            is_htmx,
            &role.0,
            &current_username,
            &csrf_token,
            &ath_csrf,
        ));
    }
    let csrf_token = auth::ensure_csrf_token(&session).await;
    let ath_csrf = auth::ath_csrf(&session).await;
    let user_role = &role.0;
    let current_username = user
        .username
        .as_ref()
        .map(|u| u.as_str().to_string())
        .unwrap_or_default();
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let Some(schema) =
        schema::get_schema(&schemas_dir, &slug).map_err(|e| format!("Error: {e}"))?
    else {
        return Ok(error_response_with_nav(
            &state,
            StatusCode::NOT_FOUND,
            "Schema not found",
            is_htmx,
            user_role,
            &current_username,
            &csrf_token,
            &ath_csrf,
        ));
    };

    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("schemas/edit.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            ath_csrf => ath_csrf,
            user_role => user_role,
            current_username => current_username,
            app => app.template_context(),
            nav_schemas => app.nav_schemas(&state.config),
            is_new => false,
            slug => slug,
            schema_json => serde_json::to_string_pretty(&schema.schema).unwrap_or_default(),
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html).into_response())
}

async fn update_schema(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, slug)): Path<(String, String)>,
    Form(form): Form<SchemaForm>,
) -> impl IntoResponse {
    if !auth::has_min_role(&role.0, "admin") {
        return (
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into_response();
    }
    let user_id_str = user.id.to_string();
    let user_role = role.0.clone();
    let current_username = user
        .username
        .as_ref()
        .map(|u| u.as_str().to_string())
        .unwrap_or_default();
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let schema_value: serde_json::Value = match serde_json::from_str(&form.schema_json) {
        Ok(v) => v,
        Err(e) => {
            return render_schema_edit(
                &state,
                &session,
                &app,
                is_htmx,
                false,
                &form.schema_json,
                &format!("Invalid JSON: {e}"),
                &user_role,
                &current_username,
            )
            .await
            .into_response();
        }
    };

    if let Err(e) = schema::validate_schema(&schema_value) {
        return render_schema_edit(
            &state,
            &session,
            &app,
            is_htmx,
            false,
            &form.schema_json,
            &format!("{e}"),
            &user_role,
            &current_username,
        )
        .await
        .into_response();
    }

    if let Err(e) = schema::save_schema(&schemas_dir, &slug, &schema_value) {
        return render_schema_edit(
            &state,
            &session,
            &app,
            is_htmx,
            false,
            &form.schema_json,
            &format!("Save error: {e}"),
            &user_role,
            &current_username,
        )
        .await
        .into_response();
    }

    state.audit.log_with_app(
        &user_id_str,
        "schema_update",
        "schema",
        &slug,
        None,
        Some(app.app.id),
    );
    auth::set_flash(&session, "success", "Schema updated").await;
    Redirect::to(&format!("/apps/{}/schemas", app.app.slug)).into_response()
}

async fn delete_schema(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, slug)): Path<(String, String)>,
) -> impl IntoResponse {
    if !auth::has_min_role(&role.0, "admin") {
        return axum::http::StatusCode::FORBIDDEN;
    }
    let user_id_str = user.id.to_string();
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let _ = schema::delete_schema(&schemas_dir, &slug);
    state.audit.log_with_app(
        &user_id_str,
        "schema_delete",
        "schema",
        &slug,
        None,
        Some(app.app.id),
    );
    auth::set_flash(&session, "success", "Schema deleted").await;
    axum::http::StatusCode::NO_CONTENT
}

async fn render_schema_edit(
    state: &AppState,
    session: &Session,
    app: &AppContext,
    is_htmx: bool,
    is_new: bool,
    schema_json: &str,
    error: &str,
    user_role: &str,
    current_username: &str,
) -> axum::response::Result<Html<String>> {
    let csrf_token = auth::ensure_csrf_token(session).await;
    let ath_csrf = auth::ath_csrf(session).await;
    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("schemas/edit.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            ath_csrf => ath_csrf,
            user_role => user_role,
            current_username => current_username,
            app => app.template_context(),
            nav_schemas => app.nav_schemas(&state.config),
            is_new => is_new,
            schema_json => schema_json,
            error => error,
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html))
}
