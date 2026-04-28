use std::sync::atomic::Ordering;

use axum::{
    Router,
    extract::{Multipart, Path, Query, RawQuery, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware,
    response::{IntoResponse, Json},
    routing::{get, post},
};
use sha2::{Digest, Sha256};

use crate::app_context::ApiAppContext;
use crate::auth::token::BearerToken;
use crate::content;
use crate::schema;
use crate::state::AppState;
use crate::uploads;

fn internal_error(e: impl std::fmt::Display) -> (StatusCode, Json<serde_json::Value>) {
    tracing::error!("Internal error: {e}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": "Internal server error"})),
    )
}

/// Build a JSON response with an ETag derived from the body, returning 304 if
/// the client already has a matching version. When `cache_key` is provided,
/// the computed ETag is stored in `etag_cache` so subsequent requests skip the
/// hash computation until the cache entry is invalidated.
fn json_with_etag(
    value: &serde_json::Value,
    request_headers: &HeaderMap,
    etag_cache: &crate::state::EtagCache,
    cache_key: Option<&str>,
) -> axum::response::Response {
    let etag = if let Some(key) = cache_key {
        if let Some(cached) = etag_cache.get(key) {
            cached.clone()
        } else {
            let body = serde_json::to_vec(value).unwrap();
            let hash = hex::encode(Sha256::digest(&body));
            let tag = format!("\"{hash}\"");
            etag_cache.insert(key.to_string(), tag.clone());
            tag
        }
    } else {
        let body = serde_json::to_vec(value).unwrap();
        let hash = hex::encode(Sha256::digest(&body));
        format!("\"{hash}\"")
    };

    if let Some(inm) = request_headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
    {
        if inm.split(',').any(|t| t.trim() == etag) {
            return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response();
        }
    }

    let mut resp = Json(value.clone()).into_response();
    resp.headers_mut()
        .insert(header::ETAG, HeaderValue::from_str(&etag).unwrap());
    resp
}

const MAX_API_LIMIT: usize = 500;

#[derive(serde::Deserialize, Default)]
pub struct ListParams {
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub render: String,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub order: Option<String>,
}

fn extract_filters_from_query(raw_query: &str) -> Vec<(String, String)> {
    raw_query
        .split('&')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            let field = key.strip_prefix("filter.")?;
            let value = simple_url_decode(value);
            Some((field.to_string(), value))
        })
        .collect()
}

fn simple_url_decode(s: &str) -> String {
    let s = s.replace('+', " ");
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            } else {
                result.push('%');
                result.push_str(&hex);
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn should_render(params_render: &str, schema_render: Option<&str>) -> bool {
    match params_render {
        "html" => true,
        "raw" => false,
        _ => schema_render == Some("html"),
    }
}

pub fn api_global_routes() -> Router<AppState> {
    Router::new()
        .route("/openapi.json", get(openapi_spec))
        .route("/backups/status", get(api_backup_status))
        .route("/backups/trigger", post(api_trigger_backup))
}

async fn openapi_spec(State(state): State<AppState>) -> impl IntoResponse {
    // Try to read from cache first
    {
        let cached = state.openapi_cache.read().unwrap();
        if let Some(spec) = cached.as_ref() {
            return Json(spec.clone()).into_response();
        }
    }

    // Generate and cache
    let spec = crate::openapi::generate_spec(&state.config.data_dir);
    if let Ok(mut cache) = state.openapi_cache.write() {
        *cache = Some(spec.clone());
    }
    Json(spec).into_response()
}

pub fn api_app_routes() -> Router<AppState> {
    Router::new()
        .route("/schemas", get(list_schemas))
        .route("/schemas/{slug}", get(get_schema))
        .route(
            "/content/{schema_slug}",
            get(list_entries).post(create_entry),
        )
        .route(
            "/content/{schema_slug}/single",
            get(get_single).put(upsert_single).delete(delete_single),
        )
        .route("/content/{schema_slug}/_bulk/create", post(api_bulk_create))
        .route("/content/{schema_slug}/_bulk/update", post(api_bulk_update))
        .route("/content/{schema_slug}/_bulk/delete", post(api_bulk_delete))
        .route(
            "/content/{schema_slug}/_bulk/publish",
            post(api_bulk_publish),
        )
        .route(
            "/content/{schema_slug}/_bulk/unpublish",
            post(api_bulk_unpublish),
        )
        .route(
            "/content/{schema_slug}/{entry_id}/publish",
            post(api_publish_entry),
        )
        .route(
            "/content/{schema_slug}/{entry_id}/unpublish",
            post(api_unpublish_entry),
        )
        .route(
            "/content/{schema_slug}/{entry_id}/versions",
            get(api_list_versions),
        )
        .route(
            "/content/{schema_slug}/{entry_id}/versions/{timestamp}",
            get(api_get_version),
        )
        .route(
            "/content/{schema_slug}/{entry_id}/versions/{timestamp}/revert",
            post(api_revert_version),
        )
        .route(
            "/content/{schema_slug}/{entry_id}",
            get(get_entry).put(update_entry).delete(delete_entry),
        )
        .route("/uploads", post(upload_file))
        .route("/uploads/{hash}", get(get_upload))
        .route("/uploads/{hash}/{filename}", get(get_upload_named))
        .route("/export", post(export_bundle))
        .route("/import", post(import_bundle))
        .route("/deployments", get(api_list_deployments))
        .route("/deployments/{slug}/fire", post(api_fire_deployment))
}

pub async fn api_rate_limit(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: middleware::Next,
) -> axum::response::Response {
    let ip = if state.config.trust_proxy_headers {
        headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|xff| xff.split(',').next())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "direct".to_string())
    } else {
        "direct".to_string()
    };

    if !state.api_limiter.check(&ip) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({"error": "Rate limit exceeded"})),
        )
            .into_response();
    }

    next.run(request).await
}

