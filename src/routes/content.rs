use axum::{
    Router,
    extract::{Multipart, Path, State},
    response::{Html, IntoResponse, Redirect},
    routing::get,
};
use axum_htmx::HxRequest;
use tower_sessions::Session;

use crate::auth;
use crate::content::{self, form as content_form};
use crate::schema;
use crate::state::AppState;
use crate::templates::base_for_htmx;
use crate::uploads;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/{schema_slug}", get(list_entries))
        .route("/{schema_slug}/new", get(new_entry_page).post(create_entry))
        .route("/{schema_slug}/{entry_id}/edit", get(edit_entry_page))
        .route(
            "/{schema_slug}/{entry_id}",
            axum::routing::post(update_entry).delete(delete_entry),
        )
}

async fn list_entries(
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    Path(schema_slug): Path<String>,
) -> axum::response::Result<Html<String>> {
    let csrf_token = auth::ensure_csrf_token(&session).await;
    let schema_file = schema::get_schema(&state.config.schemas_dir(), &schema_slug)
        .map_err(|e| format!("Error: {e}"))?
        .ok_or("Schema not found")?;

    let entries = content::list_entries(&state.config.content_dir(), &schema_file)
        .map_err(|e| format!("Error: {e}"))?;

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
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Bool(b) => b.to_string(),
                            serde_json::Value::Number(n) => n.to_string(),
                            _ => v.to_string(),
                        })
                        .unwrap_or_default();
                    minijinja::Value::from(val)
                })
                .collect();
            minijinja::context! {
                id => e.id,
                columns => cols,
            }
        })
        .collect();

    let column_headers: Vec<&str> = columns.iter().map(|(_, label)| label.as_str()).collect();

    let flash = auth::take_flash(&session).await;
    let tmpl = state.templates.acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("content/list.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            schema_title => schema_file.meta.title,
            schema_slug => schema_slug,
            columns => column_headers,
            entries => entry_data,
            flash_kind => flash.as_ref().map(|(k, _)| k.as_str()),
            flash_message => flash.as_ref().map(|(_, m)| m.as_str()),
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html))
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
                && format != Some("upload")
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

async fn new_entry_page(
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    Path(schema_slug): Path<String>,
) -> axum::response::Result<Html<String>> {
    let csrf_token = auth::ensure_csrf_token(&session).await;
    let schema_file = schema::get_schema(&state.config.schemas_dir(), &schema_slug)
        .map_err(|e| format!("Error: {e}"))?
        .ok_or("Schema not found")?;

    let form_html = content_form::render_form_fields(&schema_file.schema, None, "");

    let tmpl = state.templates.acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("content/edit.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            schema_title => schema_file.meta.title,
            schema_slug => schema_slug,
            is_new => true,
            form_fields => form_html,
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html))
}

async fn edit_entry_page(
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    Path((schema_slug, entry_id)): Path<(String, String)>,
) -> axum::response::Result<Html<String>> {
    let csrf_token = auth::ensure_csrf_token(&session).await;
    let schema_file = schema::get_schema(&state.config.schemas_dir(), &schema_slug)
        .map_err(|e| format!("Error: {e}"))?
        .ok_or("Schema not found")?;

    let entry = content::get_entry(&state.config.content_dir(), &schema_file, &entry_id)
        .map_err(|e| format!("Error: {e}"))?
        .ok_or("Entry not found")?;

    let form_html = content_form::render_form_fields(&schema_file.schema, Some(&entry.data), "");

    let tmpl = state.templates.acquire_env()
        .map_err(|e| format!("Template env error: {e}"))?;
    let template = tmpl
        .get_template("content/edit.html")
        .map_err(|e| format!("Template error: {e}"))?;
    let html = template
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            schema_title => schema_file.meta.title,
            schema_slug => schema_slug,
            entry_id => entry_id,
            is_new => false,
            form_fields => form_html,
        })
        .map_err(|e| format!("Render error: {e}"))?;
    Ok(Html(html))
}

async fn create_entry(
    State(state): State<AppState>,
    session: Session,
    Path(schema_slug): Path<String>,
    multipart: Multipart,
) -> impl IntoResponse {
    let schema_file = match schema::get_schema(&state.config.schemas_dir(), &schema_slug) {
        Ok(Some(s)) => s,
        _ => return Redirect::to("/schemas").into_response(),
    };

    let (form_fields, upload_fields) = match parse_multipart(multipart).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Multipart parse error: {e}");
            return Redirect::to(&format!("/content/{schema_slug}/new")).into_response();
        }
    };

    // Verify CSRF token from multipart form fields
    let csrf_value = form_fields.iter().find(|(k, _)| k == "_csrf").map(|(_, v)| v.as_str());
    if !matches!(csrf_value, Some(token) if auth::verify_csrf_token(&session, token).await) {
        return (axum::http::StatusCode::FORBIDDEN, "Invalid CSRF token").into_response();
    }

    let mut data = content_form::form_data_to_json(&schema_file.schema, &form_fields, "");

    // Process upload fields
    process_uploads(&state, &mut data, &upload_fields);

    // Validate
    if let Err(errors) = content::validate_content(&schema_file, &data) {
        let form_html = content_form::render_form_fields(&schema_file.schema, Some(&data), "");
        if let Ok(tmpl) = state.templates.acquire_env() {
            if let Ok(template) = tmpl.get_template("content/edit.html") {
                if let Ok(html) = template.render(minijinja::context! {
                    schema_title => schema_file.meta.title,
                    schema_slug => schema_slug,
                    is_new => true,
                    form_fields => form_html,
                    errors => errors,
                }) {
                    return Html(html).into_response();
                }
            }
        }
        return Redirect::to(&format!("/content/{schema_slug}/new")).into_response();
    }

    match content::save_entry(&state.config.content_dir(), &schema_file, None, data) {
        Ok(id) => {
            crate::cache::reload_entry(
                &state.cache,
                &state.config.content_dir(),
                &schema_file,
                &id,
            );
            let user_id = auth::current_user_id(&session).await.unwrap_or(0);
            state.audit.log(&user_id.to_string(), "content_create", "content", &format!("{schema_slug}/{id}"), None);
            auth::set_flash(&session, "success", "Entry created").await;
            Redirect::to(&format!("/content/{schema_slug}")).into_response()
        }
        Err(e) => {
            tracing::error!("Save error: {e}");
            Redirect::to(&format!("/content/{schema_slug}/new")).into_response()
        }
    }
}

