use axum::{
    Extension, Router,
    body::Body,
    extract::{Multipart, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Json},
    routing::get,
};
use axum_htmx::HxRequest;
use tower_sessions::Session;

use crate::app_context::AppContext;
use crate::auth;
use crate::schema;
use crate::state::AppState;
use crate::templates::base_for_htmx;
use crate::uploads;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_uploads).post(upload_file))
        .route("/file/{hash}/{filename}", get(serve_upload))
        .route("/file/{hash}", get(serve_upload_no_name))
}

#[derive(serde::Deserialize)]
pub struct UploadFilter {
    q: Option<String>,
    schema: Option<String>,
}

#[derive(serde::Serialize)]
pub struct UploadRow {
    pub hash: String,
    pub filename: String,
    pub mime: String,
    pub size: String,
    pub created_at: String,
    pub references: Vec<UploadRef>,
}

#[derive(serde::Serialize)]
pub struct UploadRef {
    pub schema_slug: String,
    pub entry_id: String,
}

#[derive(serde::Serialize)]
struct SchemaOption {
    slug: String,
    title: String,
}

async fn list_uploads(
    Extension(user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    app: AppContext,
    Query(filter): Query<UploadFilter>,
) -> Result<Html<String>, StatusCode> {
    let csrf_token = auth::ensure_csrf_token(&session).await;

    let rows =
        match (&filter.q, &filter.schema) {
            (Some(q), Some(schema_slug)) if !q.is_empty() && !schema_slug.is_empty() => {
                let pattern = format!("%{q}%");
                sqlx::query_as::<
                _,
                (
                    String,
                    String,
                    String,
                    i64,
                    String,
                    Option<String>,
                    Option<String>,
                ),
            >(
                "SELECT u.hash, u.filename, u.mime, u.size, u.created_at, r.schema_slug, r.entry_id
                 FROM uploads u
                 LEFT JOIN upload_references r ON u.app_id = r.app_id AND u.hash = r.upload_hash
                 WHERE u.app_id = ? AND u.filename LIKE ? AND r.schema_slug = ?
                 ORDER BY u.created_at DESC",
            )
            .bind(app.app.id)
            .bind(&pattern)
            .bind(schema_slug)
            .fetch_all(&state.pool)
            .await
            }
            (Some(q), _) if !q.is_empty() => {
                let pattern = format!("%{q}%");
                sqlx::query_as::<
                _,
                (
                    String,
                    String,
                    String,
                    i64,
                    String,
                    Option<String>,
                    Option<String>,
                ),
            >(
                "SELECT u.hash, u.filename, u.mime, u.size, u.created_at, r.schema_slug, r.entry_id
                 FROM uploads u
                 LEFT JOIN upload_references r ON u.app_id = r.app_id AND u.hash = r.upload_hash
                 WHERE u.app_id = ? AND u.filename LIKE ?
                 ORDER BY u.created_at DESC",
            )
            .bind(app.app.id)
            .bind(&pattern)
            .fetch_all(&state.pool)
            .await
            }
            (_, Some(schema_slug)) if !schema_slug.is_empty() => sqlx::query_as::<
                _,
                (
                    String,
                    String,
                    String,
                    i64,
                    String,
                    Option<String>,
                    Option<String>,
                ),
            >(
                "SELECT u.hash, u.filename, u.mime, u.size, u.created_at, r.schema_slug, r.entry_id
                 FROM uploads u
                 LEFT JOIN upload_references r ON u.app_id = r.app_id AND u.hash = r.upload_hash
                 WHERE u.app_id = ? AND r.schema_slug = ?
                 ORDER BY u.created_at DESC",
            )
            .bind(app.app.id)
            .bind(schema_slug)
            .fetch_all(&state.pool)
            .await,
            _ => sqlx::query_as::<
                _,
                (
                    String,
                    String,
                    String,
                    i64,
                    String,
                    Option<String>,
                    Option<String>,
                ),
            >(
                "SELECT u.hash, u.filename, u.mime, u.size, u.created_at, r.schema_slug, r.entry_id
                 FROM uploads u
                 LEFT JOIN upload_references r ON u.app_id = r.app_id AND u.hash = r.upload_hash
                 WHERE u.app_id = ?
                 ORDER BY u.created_at DESC",
            )
            .bind(app.app.id)
            .fetch_all(&state.pool)
            .await,
        }
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Group rows by upload hash (JOIN produces multiple rows per upload if multiple refs)
    let mut upload_map: indexmap::IndexMap<String, UploadRow> = indexmap::IndexMap::new();
    for (hash, filename, mime, size, created_at, ref_schema, ref_entry) in rows {
        let entry = upload_map.entry(hash.clone()).or_insert_with(|| UploadRow {
            hash,
            filename,
            mime,
            size: format_size(size as u64),
            created_at,
            references: Vec::new(),
        });
        if let (Some(schema_slug), Some(entry_id)) = (ref_schema, ref_entry) {
            entry.references.push(UploadRef {
                schema_slug,
                entry_id,
            });
        }
    }
    let upload_rows: Vec<UploadRow> = upload_map.into_values().collect();

    // Get schema list for filter dropdown
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let schemas: Vec<SchemaOption> = schema::list_schemas(&schemas_dir)
        .unwrap_or_default()
        .into_iter()
        .map(|s| SchemaOption {
            slug: s.meta.slug,
            title: s.meta.title,
        })
        .collect();

    let user_role = &role.0;
    let current_username = user
        .username
        .as_ref()
        .map(|u| u.as_str().to_string())
        .unwrap_or_default();
    let env = state
        .templates
        .acquire_env()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let tmpl = env
        .get_template("uploads/list.html")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let html = tmpl
        .render(minijinja::context! {
            base_template => base_for_htmx(is_htmx),
            csrf_token => csrf_token,
            user_role => user_role,
            current_username => current_username,
            app => app.template_context(),
            nav_schemas => app.nav_schemas(&state.config),
            uploads => upload_rows,
            schemas => schemas,
            filter_q => filter.q.unwrap_or_default(),
            filter_schema => filter.schema.unwrap_or_default(),
        })
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Html(html))
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

async fn upload_file(
    Extension(_user): Extension<allowthem_core::User>,
    Extension(role): Extension<auth::CurrentUserRole>,
    State(state): State<AppState>,
    _session: Session,
    app: AppContext,
    mut multipart: Multipart,
) -> impl IntoResponse {
    if !auth::has_min_role(&role.0, "editor") {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Insufficient permissions"})),
        )
            .into_response();
    }
    let uploads_dir = state.config.app_uploads_dir(&app.app.slug);
    while let Ok(Some(field)) = multipart.next_field().await {
        let filename = field.file_name().unwrap_or("file").to_string();
        let content_type = field
            .content_type()
            .unwrap_or("application/octet-stream")
            .to_string();
        let data = match field.bytes().await {
            Ok(d) => d,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": e.to_string()})),
                )
                    .into_response();
            }
        };

        if data.is_empty() {
            continue;
        }

        if !uploads::is_mime_allowed(&content_type) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!(
                        "MIME type '{}' is not allowed. Allowed types: {}",
                        content_type,
                        uploads::allowed_mimes_display()
                    )
                })),
            )
                .into_response();
        }

        match uploads::store_upload(
            &uploads_dir,
            &state.pool,
            app.app.id,
            &filename,
            &content_type,
            &data,
        )
        .await
        {
            Ok(meta) => {
                return Json(serde_json::json!({
                    "hash": meta.hash,
                    "filename": meta.filename,
                    "mime": meta.mime,
                    "size": meta.size,
                }))
                .into_response();
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": e.to_string()})),
                )
                    .into_response();
            }
        }
    }

    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": "No file provided"})),
    )
        .into_response()
}