fn require_api_role(
    bearer: &BearerToken,
    min_role: &str,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let role_level = |r: &str| -> u8 {
        match r {
            "admin" => 3,
            "editor" => 2,
            "viewer" => 1,
            _ => 0,
        }
    };
    if role_level(&bearer.role) >= role_level(min_role) {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Insufficient permissions"})),
        ))
    }
}

fn require_token_app(
    token: &BearerToken,
    app: &ApiAppContext,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    if token.app_id != Some(app.app.id) {
        Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "Token not authorized for this app"})),
        ))
    } else {
        Ok(())
    }
}

fn resolve_references(
    data: &mut serde_json::Value,
    schema: &serde_json::Value,
    cache: &crate::state::ContentCache,
    app_slug: &str,
) {
    resolve_references_inner(data, schema, cache, app_slug, 0);
}

fn resolve_references_inner(
    data: &mut serde_json::Value,
    schema: &serde_json::Value,
    cache: &crate::state::ContentCache,
    app_slug: &str,
    depth: usize,
) {
    if depth > 32 {
        return;
    }
    let Some(props) = schema.get("properties").and_then(|p| p.as_object()) else {
        return;
    };
    let Some(obj) = data.as_object_mut() else {
        return;
    };
    for (key, prop) in props {
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
            if let Some(serde_json::Value::String(ref_id)) = obj.get(key).cloned() {
                let cache_key = format!("{app_slug}/{target_slug}/{ref_id}");
                if let Some(entry) = cache.get(&cache_key) {
                    obj.insert(key.clone(), entry.value().clone());
                }
            }
        } else if field_type == Some("object") {
            if let Some(nested) = obj.get_mut(key) {
                resolve_references_inner(nested, prop, cache, app_slug, depth + 1);
            }
        } else if field_type == Some("array") {
            if let Some(items_schema) = prop.get("items")
                && let Some(serde_json::Value::Array(arr)) = obj.get_mut(key)
            {
                for item in arr.iter_mut() {
                    resolve_references_inner(item, items_schema, cache, app_slug, depth + 1);
                }
            }
        }
    }
}

async fn list_schemas(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
) -> impl IntoResponse {
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    match schema::list_schemas(&schemas_dir) {
        Ok(schemas) => {
            let data: Vec<serde_json::Value> = schemas
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "title": s.meta.title,
                        "slug": s.meta.slug,
                        "storage": s.meta.storage.to_string(),
                        "kind": s.meta.kind.to_string(),
                        "schema": s.schema,
                    })
                })
                .collect();
            Json(serde_json::json!(data)).into_response()
        }
        Err(e) => internal_error(e).into_response(),
    }
}

async fn get_schema(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, slug)): Path<(String, String)>,
) -> impl IntoResponse {
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    match schema::get_schema(&schemas_dir, &slug) {
        Ok(Some(s)) => Json(s.schema).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

async fn list_entries(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, schema_slug)): Path<(String, String)>,
    Query(params): Query<ListParams>,
    RawQuery(raw_query): RawQuery,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            return internal_error(e).into_response();
        }
    };

    match content::list_entries(&content_dir, &schema_file) {
        Ok(entries) => {
            let filters = extract_filters_from_query(raw_query.as_deref().unwrap_or(""));
            let use_envelope = params.limit.is_some() || params.offset.is_some();
            let limit = params
                .limit
                .map(|l| if l == 0 { 20 } else { l.min(MAX_API_LIMIT) });

            let query_params = content::QueryParams {
                status: if params.status.is_empty() {
                    "published".into()
                } else {
                    params.status.clone()
                },
                q: params.q.trim().to_string(),
                filters,
                sort_field: params.sort.clone().unwrap_or_else(|| "_id".into()),
                sort_order: match params.order.as_deref() {
                    Some("desc") => content::SortOrder::Desc,
                    _ => content::SortOrder::Asc,
                },
                offset: params.offset.unwrap_or(0),
                limit,
            };

            let result = content::query_entries(entries, &query_params);

            let data: Vec<serde_json::Value> = result
                .entries
                .iter()
                .map(|e| {
                    let mut d = content::strip_internal_status(&e.data);
                    resolve_references(&mut d, &schema_file.schema, &state.cache, &app.app.slug);
                    if should_render(&params.render, schema_file.meta.render.as_deref()) {
                        content::render_markdown_fields(&mut d, &schema_file.schema);
                    }
                    content::project_richtext_fields(&mut d, &schema_file.schema, params.render == "raw", &app.app.slug);
                    d
                })
                .collect();

            if use_envelope {
                let value = serde_json::json!({
                    "data": data,
                    "meta": {
                        "total": result.total,
                        "limit": limit.unwrap_or(result.total),
                        "offset": query_params.offset,
                        "count": data.len(),
                    }
                });
                Json(value).into_response()
            } else {
                let value = serde_json::json!(data);
                json_with_etag(&value, &headers, &state.etag_cache, None)
            }
        }
        Err(e) => internal_error(e).into_response(),
    }
}

