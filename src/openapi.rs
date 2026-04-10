use std::path::Path;

use serde_json::{Value, json};

use crate::schema;

/// Generate a complete OpenAPI 3.1 spec for the Substrukt API.
///
/// The static portion covers all fixed routes. The dynamic portion generates
/// content CRUD paths per app/schema with request/response bodies derived
/// from user-defined JSON Schemas.
pub fn generate_spec(data_dir: &Path) -> Value {
    let mut paths = serde_json::Map::new();

    // -- Static global routes --
    add_openapi_spec_path(&mut paths);
    add_backup_paths(&mut paths);

    // -- Dynamic per-app routes --
    if data_dir.exists()
        && let Ok(entries) = std::fs::read_dir(data_dir)
    {
        for dir_entry in entries.flatten() {
            if !dir_entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                continue;
            }
            let app_dir = dir_entry.path();
            let schemas_dir = app_dir.join("schemas");
            if !schemas_dir.exists() {
                continue;
            }
            let app_slug = dir_entry.file_name().to_string_lossy().to_string();
            add_app_paths(&mut paths, &app_slug, &schemas_dir);
        }
    }

    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Substrukt API",
            "description": "Schema-driven CMS API. Content endpoints are dynamically generated from user-defined JSON Schemas.",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "servers": [
            { "url": "/api/v1", "description": "API v1" }
        ],
        "components": {
            "securitySchemes": {
                "bearerAuth": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "API token created through the Substrukt UI. Tokens inherit the role of the user who created them (viewer, editor, or admin)."
                }
            },
            "schemas": {
                "Error": {
                    "type": "object",
                    "properties": {
                        "error": { "type": "string" }
                    },
                    "required": ["error"]
                },
                "ValidationErrors": {
                    "type": "object",
                    "properties": {
                        "errors": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    },
                    "required": ["errors"]
                }
            }
        },
        "security": [
            { "bearerAuth": [] }
        ],
        "paths": paths,
    })
}

fn bearer_security() -> Value {
    json!([{ "bearerAuth": [] }])
}

fn error_responses() -> Value {
    json!({
        "401": {
            "description": "Unauthorized -- missing or invalid bearer token"
        },
        "403": {
            "description": "Forbidden -- insufficient role permissions"
        },
        "500": {
            "description": "Internal server error",
            "content": {
                "application/json": {
                    "schema": { "$ref": "#/components/schemas/Error" }
                }
            }
        }
    })
}

fn add_openapi_spec_path(paths: &mut serde_json::Map<String, Value>) {
    paths.insert(
        "/openapi.json".to_string(),
        json!({
            "get": {
                "summary": "OpenAPI specification",
                "description": "Returns the full OpenAPI 3.1 spec for the Substrukt API. No authentication required.",
                "operationId": "getOpenApiSpec",
                "security": [],
                "responses": {
                    "200": {
                        "description": "OpenAPI spec",
                        "content": {
                            "application/json": {
                                "schema": { "type": "object" }
                            }
                        }
                    }
                }
            }
        }),
    );
}

fn add_backup_paths(paths: &mut serde_json::Map<String, Value>) {
    paths.insert(
        "/backups/status".to_string(),
        json!({
            "get": {
                "summary": "Get backup status",
                "description": "Returns current backup configuration and status. Requires admin role.",
                "operationId": "getBackupStatus",
                "security": bearer_security(),
                "responses": merge_responses(json!({
                    "200": {
                        "description": "Backup status",
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "properties": {
                                        "s3_configured": { "type": "boolean" },
                                        "config": {
                                            "type": "object",
                                            "properties": {
                                                "frequency_hours": { "type": "integer" },
                                                "retention_count": { "type": "integer" },
                                                "enabled": { "type": "boolean" }
                                            }
                                        },
                                        "backup_running": { "type": "boolean" },
                                        "latest_backup": {}
                                    }
                                }
                            }
                        }
                    }
                }))
            }
        }),
    );

    paths.insert(
        "/backups/trigger".to_string(),
        json!({
            "post": {
                "summary": "Trigger a backup",
                "description": "Manually trigger an S3 backup. Requires admin role.",
                "operationId": "triggerBackup",
                "security": bearer_security(),
                "responses": merge_responses(json!({
                    "200": {
                        "description": "Backup triggered",
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "properties": {
                                        "status": { "type": "string" }
                                    }
                                }
                            }
                        }
                    },
                    "400": {
                        "description": "S3 backup not configured"
                    },
                    "409": {
                        "description": "Backup already in progress"
                    }
                }))
            }
        }),
    );
}