async fn serve_upload(
    State(state): State<AppState>,
    app: AppContext,
    Path((_app_slug, hash, _filename)): Path<(String, String, String)>,
) -> impl IntoResponse {
    serve_file(&state, &app, &hash).await
}

async fn serve_upload_no_name(
    State(state): State<AppState>,
    app: AppContext,
    Path((_app_slug, hash)): Path<(String, String)>,
) -> impl IntoResponse {
    serve_file(&state, &app, &hash).await
}

pub async fn serve_upload_by_hash(
    state: &AppState,
    app_id: i64,
    uploads_dir: &std::path::Path,
    hash: &str,
    request_headers: &HeaderMap,
) -> axum::response::Response {
    let path = match uploads::get_upload_path(uploads_dir, hash) {
        Some(p) => p,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    // The content-addressed hash is a natural ETag.
    let etag = format!("\"{hash}\"");
    if etag_matches(request_headers, &etag) {
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response();
    }

    let meta = uploads::db_get_upload_meta(&state.pool, app_id, hash)
        .await
        .ok()
        .flatten();
    let content_type = meta
        .as_ref()
        .map(|m| m.mime.clone())
        .unwrap_or_else(|| "application/octet-stream".to_string());

    match std::fs::read(&path) {
        Ok(data) => {
            let mut response = Body::from(data).into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_str(&content_type)
                    .unwrap_or(HeaderValue::from_static("application/octet-stream")),
            );
            response
                .headers_mut()
                .insert(header::ETAG, HeaderValue::from_str(&etag).unwrap());
            if let Some(meta) = &meta {
                let disposition = format!("inline; filename=\"{}\"", meta.filename);
                if let Ok(val) = HeaderValue::from_str(&disposition) {
                    response
                        .headers_mut()
                        .insert(header::CONTENT_DISPOSITION, val);
                }
            }
            response
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn serve_file(state: &AppState, app: &AppContext, hash: &str) -> axum::response::Response {
    let uploads_dir = state.config.app_uploads_dir(&app.app.slug);
    serve_upload_by_hash(state, app.app.id, &uploads_dir, hash, &HeaderMap::new()).await
}

/// Check if any value in If-None-Match matches the given ETag.
fn etag_matches(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').any(|t| t.trim() == etag))
        .unwrap_or(false)
}
