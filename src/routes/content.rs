use axum::{
    Extension, Router,
    extract::{Multipart, Path, Query, State},
    response::{Html, IntoResponse, Redirect},
    routing::get,
};
use axum_htmx::HxRequest;
use tower_sessions::Session;

use crate::app_context::AppContext;
use crate::auth;
use crate::content::form::ReferenceOptions;
use crate::content::{self, form as content_form};
use crate::schema;
use crate::schema::models::Kind;
use crate::state::{AppState, ContentCache};
use crate::templates::base_for_htmx;
use crate::uploads;

const PAGE_SIZE: usize = 50;

#[derive(serde::Deserialize, Default)]
pub struct ListParams {
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub page: Option<u32>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub order: Option<String>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/{schema_slug}", get(list_entries))
        .route("/{schema_slug}/new", get(new_entry_page).post(create_entry))
        .route("/{schema_slug}/{entry_id}/edit", get(edit_entry_page))
        .route(
            "/{schema_slug}/{entry_id}/publish",
            axum::routing::post(publish_entry),
        )
        .route(
            "/{schema_slug}/{entry_id}/unpublish",
            axum::routing::post(unpublish_entry),
        )
        .route(
            "/{schema_slug}/{entry_id}",
            axum::routing::post(update_entry).delete(delete_entry),
        )
        .route(
            "/{schema_slug}/{entry_id}/delete",
            axum::routing::post(delete_entry_post),
        )
        .route(
            "/{schema_slug}/_bulk/publish",
            axum::routing::post(bulk_publish),
        )
        .route(
            "/{schema_slug}/_bulk/unpublish",
            axum::routing::post(bulk_unpublish),
        )
        .route(
            "/{schema_slug}/_bulk/delete",
            axum::routing::post(bulk_delete),
        )
        .route("/{schema_slug}/{entry_id}/history", get(entry_history))
        .route("/{schema_slug}/{entry_id}/diff", get(entry_diff))
        .route(
            "/{schema_slug}/{entry_id}/revert/{timestamp}",
            axum::routing::post(revert_entry),
        )
}

/// Extract username string from allowthem User.
fn username_str(user: &allowthem_core::User) -> String {
    user.username
        .as_ref()
        .map(|u| u.as_str().to_string())
        .unwrap_or_default()
}

async fn list_entries(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, schema_slug)): Path<(String, String)>,
    Query(params): Query<ListParams>,
) -> axum::response::Result<axum::response::Response> {
    let csrf_token = auth::ensure_csrf_token(&session).await;
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let schema_file = schema::get_schema(&schemas_dir, &schema_slug)
        .map_err(|e| format!("Error: {e}"))?
        .ok_or("Schema not found")?;

    if schema_file.meta.kind == Kind::Single {
        return Ok(Redirect::to(&format!(
            "/apps/{}/content/{schema_slug}/_single/edit",
            app.app.slug
        ))
        .into_response());
    }

    let entries =
        content::list_entries(&content_dir, &schema_file).map_err(|e| format!("Error: {e}"))?;

    let total = entries.len();
    let q = params.q.trim().to_string();
    let status_filter = params.status.as_deref().unwrap_or("all").to_string();
    let sort_field = params.sort.clone().unwrap_or_default();
    let sort_order = params.order.as_deref().unwrap_or("asc").to_string();
    let page = params.page.unwrap_or(1).max(1) as usize;

    let query_params = content::QueryParams {
        status: status_filter.clone(),
        q: q.clone(),
        filters: vec![],
        sort_field: if sort_field.is_empty() {
            "_id".into()
        } else {
            sort_field.clone()
        },
        sort_order: if sort_order == "desc" {
            content::SortOrder::Desc
        } else {
            content::SortOrder::Asc
        },
        offset: (page - 1) * PAGE_SIZE,
        limit: Some(PAGE_SIZE),
    };
    let result = content::query_entries(entries, &query_params);
    let filtered = result.total;
    let total_pages = (filtered + PAGE_SIZE - 1) / PAGE_SIZE.max(1);
    let entries = result.entries;
    let has_prev = page > 1;
    let has_next = page < total_pages;

    let columns = get_display_columns(&schema_file.schema);

    let entry_data: Vec<minijinja::Value> = entries
        .iter()
        .map(|e| {
            let cols: Vec<minijinja::Value> = columns
                .iter()
                .map(|(key, _)| {
                    let val = e
                        .data
                        .get(key)
                        .map(|v| match v {
                            serde_json::Value::String(s) => {
                                if s.len() > 100 {
                                    format!("{}…", &s[..s.floor_char_boundary(100)])
                                } else {
                                    s.clone()
                                }
                            }
                            serde_json::Value::Bool(b) => b.to_string(),
                            serde_json::Value::Number(n) => n.to_string(),
                            _ => v.to_string(),
                        })
                        .unwrap_or_default();
                    minijinja::Value::from(val)
                })
                .collect();
            let status = content::get_entry_status(&e.data);
            minijinja::context! {
                id => e.id,
                columns => cols,
                status => status,
            }
        })
        .collect();

    let column_headers: Vec<&str> = columns.iter().map(|(_, label)| label.as_str()).collect();
    let column_keys: Vec<&str> = columns.iter().map(|(key, _)| key.as_str()).collect();

    let user_role = &role.0;
    let current_username = username_str(&user);
    let flash = auth::take_flash(&session).await;
    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("content/list.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            user_role => user_role,
            current_username => current_username,
            app => app.template_context(),
            nav_schemas => app.nav_schemas(&state.config),
            schema_title => schema_file.meta.title,
            schema_slug => schema_slug,
            columns => column_headers,
            column_keys => column_keys,
            entries => entry_data,
            q => q,
            status_filter => status_filter,
            sort_field => sort_field,
            sort_order => sort_order,
            total => total,
            filtered => filtered,
            page => page,
            has_prev => has_prev,
            has_next => has_next,
            flash_kind => flash.as_ref().map(|(k, _)| k.as_str()),
            flash_message => flash.as_ref().map(|(_, m)| m.as_str()),
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html).into_response())
}