async fn get_entry(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, schema_slug, entry_id)): Path<(String, String, String)>,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            return internal_error(e).into_response();
        }
    };

    match content::get_entry(&content_dir, &schema_file, &entry_id) {
        Ok(Some(entry)) => {
            let mut data = content::strip_internal_status(&entry.data);
            resolve_references(&mut data, &schema_file.schema, &state.cache, &app.app.slug);
            let render = should_render(&params.render, schema_file.meta.render.as_deref());
            if render {
                content::render_markdown_fields(&mut data, &schema_file.schema);
            }
            content::project_richtext_fields(&mut data, &schema_file.schema, params.render == "raw", &app.app.slug);
            // Bypass ETag cache for rendered responses to avoid serving
            // a cached raw-markdown ETag for a rendered response or vice versa
            let cache_key = if render || params.render == "raw" {
                None
            } else {
                Some(format!("{}/{}/{}", app.app.slug, schema_slug, entry_id))
            };
            json_with_etag(&data, &headers, &state.etag_cache, cache_key.as_deref())
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

async fn create_entry(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, schema_slug)): Path<(String, String)>,
    Json(data): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Err(e) = require_api_role(&token, "editor") {
        return e.into_response();
    }
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            return internal_error(e).into_response();
        }
    };

    if schema_file.meta.kind == crate::schema::models::Kind::Single {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "This schema is a single. Use PUT /content/{slug}/single instead."})),
        )
            .into_response();
    }

    let target_status = content::resolve_target_status(&data, &content_dir, &schema_file, None);
    let ctx = content::ValidationContext {
        entry_id: None,
        target_status: &target_status,
        cache: &state.cache,
        app_slug: &app.app.slug,
        schema_slug: &schema_slug,
    };
    if let Err(errors) = content::validate_content(&schema_file, &data, &ctx) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"errors": errors})),
        )
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
                "api",
                "content_create",
                "content",
                &format!("{schema_slug}/{id}"),
                None,
                Some(app.app.id),
            );
            (StatusCode::CREATED, Json(serde_json::json!({"id": id}))).into_response()
        }
        Err(e) => internal_error(e).into_response(),
    }
}

async fn update_entry(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, schema_slug, entry_id)): Path<(String, String, String)>,
    Json(data): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Err(e) = require_api_role(&token, "editor") {
        return e.into_response();
    }
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let app_dir = state.config.app_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            return internal_error(e).into_response();
        }
    };

    let target_status =
        content::resolve_target_status(&data, &content_dir, &schema_file, Some(&entry_id));
    let ctx = content::ValidationContext {
        entry_id: Some(&entry_id),
        target_status: &target_status,
        cache: &state.cache,
        app_slug: &app.app.slug,
        schema_slug: &schema_slug,
    };
    if let Err(errors) = content::validate_content(&schema_file, &data, &ctx) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"errors": errors})),
        )
            .into_response();
    }

    // Snapshot current version for history
    if let Ok(Some(current)) = content::get_entry(&content_dir, &schema_file, &entry_id) {
        let snap_meta = crate::history::SnapshotMeta {
            user_id: "api".into(),
            username: "api".into(),
            source: crate::history::SnapshotSource::Api,
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
                "api",
                "content_update",
                "content",
                &format!("{schema_slug}/{entry_id}"),
                None,
                Some(app.app.id),
            );
            StatusCode::OK.into_response()
        }
        Err(e) => internal_error(e).into_response(),
    }
}

async fn delete_entry(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, schema_slug, entry_id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    if let Err(e) = require_api_role(&token, "editor") {
        return e.into_response();
    }
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let app_dir = state.config.app_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            return internal_error(e).into_response();
        }
    };

    let referencing = content::find_referencing_entries(
        &state.cache,
        &schemas_dir,
        &app.app.slug,
        &schema_slug,
        &entry_id,
    );

    let _ = uploads::db_delete_references(&state.pool, app.app.id, &schema_slug, &entry_id).await;
    match content::delete_entry(&content_dir, &schema_file, &entry_id) {
        Ok(()) => {
            crate::history::delete_history(&app_dir, &schema_slug, &entry_id);
            let key = format!("{}/{schema_slug}/{entry_id}", app.app.slug);
            state.cache.remove(&key);
            state.audit.log_with_app(
                "api",
                "content_delete",
                "content",
                &format!("{schema_slug}/{entry_id}"),
                None,
                Some(app.app.id),
            );
            let mut response = StatusCode::NO_CONTENT.into_response();
            if !referencing.is_empty() {
                let warnings: Vec<String> = referencing
                    .iter()
                    .map(|(s, e)| format!("{s}/{e}"))
                    .collect();
                if let Ok(val) = HeaderValue::from_str(&warnings.join(", ")) {
                    response.headers_mut().insert("x-substrukt-warnings", val);
                }
            }
            response
        }
        Err(e) => internal_error(e).into_response(),
    }
}