fn add_app_paths(paths: &mut serde_json::Map<String, Value>, app_slug: &str, schemas_dir: &Path) {
    let prefix = format!("/apps/{app_slug}");

    // Static app-scoped routes
    add_schema_routes(paths, &prefix);
    add_upload_routes(paths, &prefix);
    add_export_import_routes(paths, &prefix);
    add_deployment_routes(paths, &prefix);

    // Dynamic content routes per schema
    if let Ok(schemas) = schema::list_schemas(schemas_dir) {
        for s in &schemas {
            add_content_routes(paths, &prefix, &s.meta, &s.schema);
        }
    }
}

fn add_schema_routes(paths: &mut serde_json::Map<String, Value>, prefix: &str) {
    paths.insert(
        format!("{prefix}/schemas"),
        json!({
            "get": {
                "summary": "List schemas",
                "description": "List all schemas for this app. Requires viewer role or above.",
                "operationId": format!("listSchemas_{}", prefix.replace('/', "_")),
                "security": bearer_security(),
                "responses": merge_responses(json!({
                    "200": {
                        "description": "Array of schemas",
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "title": { "type": "string" },
                                            "slug": { "type": "string" },
                                            "storage": { "type": "string", "enum": ["directory", "single-file"] },
                                            "kind": { "type": "string", "enum": ["single", "collection"] },
                                            "schema": { "type": "object" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }))
            }
        }),
    );

    paths.insert(
        format!("{prefix}/schemas/{{slug}}"),
        json!({
            "get": {
                "summary": "Get schema",
                "description": "Get a single schema by slug. Requires viewer role or above.",
                "operationId": format!("getSchema_{}", prefix.replace('/', "_")),
                "security": bearer_security(),
                "parameters": [
                    {
                        "name": "slug",
                        "in": "path",
                        "required": true,
                        "schema": { "type": "string" }
                    }
                ],
                "responses": merge_responses(json!({
                    "200": {
                        "description": "JSON Schema",
                        "content": {
                            "application/json": {
                                "schema": { "type": "object" }
                            }
                        }
                    },
                    "404": { "description": "Schema not found" }
                }))
            }
        }),
    );
}

fn add_upload_routes(paths: &mut serde_json::Map<String, Value>, prefix: &str) {
    paths.insert(
        format!("{prefix}/uploads"),
        json!({
            "post": {
                "summary": "Upload a file",
                "description": "Upload a file via multipart form data. Requires editor role or above.",
                "operationId": format!("uploadFile_{}", prefix.replace('/', "_")),
                "security": bearer_security(),
                "requestBody": {
                    "required": true,
                    "content": {
                        "multipart/form-data": {
                            "schema": {
                                "type": "object",
                                "properties": {
                                    "file": {
                                        "type": "string",
                                        "format": "binary"
                                    }
                                }
                            }
                        }
                    }
                },
                "responses": merge_responses(json!({
                    "200": {
                        "description": "Upload metadata",
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "properties": {
                                        "hash": { "type": "string" },
                                        "filename": { "type": "string" },
                                        "mime": { "type": "string" },
                                        "size": { "type": "integer" }
                                    }
                                }
                            }
                        }
                    }
                }))
            }
        }),
    );

    paths.insert(
        format!("{prefix}/uploads/{{hash}}"),
        json!({
            "get": {
                "summary": "Get upload by hash",
                "description": "Retrieve an uploaded file by its content hash. Requires viewer role or above.",
                "operationId": format!("getUpload_{}", prefix.replace('/', "_")),
                "security": bearer_security(),
                "parameters": [
                    {
                        "name": "hash",
                        "in": "path",
                        "required": true,
                        "schema": { "type": "string" }
                    }
                ],
                "responses": merge_responses(json!({
                    "200": {
                        "description": "File content"
                    },
                    "404": { "description": "Upload not found" }
                }))
            }
        }),
    );
}

