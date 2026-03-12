use axum::{
    Form, Router,
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect},
    routing::get,
};
use axum_htmx::HxRequest;
use tower_sessions::Session;

use crate::auth;
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
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
) -> axum::response::Result<Html<String>> {
    let schemas = schema::list_schemas(&state.config.schemas_dir())
        .map_err(|e| format!("Failed to list schemas: {e}"))?;

    let schema_data: Vec<minijinja::Value> = schemas
        .iter()
        .map(|s| {
            minijinja::context! {
                title => s.meta.title,
                slug => s.meta.slug,
                storage => s.meta.storage.to_string(),
                field_count => schema::property_count(&s.schema),
            }
        })
        .collect();

    let csrf_token = auth::ensure_csrf_token(&session).await;
    let flash = auth::take_flash(&session).await;
    let tmpl = state.templates.acquire_env().map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("schemas/list.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            schemas => schema_data,
            flash_kind => flash.as_ref().map(|(k, _)| k.as_str()),
            flash_message => flash.as_ref().map(|(_, m)| m.as_str()),
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html))
}

async fn new_schema_page(
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
) -> axum::response::Result<Html<String>> {
    let csrf_token = auth::ensure_csrf_token(&session).await;
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

    let tmpl = state.templates.acquire_env().map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("schemas/edit.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
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
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<SchemaForm>,
) -> impl IntoResponse {
    let user_id = auth::current_user_id(&session).await.unwrap_or(0);
    let schema_value: serde_json::Value = match serde_json::from_str(&form.schema_json) {
        Ok(v) => v,
        Err(e) => {
            return render_schema_edit(
                &state,
                true,
                &form.schema_json,
                &format!("Invalid JSON: {e}"),
            )
            .await
            .into_response();
        }
    };

    if let Err(e) = schema::validate_schema(&schema_value) {
        return render_schema_edit(&state, true, &form.schema_json, &format!("{e}"))
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
                true,
                &form.schema_json,
                "Schema must have a title and slug in x-substrukt",
            )
            .await
            .into_response();
        }
    };

    if let Err(e) = schema::save_schema(&state.config.schemas_dir(), &slug, &schema_value) {
        return render_schema_edit(&state, true, &form.schema_json, &format!("Save error: {e}"))
            .await
            .into_response();
    }

    state.audit.log(&user_id.to_string(), "schema_create", "schema", &slug, None);
    auth::set_flash(&session, "success", "Schema created").await;
    Redirect::to("/schemas").into_response()
}

async fn edit_schema_page(
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    Path(slug): Path<String>,
) -> axum::response::Result<impl IntoResponse> {
    let csrf_token = auth::ensure_csrf_token(&session).await;
    let schema = schema::get_schema(&state.config.schemas_dir(), &slug)
        .map_err(|e| format!("Error: {e}"))?
        .ok_or("Schema not found")?;

    let tmpl = state.templates.acquire_env().map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("schemas/edit.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            is_new => false,
            slug => slug,
            schema_json => serde_json::to_string_pretty(&schema.schema).unwrap_or_default(),
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html))
}

async fn update_schema(
    State(state): State<AppState>,
    session: Session,
    Path(slug): Path<String>,
    Form(form): Form<SchemaForm>,
) -> impl IntoResponse {
    let user_id = auth::current_user_id(&session).await.unwrap_or(0);
    let schema_value: serde_json::Value = match serde_json::from_str(&form.schema_json) {
        Ok(v) => v,
        Err(e) => {
            return render_schema_edit(
                &state,
                false,
                &form.schema_json,
                &format!("Invalid JSON: {e}"),
            )
            .await
            .into_response();
        }
    };

    if let Err(e) = schema::validate_schema(&schema_value) {
        return render_schema_edit(&state, false, &form.schema_json, &format!("{e}"))
            .await
            .into_response();
    }

    if let Err(e) = schema::save_schema(&state.config.schemas_dir(), &slug, &schema_value) {
        return render_schema_edit(
            &state,
            false,
            &form.schema_json,
            &format!("Save error: {e}"),
        )
        .await
        .into_response();
    }

    state.audit.log(&user_id.to_string(), "schema_update", "schema", &slug, None);
    auth::set_flash(&session, "success", "Schema updated").await;
    Redirect::to("/schemas").into_response()
}

async fn delete_schema(
    State(state): State<AppState>,
    session: Session,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    let user_id = auth::current_user_id(&session).await.unwrap_or(0);
    let _ = schema::delete_schema(&state.config.schemas_dir(), &slug);
    state.audit.log(&user_id.to_string(), "schema_delete", "schema", &slug, None);
    axum::http::StatusCode::NO_CONTENT
}

async fn render_schema_edit(
    state: &AppState,
    is_new: bool,
    schema_json: &str,
    error: &str,
) -> axum::response::Result<Html<String>> {
    let tmpl = state.templates.acquire_env().map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("schemas/edit.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            is_new => is_new,
            schema_json => schema_json,
            error => error,
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html))
}