async fn get_single(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, schema_slug)): Path<(String, String)>,
    Query(params): Query<ListParams>,
) -> impl IntoResponse {
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            return internal_error(e).into_response();
        }
    };

    match content::get_entry(&content_dir, &schema_file, "_single") {
        Ok(Some(entry)) => {
            // Default to "all" for single-kind — unlike collections, singletons should
            // be visible immediately after upsert. Callers can still pass ?status=published
            // to explicitly filter.
            let status = if params.status.is_empty() {
                "all"
            } else {
                &params.status
            };
            if status != "all" && content::get_entry_status(&entry.data) != status {
                return StatusCode::NOT_FOUND.into_response();
            }

            let mut data = content::strip_internal_status(&entry.data);
            resolve_references(&mut data, &schema_file.schema, &state.cache, &app.app.slug);
            if should_render(&params.render, schema_file.meta.render.as_deref()) {
                content::render_markdown_fields(&mut data, &schema_file.schema);
            }
            content::project_richtext_fields(&mut data, &schema_file.schema, params.render == "raw", &app.app.slug);
            Json(data).into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

async fn upsert_single(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, schema_slug)): Path<(String, String)>,
    Json(data): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Err(e) = require_api_role(&token, "editor") {
        return e.into_response();
    }
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let app_dir = state.config.app_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            return internal_error(e).into_response();
        }
    };

    let target_status =
        content::resolve_target_status(&data, &content_dir, &schema_file, Some("_single"));
    let ctx = content::ValidationContext {
        entry_id: Some("_single"),
        target_status: &target_status,
        cache: &state.cache,
        app_slug: &app.app.slug,
        schema_slug: &schema_slug,
    };
    if let Err(errors) = content::validate_content(&schema_file, &data, &ctx) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"errors": errors})),
        )
            .into_response();
    }

    // Snapshot current version for history
    if let Ok(Some(current)) = content::get_entry(&content_dir, &schema_file, "_single") {
        let snap_meta = crate::history::SnapshotMeta {
            user_id: "api".into(),
            username: "api".into(),
            source: crate::history::SnapshotSource::Api,
        };
        if let Err(e) = crate::history::snapshot_entry(
            &app_dir,
            &schema_slug,
            "_single",
            &current.data,
            state.config.version_history_count,
            Some(&snap_meta),
        ) {
            tracing::warn!("Failed to snapshot version: {e}");
        }
    }

    let hashes = uploads::extract_upload_hashes(&data);
    match content::save_entry(&content_dir, &schema_file, Some("_single"), data) {
        Ok(_) => {
            crate::cache::reload_entry(
                &state.cache,
                &state.etag_cache,
                &content_dir,
                &schema_file,
                "_single",
                &app.app.slug,
            );
            let _ = uploads::db_update_references(
                &state.pool,
                app.app.id,
                &schema_slug,
                "_single",
                &hashes,
            )
            .await;
            state.audit.log_with_app(
                "api",
                "content_update",
                "content",
                &format!("{schema_slug}/_single"),
                None,
                Some(app.app.id),
            );
            StatusCode::OK.into_response()
        }
        Err(e) => internal_error(e).into_response(),
    }
}

async fn delete_single(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, schema_slug)): Path<(String, String)>,
) -> impl IntoResponse {
    if let Err(e) = require_api_role(&token, "editor") {
        return e.into_response();
    }
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let app_dir = state.config.app_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            return internal_error(e).into_response();
        }
    };

    let _ = uploads::db_delete_references(&state.pool, app.app.id, &schema_slug, "_single").await;
    match content::delete_entry(&content_dir, &schema_file, "_single") {
        Ok(()) => {
            crate::history::delete_history(&app_dir, &schema_slug, "_single");
            let key = format!("{}/{schema_slug}/_single", app.app.slug);
            state.cache.remove(&key);
            state.audit.log_with_app(
                "api",
                "content_delete",
                "content",
                &format!("{schema_slug}/_single"),
                None,
                Some(app.app.id),
            );
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => internal_error(e).into_response(),
    }
}

async fn api_publish_entry(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, schema_slug, entry_id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    if let Err(e) = require_api_role(&token, "editor") {
        return e.into_response();
    }
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            return internal_error(e).into_response();
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
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"errors": errors})),
            )
                .into_response();
        }
    }

    if let Err(e) = content::set_entry_status(&content_dir, &schema_file, &entry_id, "published") {
        let msg = e.to_string();
        if msg.contains("not found") {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": msg})),
            )
                .into_response();
        }
        return internal_error(e).into_response();
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
        "api",
        "entry_published",
        "content",
        &format!("{schema_slug}/{entry_id}"),
        None,
        Some(app.app.id),
    );

    Json(serde_json::json!({"status": "published", "entry_id": entry_id})).into_response()
}