fn add_export_import_routes(paths: &mut serde_json::Map<String, Value>, prefix: &str) {
    paths.insert(
        format!("{prefix}/export"),
        json!({
            "post": {
                "summary": "Export app bundle",
                "description": "Export the entire app (schemas, content, uploads) as a tar.gz bundle. Requires admin role.",
                "operationId": format!("exportBundle_{}", prefix.replace('/', "_")),
                "security": bearer_security(),
                "responses": merge_responses(json!({
                    "200": {
                        "description": "Bundle tar.gz file",
                        "content": {
                            "application/gzip": {
                                "schema": {
                                    "type": "string",
                                    "format": "binary"
                                }
                            }
                        }
                    }
                }))
            }
        }),
    );

    paths.insert(
        format!("{prefix}/import"),
        json!({
            "post": {
                "summary": "Import app bundle",
                "description": "Import a tar.gz bundle to replace app content. Requires admin role.",
                "operationId": format!("importBundle_{}", prefix.replace('/', "_")),
                "security": bearer_security(),
                "requestBody": {
                    "required": true,
                    "content": {
                        "multipart/form-data": {
                            "schema": {
                                "type": "object",
                                "properties": {
                                    "file": {
                                        "type": "string",
                                        "format": "binary"
                                    }
                                }
                            }
                        }
                    }
                },
                "responses": merge_responses(json!({
                    "200": {
                        "description": "Import result",
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "properties": {
                                        "status": { "type": "string" },
                                        "warnings": {
                                            "type": "array",
                                            "items": { "type": "string" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }))
            }
        }),
    );
}

fn add_deployment_routes(paths: &mut serde_json::Map<String, Value>, prefix: &str) {
    paths.insert(
        format!("{prefix}/deployments"),
        json!({
            "get": {
                "summary": "List deployments",
                "description": "List all deployment targets for this app. Requires viewer role or above.",
                "operationId": format!("listDeployments_{}", prefix.replace('/', "_")),
                "security": bearer_security(),
                "responses": merge_responses(json!({
                    "200": {
                        "description": "Array of deployments",
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "name": { "type": "string" },
                                            "slug": { "type": "string" },
                                            "webhook_url": { "type": "string" },
                                            "include_drafts": { "type": "boolean" },
                                            "auto_deploy": { "type": "boolean" },
                                            "debounce_seconds": { "type": "integer" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }))
            }
        }),
    );

    paths.insert(
        format!("{prefix}/deployments/{{slug}}/fire"),
        json!({
            "post": {
                "summary": "Fire deployment",
                "description": "Trigger a deployment webhook. Requires editor role or above.",
                "operationId": format!("fireDeployment_{}", prefix.replace('/', "_")),
                "security": bearer_security(),
                "parameters": [
                    {
                        "name": "slug",
                        "in": "path",
                        "required": true,
                        "schema": { "type": "string" }
                    }
                ],
                "responses": merge_responses(json!({
                    "200": {
                        "description": "Deployment triggered",
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "properties": {
                                        "status": { "type": "string" }
                                    }
                                }
                            }
                        }
                    },
                    "404": { "description": "Deployment not found" },
                    "502": {
                        "description": "Webhook request failed",
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/Error" }
                            }
                        }
                    }
                }))
            }
        }),
    );
}

fn add_content_routes(
    paths: &mut serde_json::Map<String, Value>,
    prefix: &str,
    meta: &schema::models::SubstruktMeta,
    user_schema: &Value,
) {
    let slug = &meta.slug;
    let title = &meta.title;
    let is_single = meta.kind == schema::models::Kind::Single;

    // Extract just the properties portion of the user's schema for request bodies
    let content_schema = extract_content_schema(user_schema);

    if is_single {
        // Single content type: GET/PUT/DELETE on /content/{slug}/single
        add_single_content_routes(paths, prefix, slug, title, &content_schema);
    } else {
        // Collection content type: CRUD on /content/{slug} and /content/{slug}/{entry_id}
        add_collection_content_routes(paths, prefix, slug, title, &content_schema);
    }
}

fn add_single_content_routes(
    paths: &mut serde_json::Map<String, Value>,
    prefix: &str,
    slug: &str,
    title: &str,
    content_schema: &Value,
) {
    let op_suffix = format!("{}_{}", prefix.replace('/', "_"), slug);

    paths.insert(
        format!("{prefix}/content/{slug}/single"),
        json!({
            "get": {
                "summary": format!("Get {title}"),
                "description": format!("Get the single {title} entry. Requires viewer role or above."),
                "operationId": format!("getSingle_{op_suffix}"),
                "security": bearer_security(),
                "parameters": [
                    {
                        "name": "status",
                        "in": "query",
                        "required": false,
                        "schema": { "type": "string", "enum": ["published", "draft", "all"], "default": "published" },
                        "description": "Filter by publish status"
                    },
                    {
                        "name": "render",
                        "in": "query",
                        "required": false,
                        "schema": { "type": "string", "enum": ["html"] },
                        "description": "Set to 'html' to render markdown fields as HTML in the response"
                    }
                ],
                "responses": merge_responses(json!({
                    "200": {
                        "description": format!("{title} entry"),
                        "content": {
                            "application/json": {
                                "schema": content_schema
                            }
                        }
                    },
                    "404": { "description": "Not found" }
                }))
            },
            "put": {
                "summary": format!("Upsert {title}"),
                "description": format!("Create or update the single {title} entry. Requires editor role or above."),
                "operationId": format!("upsertSingle_{op_suffix}"),
                "security": bearer_security(),
                "requestBody": {
                    "required": true,
                    "content": {
                        "application/json": {
                            "schema": content_schema
                        }
                    }
                },
                "responses": merge_responses(json!({
                    "200": { "description": "Entry saved" },
                    "400": {
                        "description": "Validation error",
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/ValidationErrors" }
                            }
                        }
                    }
                }))
            },
            "delete": {
                "summary": format!("Delete {title}"),
                "description": format!("Delete the single {title} entry. Requires editor role or above."),
                "operationId": format!("deleteSingle_{op_suffix}"),
                "security": bearer_security(),
                "responses": merge_responses(json!({
                    "204": { "description": "Deleted" },
                    "404": { "description": "Not found" }
                }))
            }
        }),
    );
}