fn get_display_columns(schema: &serde_json::Value) -> Vec<(String, String)> {
    let mut columns = Vec::new();
    if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
        for (key, val) in props {
            if key == "_id" {
                continue;
            }
            let field_type = val.get("type").and_then(|t| t.as_str()).unwrap_or("");
            let format = val.get("format").and_then(|f| f.as_str());
            if matches!(field_type, "string" | "number" | "integer" | "boolean")
                && !matches!(
                    format,
                    Some("upload") | Some("markdown") | Some("reference")
                )
            {
                let label = val
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or(key)
                    .to_string();
                columns.push((key.clone(), label));
                if columns.len() >= 4 {
                    break;
                }
            }
        }
    }
    columns
}

/// Extract a display title from entry data using the schema's first required string field,
/// or falling back to the first string property.
fn extract_entry_title(data: &serde_json::Value, schema: &serde_json::Value) -> Option<String> {
    let obj = data.as_object()?;
    let props = schema.get("properties")?.as_object()?;
    let required: Vec<&str> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    // Try required string fields first
    for key in &required {
        if *key == "_id" || *key == "_status" {
            continue;
        }
        let is_string = props
            .get(*key)
            .and_then(|p| p.get("type"))
            .and_then(|t| t.as_str())
            == Some("string");
        let is_upload = props
            .get(*key)
            .and_then(|p| p.get("format"))
            .and_then(|f| f.as_str())
            == Some("upload");
        if is_string && !is_upload {
            if let Some(serde_json::Value::String(s)) = obj.get(*key) {
                if !s.is_empty() {
                    return Some(s.clone());
                }
            }
        }
    }

    // Fall back to first non-empty string property
    for (key, prop) in props {
        if key == "_id" || key == "_status" {
            continue;
        }
        let is_string = prop.get("type").and_then(|t| t.as_str()) == Some("string");
        let is_upload = prop.get("format").and_then(|f| f.as_str()) == Some("upload");
        if is_string && !is_upload {
            if let Some(serde_json::Value::String(s)) = obj.get(key) {
                if !s.is_empty() {
                    return Some(s.clone());
                }
            }
        }
    }

    None
}

fn build_reference_options(
    schema: &serde_json::Value,
    cache: &ContentCache,
    prefix: &str,
    app_slug: &str,
) -> ReferenceOptions {
    let mut opts = ReferenceOptions::new();
    build_reference_options_inner(schema, cache, prefix, app_slug, &mut opts, 0);
    opts
}

fn build_reference_options_inner(
    schema: &serde_json::Value,
    cache: &ContentCache,
    prefix: &str,
    app_slug: &str,
    opts: &mut ReferenceOptions,
    depth: usize,
) {
    if depth > 32 {
        return;
    }
    let Some(props) = schema.get("properties").and_then(|p| p.as_object()) else {
        return;
    };
    for (key, prop) in props {
        if key == "_id" {
            continue;
        }
        let field_name = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        let field_type = prop.get("type").and_then(|t| t.as_str());
        let format = prop.get("format").and_then(|f| f.as_str());

        if field_type == Some("string") && format == Some("reference") {
            let target_slug = prop
                .get("x-substrukt-reference")
                .and_then(|r| r.get("schema"))
                .and_then(|s| s.as_str());
            let Some(target_slug) = target_slug else {
                continue;
            };
            let label_field = prop
                .get("x-substrukt-reference")
                .and_then(|r| r.get("label_field"))
                .and_then(|l| l.as_str());
            let target_prefix = format!("{app_slug}/{target_slug}/");
            let mut entries: Vec<(String, String)> = cache
                .iter()
                .filter(|entry| entry.key().starts_with(&target_prefix))
                .map(|entry| {
                    let id = entry
                        .key()
                        .strip_prefix(&target_prefix)
                        .unwrap_or(entry.key())
                        .to_string();
                    let label = if let Some(lf) = label_field {
                        entry
                            .value()
                            .get(lf)
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    } else {
                        None
                    }
                    .or_else(|| {
                        entry.value().as_object().and_then(|obj| {
                            obj.iter()
                                .find(|(k, v)| !k.starts_with('_') && v.is_string())
                                .and_then(|(_, v)| v.as_str())
                                .map(|s| s.to_string())
                        })
                    })
                    .unwrap_or_else(|| id.clone());
                    (id, label)
                })
                .collect();
            entries.sort_by(|a, b| a.1.cmp(&b.1));
            opts.insert(field_name, entries);
        } else if field_type == Some("object") {
            build_reference_options_inner(prop, cache, &field_name, app_slug, opts, depth + 1);
        } else if field_type == Some("array") {
            if let Some(items) = prop.get("items") {
                build_reference_options_inner(items, cache, &field_name, app_slug, opts, depth + 1);
            }
        }
    }
}

fn warn_dangling_references(
    data: &serde_json::Value,
    schema: &serde_json::Value,
    cache: &ContentCache,
    app_slug: &str,
) {
    warn_dangling_references_inner(data, schema, cache, app_slug, 0);
}