async fn api_unpublish_entry(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, schema_slug, entry_id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    if let Err(e) = require_api_role(&token, "editor") {
        return e.into_response();
    }
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            return internal_error(e).into_response();
        }
    };

    if let Err(e) = content::set_entry_status(&content_dir, &schema_file, &entry_id, "draft") {
        let msg = e.to_string();
        if msg.contains("not found") {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": msg})),
            )
                .into_response();
        }
        return internal_error(e).into_response();
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
        "api",
        "entry_unpublished",
        "content",
        &format!("{schema_slug}/{entry_id}"),
        None,
        Some(app.app.id),
    );

    Json(serde_json::json!({"status": "draft", "entry_id": entry_id})).into_response()
}

async fn upload_file(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    mut multipart: Multipart,
) -> impl IntoResponse {
    if let Err(e) = require_api_role(&token, "editor") {
        return e.into_response();
    }
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
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
                return internal_error(e).into_response();
            }
        }
    }

    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": "No file provided"})),
    )
        .into_response()
}

async fn get_upload(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, hash)): Path<(String, String)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    get_upload_by_hash(state, token, app, hash, headers).await
}

async fn get_upload_named(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, hash, _filename)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    get_upload_by_hash(state, token, app, hash, headers).await
}

async fn get_upload_by_hash(
    state: AppState,
    token: BearerToken,
    app: ApiAppContext,
    hash: String,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let uploads_dir = state.config.app_uploads_dir(&app.app.slug);
    crate::routes::uploads::serve_upload_by_hash(&state, app.app.id, &uploads_dir, &hash, &headers)
        .await
}

async fn export_bundle(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
) -> impl IntoResponse {
    if let Err(e) = require_api_role(&token, "admin") {
        return e.into_response();
    }
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let app_dir = state.config.app_dir(&app.app.slug);
    let tmp =
        std::env::temp_dir().join(format!("substrukt-export-{}.tar.gz", uuid::Uuid::new_v4()));
    match crate::sync::export_bundle(&app_dir, &state.pool, app.app.id, &tmp).await {
        Ok(()) => match std::fs::read(&tmp) {
            Ok(data) => {
                let _ = std::fs::remove_file(&tmp);
                state
                    .audit
                    .log_with_app("api", "export", "bundle", "", None, Some(app.app.id));
                let mut response = axum::body::Body::from(data).into_response();
                response.headers_mut().insert(
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("application/gzip"),
                );
                response.headers_mut().insert(
                    axum::http::header::CONTENT_DISPOSITION,
                    axum::http::HeaderValue::from_static("attachment; filename=\"bundle.tar.gz\""),
                );
                response
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                internal_error(e).into_response()
            }
        },
        Err(e) => internal_error(e).into_response(),
    }
}

async fn import_bundle(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    mut multipart: Multipart,
) -> impl IntoResponse {
    if let Err(e) = require_api_role(&token, "admin") {
        return e.into_response();
    }
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let app_dir = state.config.app_dir(&app.app.slug);
    while let Ok(Some(field)) = multipart.next_field().await {
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

        match crate::sync::import_bundle_from_bytes(&app_dir, &state.pool, app.app.id, &data).await
        {
            Ok(warnings) => {
                // Rebuild cache after import
                crate::cache::rebuild(&state.cache, &state.etag_cache, &state.config.data_dir);
                state
                    .audit
                    .log_with_app("api", "import", "bundle", "", None, Some(app.app.id));
                return Json(serde_json::json!({
                    "status": "ok",
                    "warnings": warnings,
                }))
                .into_response();
            }
            Err(e) => {
                return internal_error(e).into_response();
            }
        }
    }

    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": "No file provided"})),
    )
        .into_response()
}

async fn api_list_deployments(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
) -> impl IntoResponse {
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    match state.audit.list_deployments_for_app(app.app.id).await {
        Ok(deployments) => {
            let data: Vec<serde_json::Value> = deployments
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "name": d.name,
                        "slug": d.slug,
                        "webhook_url": d.webhook_url,
                        "include_drafts": d.include_drafts,
                        "auto_deploy": d.auto_deploy,
                        "debounce_seconds": d.debounce_seconds,
                    })
                })
                .collect();
            Json(serde_json::json!(data)).into_response()
        }
        Err(e) => internal_error(e).into_response(),
    }
}

const MAX_BULK_ENTRIES: usize = 500;

#[derive(serde::Deserialize)]
struct BulkCreateRequest {
    entries: Vec<serde_json::Value>,
}

#[derive(serde::Deserialize)]
struct BulkUpdateRequest {
    entries: Vec<serde_json::Value>,
}

#[derive(serde::Deserialize)]
struct BulkIdsRequest {
    ids: Vec<String>,
}

#[derive(serde::Serialize)]
struct BulkItemResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    errors: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