fn add_collection_content_routes(
    paths: &mut serde_json::Map<String, Value>,
    prefix: &str,
    slug: &str,
    title: &str,
    content_schema: &Value,
) {
    let op_suffix = format!("{}_{}", prefix.replace('/', "_"), slug);

    // List + Create
    paths.insert(
        format!("{prefix}/content/{slug}"),
        json!({
            "get": {
                "summary": format!("List {title} entries"),
                "description": format!("List entries for {title}. Requires viewer role or above."),
                "operationId": format!("listEntries_{op_suffix}"),
                "security": bearer_security(),
                "parameters": [
                    {
                        "name": "q",
                        "in": "query",
                        "required": false,
                        "schema": { "type": "string" },
                        "description": "Search query"
                    },
                    {
                        "name": "status",
                        "in": "query",
                        "required": false,
                        "schema": { "type": "string", "enum": ["published", "draft", "all"], "default": "published" },
                        "description": "Filter by publish status"
                    },
                    {
                        "name": "render",
                        "in": "query",
                        "required": false,
                        "schema": { "type": "string", "enum": ["html"] },
                        "description": "Set to 'html' to render markdown fields as HTML in the response"
                    }
                ],
                "responses": merge_responses(json!({
                    "200": {
                        "description": format!("Array of {title} entries"),
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "array",
                                    "items": content_schema
                                }
                            }
                        }
                    }
                }))
            },
            "post": {
                "summary": format!("Create {title} entry"),
                "description": format!("Create a new {title} entry. Requires editor role or above."),
                "operationId": format!("createEntry_{op_suffix}"),
                "security": bearer_security(),
                "requestBody": {
                    "required": true,
                    "content": {
                        "application/json": {
                            "schema": content_schema
                        }
                    }
                },
                "responses": merge_responses(json!({
                    "201": {
                        "description": "Entry created",
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "properties": {
                                        "id": { "type": "string" }
                                    }
                                }
                            }
                        }
                    },
                    "400": {
                        "description": "Validation error",
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/ValidationErrors" }
                            }
                        }
                    }
                }))
            }
        }),
    );

    // Get + Update + Delete single entry
    paths.insert(
        format!("{prefix}/content/{slug}/{{entry_id}}"),
        json!({
            "get": {
                "summary": format!("Get {title} entry"),
                "description": format!("Get a single {title} entry by ID. Requires viewer role or above."),
                "operationId": format!("getEntry_{op_suffix}"),
                "security": bearer_security(),
                "parameters": [
                    {
                        "name": "entry_id",
                        "in": "path",
                        "required": true,
                        "schema": { "type": "string" }
                    },
                    {
                        "name": "render",
                        "in": "query",
                        "required": false,
                        "schema": { "type": "string", "enum": ["html"] },
                        "description": "Set to 'html' to render markdown fields as HTML in the response"
                    }
                ],
                "responses": merge_responses(json!({
                    "200": {
                        "description": format!("{title} entry"),
                        "content": {
                            "application/json": {
                                "schema": content_schema
                            }
                        }
                    },
                    "404": { "description": "Entry not found" }
                }))
            },
            "put": {
                "summary": format!("Update {title} entry"),
                "description": format!("Update an existing {title} entry. Requires editor role or above."),
                "operationId": format!("updateEntry_{op_suffix}"),
                "security": bearer_security(),
                "parameters": [
                    {
                        "name": "entry_id",
                        "in": "path",
                        "required": true,
                        "schema": { "type": "string" }
                    }
                ],
                "requestBody": {
                    "required": true,
                    "content": {
                        "application/json": {
                            "schema": content_schema
                        }
                    }
                },
                "responses": merge_responses(json!({
                    "200": { "description": "Entry updated" },
                    "400": {
                        "description": "Validation error",
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/ValidationErrors" }
                            }
                        }
                    },
                    "404": { "description": "Entry not found" }
                }))
            },
            "delete": {
                "summary": format!("Delete {title} entry"),
                "description": format!("Delete a {title} entry. Requires editor role or above."),
                "operationId": format!("deleteEntry_{op_suffix}"),
                "security": bearer_security(),
                "parameters": [
                    {
                        "name": "entry_id",
                        "in": "path",
                        "required": true,
                        "schema": { "type": "string" }
                    }
                ],
                "responses": merge_responses(json!({
                    "204": { "description": "Deleted" },
                    "404": { "description": "Entry not found" }
                }))
            }
        }),
    );

    // Publish / Unpublish
    paths.insert(
        format!("{prefix}/content/{slug}/{{entry_id}}/publish"),
        json!({
            "post": {
                "summary": format!("Publish {title} entry"),
                "description": format!("Set a {title} entry's status to published. Requires editor role or above."),
                "operationId": format!("publishEntry_{op_suffix}"),
                "security": bearer_security(),
                "parameters": [
                    {
                        "name": "entry_id",
                        "in": "path",
                        "required": true,
                        "schema": { "type": "string" }
                    }
                ],
                "responses": merge_responses(json!({
                    "200": {
                        "description": "Entry published",
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "properties": {
                                        "status": { "type": "string" },
                                        "entry_id": { "type": "string" }
                                    }
                                }
                            }
                        }
                    },
                    "404": { "description": "Entry not found" }
                }))
            }
        }),
    );

    paths.insert(
        format!("{prefix}/content/{slug}/{{entry_id}}/unpublish"),
        json!({
            "post": {
                "summary": format!("Unpublish {title} entry"),
                "description": format!("Set a {title} entry's status to draft. Requires editor role or above."),
                "operationId": format!("unpublishEntry_{op_suffix}"),
                "security": bearer_security(),
                "parameters": [
                    {
                        "name": "entry_id",
                        "in": "path",
                        "required": true,
                        "schema": { "type": "string" }
                    }
                ],
                "responses": merge_responses(json!({
                    "200": {
                        "description": "Entry unpublished",
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "properties": {
                                        "status": { "type": "string" },
                                        "entry_id": { "type": "string" }
                                    }
                                }
                            }
                        }
                    },
                    "404": { "description": "Entry not found" }
                }))
            }
        }),
    );
}