fn warn_dangling_references_inner(
    data: &serde_json::Value,
    schema: &serde_json::Value,
    cache: &ContentCache,
    app_slug: &str,
    depth: usize,
) {
    if depth > 32 {
        return;
    }
    let Some(props) = schema.get("properties").and_then(|p| p.as_object()) else {
        return;
    };
    let Some(obj) = data.as_object() else {
        return;
    };
    for (key, prop) in props {
        let field_type = prop.get("type").and_then(|t| t.as_str());
        let format = prop.get("format").and_then(|f| f.as_str());

        if field_type == Some("string") && format == Some("reference") {
            let Some(target_slug) = prop
                .get("x-substrukt-reference")
                .and_then(|r| r.get("schema"))
                .and_then(|s| s.as_str())
            else {
                continue;
            };
            if let Some(serde_json::Value::String(ref_id)) = obj.get(key) {
                if !ref_id.is_empty() {
                    let cache_key = format!("{app_slug}/{target_slug}/{ref_id}");
                    if cache.get(&cache_key).is_none() {
                        tracing::warn!(
                            field = key,
                            reference_id = ref_id,
                            target_schema = target_slug,
                            "Dangling reference: target entry not found in cache"
                        );
                    }
                }
            }
        } else if field_type == Some("object") {
            if let Some(nested) = obj.get(key) {
                warn_dangling_references_inner(nested, prop, cache, app_slug, depth + 1);
            }
        } else if field_type == Some("array") {
            if let Some(items_schema) = prop.get("items")
                && let Some(serde_json::Value::Array(arr)) = obj.get(key)
            {
                for item in arr {
                    warn_dangling_references_inner(item, items_schema, cache, app_slug, depth + 1);
                }
            }
        }
    }
}

async fn new_entry_page(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, schema_slug)): Path<(String, String)>,
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
    let current_username = username_str(&user);
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let schema_file = schema::get_schema(&schemas_dir, &schema_slug)
        .map_err(|e| format!("Error: {e}"))?
        .ok_or("Schema not found")?;

    if schema_file.meta.kind == Kind::Single {
        return Ok(Redirect::to(&format!(
            "/apps/{}/content/{schema_slug}/_single/edit",
            app.app.slug
        ))
        .into_response());
    }

    let ref_options = build_reference_options(&schema_file.schema, &state.cache, "", &app.app.slug);
    let form_html = content_form::render_form_fields(
        &schema_file.schema,
        None,
        "",
        &ref_options,
        &app.app.slug,
    );

    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("content/edit.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            user_role => user_role,
            current_username => current_username,
            app => app.template_context(),
            nav_schemas => app.nav_schemas(&state.config),
            schema_title => schema_file.meta.title,
            schema_slug => schema_slug,
            is_new => true,
            form_fields => form_html,
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html).into_response())
}

async fn edit_entry_page(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, schema_slug, entry_id)): Path<(String, String, String)>,
) -> axum::response::Result<Html<String>> {
    let csrf_token = auth::ensure_csrf_token(&session).await;
    let flash = auth::take_flash(&session).await;
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let schema_file = schema::get_schema(&schemas_dir, &schema_slug)
        .map_err(|e| format!("Error: {e}"))?
        .ok_or("Schema not found")?;

    let entry = content::get_entry(&content_dir, &schema_file, &entry_id)
        .map_err(|e| format!("Error: {e}"))?;

    let (existing_data, is_new) = if let Some(entry) = entry {
        (Some(entry.data), false)
    } else if entry_id == "_single" && schema_file.meta.kind == Kind::Single {
        (None, true)
    } else {
        return Err("Entry not found".into());
    };

    let entry_status = existing_data
        .as_ref()
        .map(|d| content::get_entry_status(d).to_string())
        .unwrap_or_else(|| "draft".to_string());

    let entry_title = existing_data
        .as_ref()
        .and_then(|d| extract_entry_title(d, &schema_file.schema));

    let ref_options = build_reference_options(&schema_file.schema, &state.cache, "", &app.app.slug);
    let form_html = content_form::render_form_fields(
        &schema_file.schema,
        existing_data.as_ref(),
        "",
        &ref_options,
        &app.app.slug,
    );

    let user_role = &role.0;
    let current_username = username_str(&user);
    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("content/edit.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let is_single = schema_file.meta.kind == Kind::Single;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            user_role => user_role,
            current_username => current_username,
            app => app.template_context(),
            nav_schemas => app.nav_schemas(&state.config),
            schema_title => schema_file.meta.title,
            schema_slug => schema_slug,
            entry_id => entry_id,
            entry_title => entry_title,
            is_new => is_new,
            is_single => is_single,
            form_fields => form_html,
            entry_status => entry_status,
            flash_kind => flash.as_ref().map(|(k, _)| k.as_str()),
            flash_message => flash.as_ref().map(|(_, m)| m.as_str()),
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html))
}