async fn api_bulk_create(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, schema_slug)): Path<(String, String)>,
    Json(body): Json<BulkCreateRequest>,
) -> impl IntoResponse {
    if let Err(e) = require_api_role(&token, "editor") {
        return e.into_response();
    }
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return internal_error(e).into_response(),
    };
    if schema_file.meta.kind == crate::schema::models::Kind::Single {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Bulk create not supported for single-kind schemas"})),
        )
            .into_response();
    }
    if body.entries.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "entries array is empty"})),
        )
            .into_response();
    }
    if body.entries.len() > MAX_BULK_ENTRIES {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("Too many entries. Maximum is {MAX_BULK_ENTRIES} per request.")})),
        )
            .into_response();
    }

    let mut results = Vec::new();
    let mut created = 0usize;
    let mut failed = 0usize;
    let mut used_ids = std::collections::HashSet::new();

    for (i, data) in body.entries.into_iter().enumerate() {
        let target_status = content::resolve_target_status(&data, &content_dir, &schema_file, None);
        let ctx = content::ValidationContext {
            entry_id: None,
            target_status: &target_status,
            cache: &state.cache,
            app_slug: &app.app.slug,
            schema_slug: &schema_slug,
        };
        if let Err(errors) = content::validate_content(&schema_file, &data, &ctx) {
            let msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
            results.push(BulkItemResult {
                index: Some(i),
                id: None,
                status: "error".into(),
                errors: Some(msgs),
                error: None,
            });
            failed += 1;
            continue;
        }
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
                used_ids.insert(id.clone());
                results.push(BulkItemResult {
                    index: Some(i),
                    id: Some(id),
                    status: "created".into(),
                    errors: None,
                    error: None,
                });
                created += 1;
            }
            Err(e) => {
                results.push(BulkItemResult {
                    index: Some(i),
                    id: None,
                    status: "error".into(),
                    errors: None,
                    error: Some(e.to_string()),
                });
                failed += 1;
            }
        }
    }

    state.audit.log_with_app(
        "api",
        "content_bulk_create",
        "content",
        &format!("{schema_slug}: {created} created, {failed} failed"),
        None,
        Some(app.app.id),
    );

    Json(serde_json::json!({
        "total": created + failed,
        "created": created,
        "failed": failed,
        "results": results,
    }))
    .into_response()
}

async fn api_bulk_update(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, schema_slug)): Path<(String, String)>,
    Json(body): Json<BulkUpdateRequest>,
) -> impl IntoResponse {
    if let Err(e) = require_api_role(&token, "editor") {
        return e.into_response();
    }
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let app_dir = state.config.app_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return internal_error(e).into_response(),
    };
    if body.entries.is_empty() || body.entries.len() > MAX_BULK_ENTRIES {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "entries array is empty or exceeds limit"})),
        )
            .into_response();
    }

    let mut results = Vec::new();
    let mut updated = 0usize;
    let mut failed = 0usize;

    for (i, data) in body.entries.into_iter().enumerate() {
        let entry_id = data
            .get("_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if entry_id.is_empty() {
            results.push(BulkItemResult {
                index: Some(i),
                id: None,
                status: "error".into(),
                errors: None,
                error: Some("missing _id field".into()),
            });
            failed += 1;
            continue;
        }

        let target_status =
            content::resolve_target_status(&data, &content_dir, &schema_file, Some(&entry_id));
        let ctx = content::ValidationContext {
            entry_id: Some(&entry_id),
            target_status: &target_status,
            cache: &state.cache,
            app_slug: &app.app.slug,
            schema_slug: &schema_slug,
        };
        if let Err(errors) = content::validate_content(&schema_file, &data, &ctx) {
            let msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
            results.push(BulkItemResult {
                index: Some(i),
                id: Some(entry_id),
                status: "error".into(),
                errors: Some(msgs),
                error: None,
            });
            failed += 1;
            continue;
        }

        if let Ok(Some(current)) = content::get_entry(&content_dir, &schema_file, &entry_id) {
            let snap_meta = crate::history::SnapshotMeta {
                user_id: "api".into(),
                username: "api".into(),
                source: crate::history::SnapshotSource::Api,
            };
            let _ = crate::history::snapshot_entry(
                &app_dir,
                &schema_slug,
                &entry_id,
                &current.data,
                state.config.version_history_count,
                Some(&snap_meta),
            );
        }

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
                results.push(BulkItemResult {
                    index: Some(i),
                    id: Some(entry_id),
                    status: "updated".into(),
                    errors: None,
                    error: None,
                });
                updated += 1;
            }
            Err(e) => {
                results.push(BulkItemResult {
                    index: Some(i),
                    id: Some(entry_id),
                    status: "error".into(),
                    errors: None,
                    error: Some(e.to_string()),
                });
                failed += 1;
            }
        }
    }

    state.audit.log_with_app(
        "api",
        "content_bulk_update",
        "content",
        &format!("{schema_slug}: {updated} updated, {failed} failed"),
        None,
        Some(app.app.id),
    );

    Json(serde_json::json!({
        "total": updated + failed,
        "updated": updated,
        "failed": failed,
        "results": results,
    }))
    .into_response()
}