/// Extract the content-relevant parts of a user-defined JSON Schema
/// for use as request/response bodies. Strips the x-substrukt extension
/// and keeps properties, required, type, etc.
fn extract_content_schema(user_schema: &Value) -> Value {
    let mut schema = user_schema.clone();
    if let Some(obj) = schema.as_object_mut() {
        obj.remove("x-substrukt");
        // Remove $schema, $id if present -- they refer to the JSON Schema meta-schema
        obj.remove("$schema");
        obj.remove("$id");
    }
    schema
}

/// Merge endpoint-specific responses with common error responses.
fn merge_responses(mut specific: Value) -> Value {
    let errors = error_responses();
    if let (Some(specific_obj), Some(error_obj)) = (specific.as_object_mut(), errors.as_object()) {
        for (k, v) in error_obj {
            // Don't override endpoint-specific error codes
            if !specific_obj.contains_key(k) {
                specific_obj.insert(k.clone(), v.clone());
            }
        }
    }
    specific
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_schema_dir(dir: &std::path::Path) {
        let schemas_dir = dir.join("test-app").join("schemas");
        std::fs::create_dir_all(&schemas_dir).unwrap();

        let schema = json!({
            "type": "object",
            "x-substrukt": {
                "title": "Blog Posts",
                "slug": "posts",
                "kind": "collection"
            },
            "properties": {
                "title": { "type": "string" },
                "body": { "type": "string" }
            },
            "required": ["title"]
        });

        std::fs::write(
            schemas_dir.join("posts.json"),
            serde_json::to_string_pretty(&schema).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn test_generate_spec_structure() {
        let dir = tempfile::tempdir().unwrap();
        create_test_schema_dir(dir.path());

        let spec = generate_spec(dir.path());

        // Check top-level structure
        assert_eq!(spec["openapi"], "3.1.0");
        assert!(spec["info"]["title"].is_string());
        assert!(spec["paths"].is_object());
        assert!(spec["components"]["securitySchemes"]["bearerAuth"].is_object());
    }

    #[test]
    fn test_generate_spec_has_static_routes() {
        let dir = tempfile::tempdir().unwrap();
        let spec = generate_spec(dir.path());

        let paths = spec["paths"].as_object().unwrap();
        assert!(paths.contains_key("/openapi.json"));
        assert!(paths.contains_key("/backups/status"));
        assert!(paths.contains_key("/backups/trigger"));
    }

    #[test]
    fn test_generate_spec_has_dynamic_content_routes() {
        let dir = tempfile::tempdir().unwrap();
        create_test_schema_dir(dir.path());

        let spec = generate_spec(dir.path());
        let paths = spec["paths"].as_object().unwrap();

        // Should have app-scoped content routes
        assert!(paths.contains_key("/apps/test-app/content/posts"));
        assert!(paths.contains_key("/apps/test-app/content/posts/{entry_id}"));
        assert!(paths.contains_key("/apps/test-app/content/posts/{entry_id}/publish"));
        assert!(paths.contains_key("/apps/test-app/content/posts/{entry_id}/unpublish"));
        assert!(paths.contains_key("/apps/test-app/schemas"));
    }

    #[test]
    fn test_content_schema_strips_extensions() {
        let user_schema = json!({
            "type": "object",
            "x-substrukt": { "title": "Test", "slug": "test" },
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "properties": {
                "name": { "type": "string" }
            }
        });

        let content = extract_content_schema(&user_schema);
        assert!(content.get("x-substrukt").is_none());
        assert!(content.get("$schema").is_none());
        assert!(content.get("properties").is_some());
    }

    #[test]
    fn test_single_schema_generates_single_routes() {
        let dir = tempfile::tempdir().unwrap();
        let schemas_dir = dir.path().join("myapp").join("schemas");
        std::fs::create_dir_all(&schemas_dir).unwrap();

        let schema = json!({
            "type": "object",
            "x-substrukt": {
                "title": "Settings",
                "slug": "settings",
                "kind": "single"
            },
            "properties": {
                "site_name": { "type": "string" }
            }
        });

        std::fs::write(
            schemas_dir.join("settings.json"),
            serde_json::to_string_pretty(&schema).unwrap(),
        )
        .unwrap();

        let spec = generate_spec(dir.path());
        let paths = spec["paths"].as_object().unwrap();

        // Single schemas get /single route, not collection CRUD routes
        assert!(paths.contains_key("/apps/myapp/content/settings/single"));
        assert!(!paths.contains_key("/apps/myapp/content/settings"));
    }

    #[test]
    fn test_openapi_spec_no_auth_on_spec_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let spec = generate_spec(dir.path());

        let openapi_path = &spec["paths"]["/openapi.json"]["get"];
        assert_eq!(openapi_path["security"], json!([]));
    }
}