async fn update_entry(
    State(state): State<AppState>,
    session: Session,
    Path((schema_slug, entry_id)): Path<(String, String)>,
    multipart: Multipart,
) -> impl IntoResponse {
    let schema_file = match schema::get_schema(&state.config.schemas_dir(), &schema_slug) {
        Ok(Some(s)) => s,
        _ => return Redirect::to("/schemas").into_response(),
    };

    let (form_fields, upload_fields) = match parse_multipart(multipart).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Multipart parse error: {e}");
            return Redirect::to(&format!("/content/{schema_slug}/{entry_id}/edit"))
                .into_response();
        }
    };

    // Verify CSRF token from multipart form fields
    let csrf_value = form_fields.iter().find(|(k, _)| k == "_csrf").map(|(_, v)| v.as_str());
    if !matches!(csrf_value, Some(token) if auth::verify_csrf_token(&session, token).await) {
        return (axum::http::StatusCode::FORBIDDEN, "Invalid CSRF token").into_response();
    }

    let mut data = content_form::form_data_to_json(&schema_file.schema, &form_fields, "");

    process_uploads(&state, &mut data, &upload_fields);

    if let Err(errors) = content::validate_content(&schema_file, &data) {
        let form_html = content_form::render_form_fields(&schema_file.schema, Some(&data), "");
        if let Ok(tmpl) = state.templates.acquire_env() {
            if let Ok(template) = tmpl.get_template("content/edit.html") {
                if let Ok(html) = template.render(minijinja::context! {
                    schema_title => schema_file.meta.title,
                    schema_slug => schema_slug,
                    entry_id => entry_id,
                    is_new => false,
                    form_fields => form_html,
                    errors => errors,
                }) {
                    return Html(html).into_response();
                }
            }
        }
        return Redirect::to(&format!("/content/{schema_slug}/{entry_id}/edit")).into_response();
    }

    match content::save_entry(
        &state.config.content_dir(),
        &schema_file,
        Some(&entry_id),
        data,
    ) {
        Ok(_) => {
            crate::cache::reload_entry(
                &state.cache,
                &state.config.content_dir(),
                &schema_file,
                &entry_id,
            );
            let user_id = auth::current_user_id(&session).await.unwrap_or(0);
            state.audit.log(&user_id.to_string(), "content_update", "content", &format!("{schema_slug}/{entry_id}"), None);
            auth::set_flash(&session, "success", "Entry updated").await;
            Redirect::to(&format!("/content/{schema_slug}")).into_response()
        }
        Err(e) => {
            tracing::error!("Save error: {e}");
            Redirect::to(&format!("/content/{schema_slug}/{entry_id}/edit")).into_response()
        }
    }
}

async fn delete_entry(
    State(state): State<AppState>,
    session: Session,
    Path((schema_slug, entry_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let schema_file = match schema::get_schema(&state.config.schemas_dir(), &schema_slug) {
        Ok(Some(s)) => s,
        _ => return axum::http::StatusCode::NOT_FOUND,
    };

    let _ = content::delete_entry(&state.config.content_dir(), &schema_file, &entry_id);
    let key = format!("{schema_slug}/{entry_id}");
    state.cache.remove(&key);

    let user_id = auth::current_user_id(&session).await.unwrap_or(0);
    state.audit.log(&user_id.to_string(), "content_delete", "content", &format!("{schema_slug}/{entry_id}"), None);

    axum::http::StatusCode::NO_CONTENT
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
                // Empty file input — treated as text field
                form_fields.push((name, String::from_utf8_lossy(&data).to_string()));
            }
        } else {
            form_fields.push((name, String::from_utf8_lossy(&data).to_string()));
        }
    }

    Ok((form_fields, upload_fields))
}

fn process_uploads(state: &AppState, data: &mut serde_json::Value, upload_fields: &[UploadField]) {
    for upload in upload_fields {
        match uploads::store_upload(
            &state.config.uploads_dir(),
            &upload.filename,
            &upload.content_type,
            &upload.data,
        ) {
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