async fn api_bulk_delete(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, schema_slug)): Path<(String, String)>,
    Json(body): Json<BulkIdsRequest>,
) -> impl IntoResponse {
    if let Err(e) = require_api_role(&token, "editor") {
        return e.into_response();
    }
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let app_dir = state.config.app_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return internal_error(e).into_response(),
    };

    let mut results = Vec::new();
    let mut deleted = 0usize;
    let mut failed = 0usize;

    for id in &body.ids {
        let _ = uploads::db_delete_references(&state.pool, app.app.id, &schema_slug, id).await;
        match content::delete_entry(&content_dir, &schema_file, id) {
            Ok(()) => {
                crate::history::delete_history(&app_dir, &schema_slug, id);
                let key = format!("{}/{schema_slug}/{id}", app.app.slug);
                state.cache.remove(&key);
                results.push(BulkItemResult {
                    index: None,
                    id: Some(id.clone()),
                    status: "deleted".into(),
                    errors: None,
                    error: None,
                });
                deleted += 1;
            }
            Err(e) => {
                results.push(BulkItemResult {
                    index: None,
                    id: Some(id.clone()),
                    status: "error".into(),
                    errors: None,
                    error: Some(e.to_string()),
                });
                failed += 1;
            }
        }
    }

    state.audit.log_with_app(
        "api",
        "content_bulk_delete",
        "content",
        &format!("{schema_slug}: {deleted} deleted, {failed} failed"),
        None,
        Some(app.app.id),
    );

    Json(serde_json::json!({
        "total": deleted + failed,
        "deleted": deleted,
        "failed": failed,
        "results": results,
    }))
    .into_response()
}

async fn api_bulk_publish(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, schema_slug)): Path<(String, String)>,
    Json(body): Json<BulkIdsRequest>,
) -> impl IntoResponse {
    if let Err(e) = require_api_role(&token, "editor") {
        return e.into_response();
    }
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return internal_error(e).into_response(),
    };

    let mut results = Vec::new();
    let mut published = 0usize;
    let mut failed = 0usize;

    for id in &body.ids {
        if let Ok(Some(entry)) = content::get_entry(&content_dir, &schema_file, id) {
            let ctx = content::ValidationContext {
                entry_id: Some(id),
                target_status: "published",
                cache: &state.cache,
                app_slug: &app.app.slug,
                schema_slug: &schema_slug,
            };
            if let Err(errors) = content::validate_for_publish(&schema_file, &entry.data, &ctx) {
                let msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
                results.push(BulkItemResult {
                    index: None,
                    id: Some(id.clone()),
                    status: "error".into(),
                    errors: Some(msgs),
                    error: None,
                });
                failed += 1;
                continue;
            }
        }
        match content::set_entry_status(&content_dir, &schema_file, id, "published") {
            Ok(()) => {
                crate::cache::reload_entry(
                    &state.cache,
                    &state.etag_cache,
                    &content_dir,
                    &schema_file,
                    id,
                    &app.app.slug,
                );
                results.push(BulkItemResult {
                    index: None,
                    id: Some(id.clone()),
                    status: "published".into(),
                    errors: None,
                    error: None,
                });
                published += 1;
            }
            Err(e) => {
                results.push(BulkItemResult {
                    index: None,
                    id: Some(id.clone()),
                    status: "error".into(),
                    errors: None,
                    error: Some(e.to_string()),
                });
                failed += 1;
            }
        }
    }

    state.audit.log_with_app(
        "api",
        "content_bulk_publish",
        "content",
        &format!("{schema_slug}: {published} published, {failed} failed"),
        None,
        Some(app.app.id),
    );

    Json(serde_json::json!({
        "total": published + failed,
        "published": published,
        "failed": failed,
        "results": results,
    }))
    .into_response()
}

async fn api_bulk_unpublish(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, schema_slug)): Path<(String, String)>,
    Json(body): Json<BulkIdsRequest>,
) -> impl IntoResponse {
    if let Err(e) = require_api_role(&token, "editor") {
        return e.into_response();
    }
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return internal_error(e).into_response(),
    };

    let mut results = Vec::new();
    let mut unpublished = 0usize;
    let mut failed = 0usize;

    for id in &body.ids {
        match content::set_entry_status(&content_dir, &schema_file, id, "draft") {
            Ok(()) => {
                crate::cache::reload_entry(
                    &state.cache,
                    &state.etag_cache,
                    &content_dir,
                    &schema_file,
                    id,
                    &app.app.slug,
                );
                results.push(BulkItemResult {
                    index: None,
                    id: Some(id.clone()),
                    status: "unpublished".into(),
                    errors: None,
                    error: None,
                });
                unpublished += 1;
            }
            Err(e) => {
                results.push(BulkItemResult {
                    index: None,
                    id: Some(id.clone()),
                    status: "error".into(),
                    errors: None,
                    error: Some(e.to_string()),
                });
                failed += 1;
            }
        }
    }

    state.audit.log_with_app(
        "api",
        "content_bulk_unpublish",
        "content",
        &format!("{schema_slug}: {unpublished} unpublished, {failed} failed"),
        None,
        Some(app.app.id),
    );

    Json(serde_json::json!({
        "total": unpublished + failed,
        "unpublished": unpublished,
        "failed": failed,
        "results": results,
    }))
    .into_response()
}

async fn api_list_versions(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, schema_slug, entry_id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let app_dir = state.config.app_dir(&app.app.slug);
    match crate::history::list_versions(&app_dir, &schema_slug, &entry_id) {
        Ok(versions) => {
            let data: Vec<serde_json::Value> = versions
                .iter()
                .map(|v| {
                    let date = chrono::DateTime::from_timestamp_millis(v.timestamp as i64)
                        .map(|dt| dt.to_rfc3339())
                        .unwrap_or_default();
                    serde_json::json!({
                        "timestamp": v.timestamp,
                        "date": date,
                        "size": v.size,
                        "user_id": v.user_id,
                        "username": v.username,
                        "source": v.source,
                    })
                })
                .collect();
            Json(serde_json::json!(data)).into_response()
        }
        Err(e) => internal_error(e).into_response(),
    }
}