async fn create_entry(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, schema_slug)): Path<(String, String)>,
    multipart: Multipart,
) -> impl IntoResponse {
    if !auth::has_min_role(&role.0, "editor") {
        return (
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into_response();
    }
    let user_id_str = user.id.to_string();
    let user_role = role.0.clone();
    let current_username = username_str(&user);
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        _ => {
            return Redirect::to(&format!("/apps/{}/schemas", app.app.slug)).into_response();
        }
    };

    if schema_file.meta.kind == Kind::Single {
        return Redirect::to(&format!(
            "/apps/{}/content/{schema_slug}/_single/edit",
            app.app.slug
        ))
        .into_response();
    }

    let (form_fields, upload_fields) = match parse_multipart(multipart).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Multipart parse error: {e}");
            return Redirect::to(&format!("/apps/{}/content/{schema_slug}/new", app.app.slug))
                .into_response();
        }
    };

    // Verify CSRF token from multipart form fields
    let csrf_value = form_fields
        .iter()
        .find(|(k, _)| k == "_csrf")
        .map(|(_, v)| v.as_str());
    if !matches!(csrf_value, Some(token) if auth::verify_csrf_token(&session, token).await) {
        return (axum::http::StatusCode::FORBIDDEN, "Invalid CSRF token").into_response();
    }

    let mut data = content_form::form_data_to_json(&schema_file.schema, &form_fields, "");

    // Process upload fields
    process_uploads(&state, &app, &mut data, &upload_fields).await;

    warn_dangling_references(&data, &schema_file.schema, &state.cache, &app.app.slug);

    // Validate
    let target_status = content::resolve_target_status(&data, &content_dir, &schema_file, None);
    let ctx = content::ValidationContext {
        entry_id: None,
        target_status: &target_status,
        cache: &state.cache,
        app_slug: &app.app.slug,
        schema_slug: &schema_slug,
    };
    if let Err(validation_errors) = content::validate_content(&schema_file, &data, &ctx) {
        let errors: Vec<String> = validation_errors.iter().map(|e| e.to_string()).collect();
        let csrf_token = auth::ensure_csrf_token(&session).await;
        let ref_options =
            build_reference_options(&schema_file.schema, &state.cache, "", &app.app.slug);
        let form_html = content_form::render_form_fields(
            &schema_file.schema,
            Some(&data),
            "",
            &ref_options,
            &app.app.slug,
        );
        if let Ok(tmpl) = state.templates.acquire_env()
            && let Ok(template) = tmpl.get_template("content/edit.html")
            && let Ok(html) = template.render(minijinja::context! {
                base_template => base_for_htmx(is_htmx),
                csrf_token => csrf_token,
                user_role => user_role,
                current_username => current_username,
                app => app.template_context(),
                nav_schemas => app.nav_schemas(&state.config),
                schema_title => schema_file.meta.title,
                schema_slug => schema_slug,
                is_new => true,
                form_fields => form_html,
                errors => errors,
            })
        {
            return Html(html).into_response();
        }
        return Redirect::to(&format!("/apps/{}/content/{schema_slug}/new", app.app.slug))
            .into_response();
    }

    let hashes = uploads::extract_upload_hashes(&data);
    match content::save_entry(&content_dir, &schema_file, None, data) {
        Ok(id) => {
            crate::cache::reload_entry(
                &state.cache,
                &state.etag_cache,
                &content_dir,
                &schema_file,
                &id,
                &app.app.slug,
            );
            let _ =
                uploads::db_update_references(&state.pool, app.app.id, &schema_slug, &id, &hashes)
                    .await;
            state.audit.log_with_app(
                &user_id_str,
                "content_create",
                "content",
                &format!("{schema_slug}/{id}"),
                None,
                Some(app.app.id),
            );
            auth::set_flash(&session, "success", "Entry created").await;
            Redirect::to(&format!("/apps/{}/content/{schema_slug}", app.app.slug)).into_response()
        }
        Err(e) => {
            tracing::error!("Save error: {e}");
            Redirect::to(&format!("/apps/{}/content/{schema_slug}/new", app.app.slug))
                .into_response()
        }
    }
}

async fn update_entry(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, schema_slug, entry_id)): Path<(String, String, String)>,
    multipart: Multipart,
) -> impl IntoResponse {
    if !auth::has_min_role(&role.0, "editor") {
        return (
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into_response();
    }
    let user_id_str = user.id.to_string();
    let user_role = role.0.clone();
    let current_username = username_str(&user);
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let app_dir = state.config.app_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        _ => {
            return Redirect::to(&format!("/apps/{}/schemas", app.app.slug)).into_response();
        }
    };

    let (form_fields, upload_fields) = match parse_multipart(multipart).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Multipart parse error: {e}");
            return Redirect::to(&format!(
                "/apps/{}/content/{schema_slug}/{entry_id}/edit",
                app.app.slug
            ))
            .into_response();
        }
    };

    // Verify CSRF token from multipart form fields
    let csrf_value = form_fields
        .iter()
        .find(|(k, _)| k == "_csrf")
        .map(|(_, v)| v.as_str());
    if !matches!(csrf_value, Some(token) if auth::verify_csrf_token(&session, token).await) {
        return (axum::http::StatusCode::FORBIDDEN, "Invalid CSRF token").into_response();
    }

    let mut data = content_form::form_data_to_json(&schema_file.schema, &form_fields, "");

    process_uploads(&state, &app, &mut data, &upload_fields).await;

    warn_dangling_references(&data, &schema_file.schema, &state.cache, &app.app.slug);

    let target_status =
        content::resolve_target_status(&data, &content_dir, &schema_file, Some(&entry_id));
    let ctx = content::ValidationContext {
        entry_id: Some(&entry_id),
        target_status: &target_status,
        cache: &state.cache,
        app_slug: &app.app.slug,
        schema_slug: &schema_slug,
    };
    if let Err(validation_errors) = content::validate_content(&schema_file, &data, &ctx) {
        let errors: Vec<String> = validation_errors.iter().map(|e| e.to_string()).collect();
        let csrf_token = auth::ensure_csrf_token(&session).await;
        let ref_options =
            build_reference_options(&schema_file.schema, &state.cache, "", &app.app.slug);
        let form_html = content_form::render_form_fields(
            &schema_file.schema,
            Some(&data),
            "",
            &ref_options,
            &app.app.slug,
        );
        if let Ok(tmpl) = state.templates.acquire_env()
            && let Ok(template) = tmpl.get_template("content/edit.html")
            && let Ok(html) = template.render(minijinja::context! {
                base_template => base_for_htmx(is_htmx),
                csrf_token => csrf_token,
                user_role => user_role,
                current_username => current_username,
                app => app.template_context(),
                nav_schemas => app.nav_schemas(&state.config),
                schema_title => schema_file.meta.title,
                schema_slug => schema_slug,
                entry_id => entry_id,
                is_new => false,
                form_fields => form_html,
                errors => errors,
            })
        {
            return Html(html).into_response();
        }
        return Redirect::to(&format!(
            "/apps/{}/content/{schema_slug}/{entry_id}/edit",
            app.app.slug
        ))
        .into_response();
    }

    // Snapshot current version for history
    if let Ok(Some(current)) = content::get_entry(&content_dir, &schema_file, &entry_id) {
        let snap_meta = crate::history::SnapshotMeta {
            user_id: user_id_str.clone(),
            username: current_username.clone(),
            source: crate::history::SnapshotSource::AdminUi,
        };
        if let Err(e) = crate::history::snapshot_entry(
            &app_dir,
            &schema_slug,
            &entry_id,
            &current.data,
            state.config.version_history_count,
            Some(&snap_meta),
        ) {
            tracing::warn!("Failed to snapshot version: {e}");
        }
    }

    let hashes = uploads::extract_upload_hashes(&data);
    match content::save_entry(&content_dir, &schema_file, Some(&entry_id), data) {
        Ok(_) => {
            crate::cache::reload_entry(
                &state.cache,
                &state.etag_cache,
                &content_dir,
                &schema_file,
                &entry_id,
                &app.app.slug,
            );
            let _ = uploads::db_update_references(
                &state.pool,
                app.app.id,
                &schema_slug,
                &entry_id,
                &hashes,
            )
            .await;
            state.audit.log_with_app(
                &user_id_str,
                "content_update",
                "content",
                &format!("{schema_slug}/{entry_id}"),
                None,
                Some(app.app.id),
            );
            auth::set_flash(&session, "success", "Entry updated").await;
            let redirect_url = if schema_file.meta.kind == Kind::Single {
                format!("/apps/{}/content/{schema_slug}/_single/edit", app.app.slug)
            } else {
                format!("/apps/{}/content/{schema_slug}", app.app.slug)
            };
            Redirect::to(&redirect_url).into_response()
        }
        Err(e) => {
            tracing::error!("Save error: {e}");
            Redirect::to(&format!(
                "/apps/{}/content/{schema_slug}/{entry_id}/edit",
                app.app.slug
            ))
            .into_response()
        }
    }
}