async fn api_get_version(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, schema_slug, entry_id, timestamp)): Path<(String, String, String, u64)>,
) -> impl IntoResponse {
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let app_dir = state.config.app_dir(&app.app.slug);
    match crate::history::get_version(&app_dir, &schema_slug, &entry_id, timestamp) {
        Ok(Some(data)) => Json(data).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

async fn api_revert_version(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, schema_slug, entry_id, timestamp)): Path<(String, String, String, u64)>,
) -> impl IntoResponse {
    if let Err(e) = require_api_role(&token, "editor") {
        return e.into_response();
    }
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }
    let schemas_dir = state.config.app_schemas_dir(&app.app.slug);
    let content_dir = state.config.app_content_dir(&app.app.slug);
    let app_dir = state.config.app_dir(&app.app.slug);
    let schema_file = match schema::get_schema(&schemas_dir, &schema_slug) {
        Ok(Some(s)) => s,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return internal_error(e).into_response(),
    };

    let version_data =
        match crate::history::get_version(&app_dir, &schema_slug, &entry_id, timestamp) {
            Ok(Some(data)) => data,
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "Version not found"})),
                )
                    .into_response();
            }
            Err(e) => return internal_error(e).into_response(),
        };

    if let Ok(Some(current)) = content::get_entry(&content_dir, &schema_file, &entry_id) {
        let snap_meta = crate::history::SnapshotMeta {
            user_id: "api".into(),
            username: "api".into(),
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
                "api",
                "content_update",
                "content",
                &format!("{schema_slug}/{entry_id}"),
                Some(&format!("reverted to version {timestamp}")),
                Some(app.app.id),
            );
            Json(serde_json::json!({
                "status": "reverted",
                "entry_id": entry_id,
                "reverted_to": timestamp,
            }))
            .into_response()
        }
        Err(e) => internal_error(e).into_response(),
    }
}

async fn api_fire_deployment(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, slug)): Path<(String, String)>,
) -> impl IntoResponse {
    if let Err(e) = require_api_role(&token, "editor") {
        return e.into_response();
    }
    if let Err(e) = require_token_app(&token, &app) {
        return e.into_response();
    }

    let dep = match state
        .audit
        .get_deployment_by_slug_and_app(app.app.id, &slug)
        .await
    {
        Ok(Some(d)) => d,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Deployment not found"})),
            )
                .into_response();
        }
        Err(e) => {
            return internal_error(e).into_response();
        }
    };

    match crate::webhooks::fire_webhook(
        &state.http_client,
        &state.audit,
        &dep,
        crate::webhooks::TriggerSource::Manual,
        &app.app.slug,
    )
    .await
    {
        Ok(_) => {
            state.audit.log_with_app(
                "api",
                "deployment_fired",
                "deployment",
                &dep.slug,
                None,
                Some(app.app.id),
            );
            Json(serde_json::json!({"status": "triggered"})).into_response()
        }
        Err(e) => {
            state.audit.log_with_app(
                "api",
                "deployment_fired",
                "deployment",
                &dep.slug,
                None,
                Some(app.app.id),
            );
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response()
        }
    }
}

// Backup endpoints are intentionally global (no app-scoping). Backups cover the
// entire data directory, so any valid admin token may query or trigger them.
// The BearerToken extractor already validates the token exists in the database
// and require_api_role ensures the caller has admin privileges.
async fn api_backup_status(State(state): State<AppState>, token: BearerToken) -> impl IntoResponse {
    if let Err(e) = require_api_role(&token, "admin") {
        return e.into_response();
    }

    let config = match state.audit.get_backup_config().await {
        Ok(c) => c,
        Err(e) => {
            return internal_error(e).into_response();
        }
    };

    let latest_backup = state.audit.latest_backup().await.ok().flatten();
    let backup_running = state.backup_running.load(Ordering::SeqCst);

    Json(serde_json::json!({
        "s3_configured": state.s3_config.is_some(),
        "config": {
            "frequency_hours": config.frequency_hours,
            "retention_count": config.retention_count,
            "enabled": config.enabled,
        },
        "backup_running": backup_running,
        "latest_backup": latest_backup,
    }))
    .into_response()
}

async fn api_trigger_backup(
    State(state): State<AppState>,
    token: BearerToken,
) -> impl IntoResponse {
    if let Err(e) = require_api_role(&token, "admin") {
        return e.into_response();
    }

    if state.s3_config.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "S3 backup not configured"})),
        )
            .into_response();
    }

    if state.backup_running.load(Ordering::SeqCst) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "Backup already in progress"})),
        )
            .into_response();
    }

    if let Some(tx) = &state.backup_trigger
        && tx.try_send(()).is_err()
    {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "Backup already in progress"})),
        )
            .into_response();
    }

    state.audit.log(
        "api",
        "backup_triggered",
        "backup",
        "",
        Some(&serde_json::json!({"trigger": "manual"}).to_string()),
    );

    Json(serde_json::json!({"status": "triggered"})).into_response()
}