async fn delete_entry(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, schema_slug, entry_id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    if !auth::has_min_role(&role.0, "editor") {
        return axum::http::StatusCode::FORBIDDEN;
    }
    let user_id_str = user.id.to_string();
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let app_dir = state.config.app_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        _ => return axum::http::StatusCode::NOT_FOUND,
    };

    let _ = uploads::db_delete_references(&state.pool, app.app.id, &schema_slug, &entry_id).await;
    let _ = content::delete_entry(&content_dir, &schema_file, &entry_id);
    crate::history::delete_history(&app_dir, &schema_slug, &entry_id);
    let key = format!("{}/{schema_slug}/{entry_id}", app.app.slug);
    state.cache.remove(&key);

    state.audit.log_with_app(
        &user_id_str,
        "content_delete",
        "content",
        &format!("{schema_slug}/{entry_id}"),
        None,
        Some(app.app.id),
    );

    auth::set_flash(&session, "success", "Entry deleted").await;
    axum::http::StatusCode::NO_CONTENT
}

async fn delete_entry_post(
    user_ext: Extension<allowthem_core::User>,
    role_ext: Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((app_slug, schema_slug, entry_id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let redirect_url = format!("/apps/{app_slug}/content/{schema_slug}");
    delete_entry(
        user_ext,
        role_ext,
        State(state),
        session,
        app,
        Path((app_slug, schema_slug, entry_id)),
    )
    .await;
    Redirect::to(&redirect_url).into_response()
}

async fn publish_entry(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, schema_slug, entry_id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    if !auth::has_min_role(&role.0, "editor") {
        return (
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into_response();
    }
    let user_id_str = user.id.to_string();
    let user_role = role.0.clone();
    let current_username = username_str(&user);
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        _ => {
            auth::set_flash(&session, "error", "Schema not found").await;
            return Redirect::to(&format!("/apps/{}", app.app.slug)).into_response();
        }
    };

    if let Ok(Some(entry)) = content::get_entry(&content_dir, &schema_file, &entry_id) {
        let ctx = content::ValidationContext {
            entry_id: Some(&entry_id),
            target_status: "published",
            cache: &state.cache,
            app_slug: &app.app.slug,
            schema_slug: &schema_slug,
        };
        if let Err(errors) = content::validate_for_publish(&schema_file, &entry.data, &ctx) {
            let msg = errors
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ");
            auth::set_flash(&session, "error", &format!("Cannot publish: {msg}")).await;
            return Redirect::to(&format!(
                "/apps/{}/content/{schema_slug}/{entry_id}/edit",
                app.app.slug
            ))
            .into_response();
        }
    }

    if let Err(e) = content::set_entry_status(&content_dir, &schema_file, &entry_id, "published") {
        tracing::error!("Publish failed: {e}");
        auth::set_flash(&session, "error", "Failed to publish entry").await;
        return Redirect::to(&format!(
            "/apps/{}/content/{schema_slug}/{entry_id}/edit",
            app.app.slug
        ))
        .into_response();
    }

    crate::cache::reload_entry(
        &state.cache,
        &state.etag_cache,
        &content_dir,
        &schema_file,
        &entry_id,
        &app.app.slug,
    );

    state.audit.log_with_app(
        &user_id_str,
        "entry_published",
        "content",
        &format!("{schema_slug}/{entry_id}"),
        None,
        Some(app.app.id),
    );

    if is_htmx {
        let csrf_token = auth::ensure_csrf_token(&session).await;
        let tmpl = state
            .templates
            .acquire_env()
            .map_err(|e| format!("Template env error: {e}"))
            .unwrap();
        let template = tmpl
            .get_template("content/_status_control.html")
            .map_err(|e| format!("Template error: {e}"))
            .unwrap();
        let html = template
            .render(minijinja::context! {
                csrf_token => csrf_token,
                user_role => user_role,
                current_username => current_username,
                app => app.template_context(),
                schema_slug => schema_slug,
                entry_id => entry_id,
                entry_status => "published",
            })
            .map_err(|e| format!("Render error: {e}"))
            .unwrap();
        return Html(html).into_response();
    }

    auth::set_flash(&session, "success", "Entry published").await;
    Redirect::to(&format!(
        "/apps/{}/content/{schema_slug}/{entry_id}/edit",
        app.app.slug
    ))
    .into_response()
}

async fn unpublish_entry(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, schema_slug, entry_id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    if !auth::has_min_role(&role.0, "editor") {
        return (
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into_response();
    }
    let user_id_str = user.id.to_string();
    let user_role = role.0.clone();
    let current_username = username_str(&user);
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        _ => {
            auth::set_flash(&session, "error", "Schema not found").await;
            return Redirect::to(&format!("/apps/{}", app.app.slug)).into_response();
        }
    };

    if let Err(e) = content::set_entry_status(&content_dir, &schema_file, &entry_id, "draft") {
        tracing::error!("Unpublish failed: {e}");
        auth::set_flash(&session, "error", "Failed to unpublish entry").await;
        return Redirect::to(&format!(
            "/apps/{}/content/{schema_slug}/{entry_id}/edit",
            app.app.slug
        ))
        .into_response();
    }

    crate::cache::reload_entry(
        &state.cache,
        &state.etag_cache,
        &content_dir,
        &schema_file,
        &entry_id,
        &app.app.slug,
    );

    state.audit.log_with_app(
        &user_id_str,
        "entry_unpublished",
        "content",
        &format!("{schema_slug}/{entry_id}"),
        None,
        Some(app.app.id),
    );

    if is_htmx {
        let csrf_token = auth::ensure_csrf_token(&session).await;
        let tmpl = state
            .templates
            .acquire_env()
            .map_err(|e| format!("Template env error: {e}"))
            .unwrap();
        let template = tmpl
            .get_template("content/_status_control.html")
            .map_err(|e| format!("Template error: {e}"))
            .unwrap();
        let html = template
            .render(minijinja::context! {
                csrf_token => csrf_token,
                user_role => user_role,
                current_username => current_username,
                app => app.template_context(),
                schema_slug => schema_slug,
                entry_id => entry_id,
                entry_status => "draft",
            })
            .map_err(|e| format!("Render error: {e}"))
            .unwrap();
        return Html(html).into_response();
    }

    auth::set_flash(&session, "success", "Entry unpublished").await;
    Redirect::to(&format!(
        "/apps/{}/content/{schema_slug}/{entry_id}/edit",
        app.app.slug
    ))
    .into_response()
}

struct UploadField {
    field_name: String,
    filename: String,
    content_type: String,
    data: Vec<u8>,
}

async fn parse_multipart(
    mut multipart: Multipart,
) -> eyre::Result<(Vec<(String, String)>, Vec<UploadField>)> {
    let mut form_fields = Vec::new();
    let mut upload_fields = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| eyre::eyre!("{e}"))?
    {
        let name = field.name().unwrap_or("").to_string();
        let filename = field.file_name().map(|s| s.to_string());
        let content_type = field
            .content_type()
            .unwrap_or("application/octet-stream")
            .to_string();

        let data = field.bytes().await.map_err(|e| eyre::eyre!("{e}"))?;

        if let Some(filename) = filename {
            if !data.is_empty() && !filename.is_empty() {
                upload_fields.push(UploadField {
                    field_name: name,
                    filename,
                    content_type,
                    data: data.to_vec(),
                });
            } else {
                // Empty file input -- treated as text field
                form_fields.push((name, String::from_utf8_lossy(&data).to_string()));
            }
        } else {
            form_fields.push((name, String::from_utf8_lossy(&data).to_string()));
        }
    }

    Ok((form_fields, upload_fields))
}

async fn process_uploads(
    state: &AppState,
    app: &AppContext,
    data: &mut serde_json::Value,
    upload_fields: &[UploadField],
) {
    let uploads_dir = state.config.app_uploads_dir(&app.app.slug);
    for upload in upload_fields {
        match uploads::store_upload(
            &uploads_dir,
            &state.pool,
            app.app.id,
            &upload.filename,
            &upload.content_type,
            &upload.data,
        )
        .await
        {
            Ok(meta) => {
                let upload_ref = serde_json::json!({
                    "hash": meta.hash,
                    "filename": meta.filename,
                    "mime": meta.mime,
                });
                // Set the field in data
                set_nested_field(data, &upload.field_name, upload_ref);
            }
            Err(e) => {
                tracing::error!("Upload error for {}: {e}", upload.field_name);
            }
        }
    }
}

fn set_nested_field(data: &mut serde_json::Value, path: &str, value: serde_json::Value) {
    if let Some(obj) = data.as_object_mut() {
        // Handle simple field names (no dots)
        if !path.contains('.') {
            obj.insert(path.to_string(), value);
        } else {
            let parts: Vec<&str> = path.splitn(2, '.').collect();
            if parts.len() == 2 {
                let entry = obj
                    .entry(parts[0].to_string())
                    .or_insert(serde_json::Value::Object(Default::default()));
                set_nested_field(entry, parts[1], value);
            }
        }
    }
}

#[derive(serde::Deserialize)]
struct BulkForm {
    ids: String,
}

async fn bulk_publish(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, schema_slug)): Path<(String, String)>,
    axum::extract::Form(form): axum::extract::Form<BulkForm>,
) -> impl IntoResponse {
    if !auth::has_min_role(&role.0, "editor") {
        return Redirect::to(&format!("/apps/{}/content/{schema_slug}", app.app.slug))
            .into_response();
    }
    let user_id_str = user.id.to_string();
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        _ => return Redirect::to(&format!("/apps/{}/schemas", app.app.slug)).into_response(),
    };
    let mut count = 0;
    let mut skipped = 0;
    for id in form
        .ids
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        if let Ok(Some(entry)) = content::get_entry(&content_dir, &schema_file, id) {
            let ctx = content::ValidationContext {
                entry_id: Some(id),
                target_status: "published",
                cache: &state.cache,
                app_slug: &app.app.slug,
                schema_slug: &schema_slug,
            };
            if content::validate_for_publish(&schema_file, &entry.data, &ctx).is_err() {
                skipped += 1;
                continue;
            }
        }
        if content::set_entry_status(&content_dir, &schema_file, id, "published").is_ok() {
            crate::cache::reload_entry(
                &state.cache,
                &state.etag_cache,
                &content_dir,
                &schema_file,
                id,
                &app.app.slug,
            );
            state.audit.log_with_app(
                &user_id_str,
                "entry_published",
                "content",
                &format!("{schema_slug}/{id}"),
                None,
                Some(app.app.id),
            );
            count += 1;
        }
    }
    let msg = if skipped > 0 {
        format!("{count} entries published, {skipped} skipped (missing required fields)")
    } else {
        format!("{count} entries published")
    };
    auth::set_flash(&session, "success", &msg).await;
    Redirect::to(&format!("/apps/{}/content/{schema_slug}", app.app.slug)).into_response()
}

async fn bulk_unpublish(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, schema_slug)): Path<(String, String)>,
    axum::extract::Form(form): axum::extract::Form<BulkForm>,
) -> impl IntoResponse {
    if !auth::has_min_role(&role.0, "editor") {
        return Redirect::to(&format!("/apps/{}/content/{schema_slug}", app.app.slug))
            .into_response();
    }
    let user_id_str = user.id.to_string();
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        _ => return Redirect::to(&format!("/apps/{}/schemas", app.app.slug)).into_response(),
    };
    let mut count = 0;
    for id in form
        .ids
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        if content::set_entry_status(&content_dir, &schema_file, id, "draft").is_ok() {
            crate::cache::reload_entry(
                &state.cache,
                &state.etag_cache,
                &content_dir,
                &schema_file,
                id,
                &app.app.slug,
            );
            state.audit.log_with_app(
                &user_id_str,
                "entry_unpublished",
                "content",
                &format!("{schema_slug}/{id}"),
                None,
                Some(app.app.id),
            );
            count += 1;
        }
    }
    auth::set_flash(&session, "success", &format!("{count} entries unpublished")).await;
    Redirect::to(&format!("/apps/{}/content/{schema_slug}", app.app.slug)).into_response()
}

async fn bulk_delete(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, schema_slug)): Path<(String, String)>,
    axum::extract::Form(form): axum::extract::Form<BulkForm>,
) -> impl IntoResponse {
    if !auth::has_min_role(&role.0, "editor") {
        return Redirect::to(&format!("/apps/{}/content/{schema_slug}", app.app.slug))
            .into_response();
    }
    let user_id_str = user.id.to_string();
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let app_dir = state.config.app_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        _ => return Redirect::to(&format!("/apps/{}/schemas", app.app.slug)).into_response(),
    };
    let mut count = 0;
    for id in form
        .ids
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        let _ = uploads::db_delete_references(&state.pool, app.app.id, &schema_slug, id).await;
        let _ = content::delete_entry(&content_dir, &schema_file, id);
        crate::history::delete_history(&app_dir, &schema_slug, id);
        let key = format!("{}/{schema_slug}/{id}", app.app.slug);
        state.cache.remove(&key);
        state.audit.log_with_app(
            &user_id_str,
            "content_delete",
            "content",
            &format!("{schema_slug}/{id}"),
            None,
            Some(app.app.id),
        );
        count += 1;
    }
    auth::set_flash(&session, "success", &format!("{count} entries deleted")).await;
    Redirect::to(&format!("/apps/{}/content/{schema_slug}", app.app.slug)).into_response()
}

async fn entry_history(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, schema_slug, entry_id)): Path<(String, String, String)>,
) -> axum::response::Result<Html<String>> {
    let csrf_token = auth::ensure_csrf_token(&session).await;
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let app_dir = state.config.app_dir(&app.app.slug);
    let schema_file = schema::get_schema(&schemas_dir, &schema_slug)
        .map_err(|e| format!("Error: {e}"))?
        .ok_or("Schema not found")?;

    let versions = crate::history::list_versions(&app_dir, &schema_slug, &entry_id)
        .map_err(|e| format!("Error: {e}"))?;

    let version_data: Vec<minijinja::Value> = versions
        .iter()
        .map(|v| {
            let formatted_time = chrono::DateTime::from_timestamp_millis(v.timestamp as i64)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| v.timestamp.to_string());
            minijinja::context! {
                timestamp => v.timestamp,
                formatted_time => formatted_time,
                size => v.size,
                username => v.username.as_deref().unwrap_or("Unknown"),
                source => v.source.as_deref().unwrap_or(""),
            }
        })
        .collect();

    let user_role = &role.0;
    let current_username = username_str(&user);
    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("content/history.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            user_role => user_role,
            current_username => current_username,
            app => app.template_context(),
            nav_schemas => app.nav_schemas(&state.config),
            schema_title => schema_file.meta.title,
            schema_slug => schema_slug,
            entry_id => entry_id,
            versions => version_data,
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html))
}

#[derive(serde::Deserialize)]
struct DiffParams {
    from: u64,
    to: Option<String>,
}

async fn entry_diff(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, schema_slug, entry_id)): Path<(String, String, String)>,
    Query(params): Query<DiffParams>,
) -> axum::response::Result<Html<String>> {
    let csrf_token = auth::ensure_csrf_token(&session).await;
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let app_dir = state.config.app_dir(&app.app.slug);
    let schema_file = schema::get_schema(&schemas_dir, &schema_slug)
        .map_err(|e| format!("Error: {e}"))?
        .ok_or("Schema not found")?;

    let from_data = crate::history::get_version(&app_dir, &schema_slug, &entry_id, params.from)
        .map_err(|e| format!("Error: {e}"))?
        .ok_or("Version not found")?;

    let from_date = chrono::DateTime::from_timestamp_millis(params.from as i64)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| params.from.to_string());

    let (to_data, to_label) = if let Some(ref to_str) = params.to {
        if let Ok(ts) = to_str.parse::<u64>() {
            let data = crate::history::get_version(&app_dir, &schema_slug, &entry_id, ts)
                .map_err(|e| format!("Error: {e}"))?
                .ok_or("Target version not found")?;
            let label = chrono::DateTime::from_timestamp_millis(ts as i64)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| ts.to_string());
            (data, label)
        } else {
            let entry = content::get_entry(&content_dir, &schema_file, &entry_id)
                .map_err(|e| format!("Error: {e}"))?
                .ok_or("Entry not found")?;
            (entry.data, "Current".to_string())
        }
    } else {
        let entry = content::get_entry(&content_dir, &schema_file, &entry_id)
            .map_err(|e| format!("Error: {e}"))?
            .ok_or("Entry not found")?;
        (entry.data, "Current".to_string())
    };

    let diffs = crate::history::diff_entries(&from_data, &to_data, &schema_file.schema);

    let diff_data: Vec<minijinja::Value> = diffs
        .iter()
        .map(|d| {
            let (diff_type, old_val, new_val) = match &d.kind {
                crate::history::DiffKind::Changed { old, new } => {
                    ("changed", old.to_string(), new.to_string())
                }
                crate::history::DiffKind::Added { value } => {
                    ("added", String::new(), value.to_string())
                }
                crate::history::DiffKind::Removed { value } => {
                    ("removed", value.to_string(), String::new())
                }
            };
            minijinja::context! {
                path => d.path,
                label => d.label,
                diff_type => diff_type,
                old_val => old_val,
                new_val => new_val,
            }
        })
        .collect();

    let user_role = &role.0;
    let current_username = username_str(&user);
    let tmpl = state
        .templates
        .acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("content/diff.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            user_role => user_role,
            current_username => current_username,
            app => app.template_context(),
            nav_schemas => app.nav_schemas(&state.config),
            schema_title => schema_file.meta.title,
            schema_slug => schema_slug,
            entry_id => entry_id,
            from_date => from_date,
            to_label => to_label,
            diffs => diff_data,
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html))
}

async fn revert_entry(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Path((_app_slug, schema_slug, entry_id, timestamp)): Path<(String, String, String, u64)>,
    axum::extract::Form(form): axum::extract::Form<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    if !auth::has_min_role(&role.0, "editor") {
        return (
            axum::http::StatusCode::FORBIDDEN,
            "Insufficient permissions",
        )
            .into_response();
    }
    let user_id_str = user.id.to_string();
    // Verify CSRF
    let csrf_value = form.get("_csrf").map(|s| s.as_str());
    if !matches!(csrf_value, Some(token) if auth::verify_csrf_token(&session, token).await) {
        return (axum::http::StatusCode::FORBIDDEN, "Invalid CSRF token").into_response();
    }

    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let app_dir = state.config.app_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        _ => {
            return Redirect::to(&format!("/apps/{}/content/{schema_slug}", app.app.slug))
                .into_response();
        }
    };

    let version_data =
        match crate::history::get_version(&app_dir, &schema_slug, &entry_id, timestamp) {
            Ok(Some(data)) => data,
            _ => {
                auth::set_flash(&session, "error", "Version not found").await;
                return Redirect::to(&format!(
                    "/apps/{}/content/{schema_slug}/{entry_id}/history",
                    app.app.slug
                ))
                .into_response();
            }
        };

    // Snapshot current before reverting
    if let Ok(Some(current)) = content::get_entry(&content_dir, &schema_file, &entry_id) {
        let snap_meta = crate::history::SnapshotMeta {
            user_id: user_id_str.clone(),
            username: username_str(&user),
            source: crate::history::SnapshotSource::Revert,
        };
        if let Err(e) = crate::history::snapshot_entry(
            &app_dir,
            &schema_slug,
            &entry_id,
            &current.data,
            state.config.version_history_count,
            Some(&snap_meta),
        ) {
            tracing::warn!("Failed to snapshot version: {e}");
        }
    }

    match content::save_entry(&content_dir, &schema_file, Some(&entry_id), version_data) {
        Ok(_) => {
            crate::cache::reload_entry(
                &state.cache,
                &state.etag_cache,
                &content_dir,
                &schema_file,
                &entry_id,
                &app.app.slug,
            );
            state.audit.log_with_app(
                &user_id_str,
                "content_update",
                "content",
                &format!("{schema_slug}/{entry_id}"),
                Some(&format!("reverted to version {timestamp}")),
                Some(app.app.id),
            );
            auth::set_flash(&session, "success", "Entry reverted").await;
            Redirect::to(&format!(
                "/apps/{}/content/{schema_slug}/{entry_id}/edit",
                app.app.slug
            ))
            .into_response()
        }
        Err(e) => {
            tracing::error!("Revert error: {e}");
            auth::set_flash(&session, "error", "Failed to revert").await;
            Redirect::to(&format!(
                "/apps/{}/content/{schema_slug}/{entry_id}/history",
                app.app.slug
            ))
            .into_response()
        }
    }
}
