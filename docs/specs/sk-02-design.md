# SK-02: Bulk Operations (API and Admin)

## Motivation

Content migration is a critical workflow for CMS adoption. Users switching from Payload CMS, Notion, or other systems need to import dozens or hundreds of entries at once. Currently, entries can only be created or updated one at a time via the API. The admin UI supports multi-select for publish/unpublish/delete, but there is no bulk create or bulk update path anywhere in the system.

This spec designs bulk create, update, and delete operations for the REST API and extends the admin UI with an import flow for content migration.

## Existing Baseline

### Admin UI Bulk Operations (already built)

**Routes** (`src/routes/content.rs:1154+`):
- `POST /{schema_slug}/_bulk/publish` -- publishes selected entries via `set_entry_status`.
- `POST /{schema_slug}/_bulk/unpublish` -- unpublishes selected entries.
- `POST /{schema_slug}/_bulk/delete` -- deletes selected entries, cleans up upload references, history, and cache.

**Form** (`BulkForm { ids: String }`): comma-separated entry IDs from hidden form fields, populated by JavaScript.

**UI** (`templates/content/list.html`):
- Checkbox column with select-all toggle.
- Floating bulk action bar appears when entries are selected, showing Publish/Unpublish/Delete buttons.
- Per-item iteration server-side; count-based flash message on completion.

**Behavior:**
- Best-effort: failures on individual entries are silently skipped; only successful count is reported.
- Per-item audit logging (`entry_published`, `entry_unpublished`, `content_delete`).
- Per-item cache reload/removal.
- Delete also calls `delete_history` and `db_delete_references`.

### API (nothing exists for bulk)

All API content operations in `src/routes/api.rs` are single-entry:
- `POST /content/{schema_slug}` -- create one entry.
- `PUT /content/{schema_slug}/{entry_id}` -- update one entry.
- `DELETE /content/{schema_slug}/{entry_id}` -- delete one entry.

No batch endpoints exist.

## Architecture

### 1. API Bulk Endpoints

Three new endpoints under the existing API app routes:

```
POST /content/{schema_slug}/_bulk/create
POST /content/{schema_slug}/_bulk/update
POST /content/{schema_slug}/_bulk/delete
```

All require **editor+** role. All accept JSON request bodies.

#### Design Decisions

**Partial success semantics (207-style response).** For content migration, fail-fast is unacceptable -- a single validation error in entry #3 shouldn't prevent entries #1, #2, #4-100 from being created. The API uses best-effort processing with per-item results, returned as a 200 with a results array. HTTP 207 Multi-Status is semantically correct but less widely understood by client tooling; a 200 with explicit `results` is more practical.

**Validate-then-write per item, not all-then-write.** Each item is validated and written independently. This keeps memory usage linear (no need to buffer all validated entries before writing) and gives the most useful per-item error feedback.

**Single-file storage optimization.** The current `save_entry` for `SingleFile` mode rewrites the entire file on every call. A bulk create of 100 entries would rewrite the file 100 times. The bulk endpoints must batch single-file operations: load once, apply all mutations in memory, write once. This requires a new internal function (see Data Models).

**Upload fields.** Binary uploads cannot be embedded in a JSON array. Bulk create/update expects upload fields to reference pre-existing upload hashes (the `{hash, filename, mime}` object format). Callers must upload files via `POST /uploads` first, then reference them in bulk payloads. This is already the natural flow for API usage.

#### Bulk Create

```
POST /api/v1/apps/{app}/content/{schema_slug}/_bulk/create
```

**Request body:**
```json
{
  "entries": [
    { "title": "First Post", "body": "Content..." },
    { "title": "Second Post", "body": "More content..." }
  ]
}
```

**Response (200):**
```json
{
  "total": 2,
  "created": 2,
  "failed": 0,
  "results": [
    { "index": 0, "status": "created", "id": "first-post" },
    { "index": 1, "status": "created", "id": "second-post" }
  ]
}
```

**Response with partial failure:**
```json
{
  "total": 3,
  "created": 2,
  "failed": 1,
  "results": [
    { "index": 0, "status": "created", "id": "first-post" },
    { "index": 1, "status": "error", "errors": ["title: required field missing"] },
    { "index": 2, "status": "created", "id": "third-post" }
  ]
}
```

**ID generation:** Uses the existing `generate_entry_id` logic (slugify title field, fall back to UUID). If a generated ID collides with an existing entry or a previously-created entry in the same batch, a UUID suffix is appended: `my-title-a1b2c3d4`. The collision check is done in-memory during batch processing.

**Constraints:**
- Rejects `Kind::Single` schemas (use `PUT /content/{slug}/single` instead).
- Maximum entries per request: 500 (configurable via constant). Beyond this, callers should paginate.
- Bounded by `max_body_size` (default 50MB).

#### Bulk Update

```
POST /api/v1/apps/{app}/content/{schema_slug}/_bulk/update
```

**Request body:**
```json
{
  "entries": [
    { "_id": "first-post", "title": "Updated Title", "body": "Updated content" },
    { "_id": "second-post", "title": "Also Updated" }
  ]
}
```

Each entry must include `_id` to identify the target. The `_id` field is stripped before saving (for directory mode entries where ID is the filename, not a data field).

**Response:** Same format as bulk create, with `"status": "updated"` for successes. Entries with non-existent IDs return `"status": "error", "errors": ["entry not found"]`.

**Version snapshots:** Each updated entry gets a history snapshot before overwriting, using the existing `snapshot_entry` function. For single-file mode, snapshots are taken per-entry before the batched write.

#### Bulk Delete

```
POST /api/v1/apps/{app}/content/{schema_slug}/_bulk/delete
```

**Request body:**
```json
{
  "ids": ["first-post", "second-post", "nonexistent"]
}
```

**Response:**
```json
{
  "total": 3,
  "deleted": 2,
  "failed": 1,
  "results": [
    { "id": "first-post", "status": "deleted" },
    { "id": "second-post", "status": "deleted" },
    { "id": "nonexistent", "status": "error", "error": "entry not found" }
  ]
}
```

Cleans up upload references, history, and cache per entry (matches existing single-delete behavior).

#### Bulk Publish / Unpublish (API)

```
POST /api/v1/apps/{app}/content/{schema_slug}/_bulk/publish
POST /api/v1/apps/{app}/content/{schema_slug}/_bulk/unpublish
```

**Request body:** `{ "ids": ["id1", "id2"] }`

**Response:** Same results-array format. Mirrors admin UI behavior.

### 2. Single-File Batched Write

For `StorageMode::SingleFile` collections, individual `save_entry` calls rewrite the whole file per entry. Bulk operations need a batched version.

**New internal function** in `src/content/mod.rs`:

```rust
pub fn save_entries_batch(
    content_dir: &Path,
    schema: &SchemaFile,
    entries: Vec<(Option<&str>, Value)>,  // (entry_id, data)
) -> eyre::Result<Vec<Result<String, eyre::Error>>> { ... }
```

For directory mode: delegates to individual `save_entry` calls (each file is independent).
For single-file mode: loads the file once, applies all inserts/updates in memory, writes once.

### 3. Admin UI Import Flow

**Route:**

```
GET  /apps/{app}/content/{schema_slug}/import  -- import page
POST /apps/{app}/content/{schema_slug}/import  -- process import
```

**UI flow:**
1. Editor navigates to import page from the content list (new "Import" button next to "New Entry").
2. Page shows a textarea for pasting a JSON array and/or a file upload input for `.json` files.
3. On submit, server validates each entry against the schema.
4. Results page shows: entries to be created (with generated IDs), validation errors per entry.
5. User confirms to proceed. Entries are created via the batched write path.

**Template:** `content/import.html` with two-step flow (paste/upload -> preview -> confirm).

**Rationale for JSON over CSV:** The CMS stores JSON, schemas define JSON structures (including nested objects and arrays). JSON-to-JSON import is lossless and requires no field mapping. CSV import with field mapping is a significantly larger feature and is a non-goal for this iteration.

### 4. Audit Logging

**API bulk operations:** One audit log entry per bulk operation (not per item) with the count and operation type in the detail field. Example: `content_bulk_create` with detail `"schema_slug: 45 entries (42 created, 3 failed)"`. This avoids flooding the audit log during large imports.

**Admin UI bulk operations (existing):** Keep the current per-item audit logging for publish/unpublish/delete, since these are typically small selections (< 20 entries). The import flow should use the bulk audit pattern since it can involve hundreds of entries.

### 5. Auto-Deploy / Webhook Side Effects

The existing auto-deploy system detects dirty state by comparing the latest content-mutation audit log timestamp against the last-fired timestamp (`audit.rs:446-488`). Mutation actions checked: `content_create`, `content_update`, `content_delete`, `entry_published`, `entry_unpublished`, `schema_*`. Background tasks poll this dirty state with debounce.

Bulk API operations must audit-log their mutations so auto-deploy detects them. The per-bulk audit log entry (see Section 4) uses `content_bulk_create` / `content_bulk_update` / `content_bulk_delete` action names. These must be added to the dirty-detection query's `IN (...)` list in `audit.rs:464-467`, otherwise auto-deploy will not trigger after bulk operations.

The debounce timer naturally coalesces the single bulk audit event into one webhook fire. No other special handling is needed.

## Data Models

### API Request/Response Types

```rust
// Bulk create request
#[derive(Deserialize)]
struct BulkCreateRequest {
    entries: Vec<Value>,
}

// Bulk update request
#[derive(Deserialize)]
struct BulkUpdateRequest {
    entries: Vec<Value>,  // each must contain "_id"
}

// Bulk delete / publish / unpublish request
#[derive(Deserialize)]
struct BulkIdsRequest {
    ids: Vec<String>,
}

// Per-item result
#[derive(Serialize)]
struct BulkItemResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    status: String,  // "created", "updated", "deleted", "error"
    #[serde(skip_serializing_if = "Option::is_none")]
    errors: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

// Bulk operation response
#[derive(Serialize)]
struct BulkResponse {
    total: usize,
    #[serde(flatten)]
    counts: HashMap<String, usize>,  // "created"/"updated"/"deleted": N, "failed": M
    results: Vec<BulkItemResult>,
}
```

### Constants

```rust
const MAX_BULK_ENTRIES: usize = 500;
```

## Error Handling

| Scenario | Behavior |
|---|---|
| Schema not found | 404 before processing any entries. |
| Single-kind schema for bulk create | 400 `"This schema is a single. Use PUT /content/{slug}/single instead."` |
| Empty entries array | 400 `"entries array is empty"` |
| Exceeds MAX_BULK_ENTRIES | 400 `"Too many entries. Maximum is 500 per request."` |
| Individual entry validation failure | Item marked as error in results; other items proceed. |
| Disk write failure mid-batch | For directory mode, partial writes are possible (some files written, some not). Results reflect what succeeded. For single-file mode, the atomic write succeeds or fails as a unit. |
| Request body exceeds max_body_size | 413 Payload Too Large (handled by Axum's DefaultBodyLimit layer). |
| Entry ID not found on bulk update/delete | Item marked as error; others proceed. |

## Edge Cases

### ID collisions in bulk create
The existing `generate_entry_id` in `content/mod.rs:402-452` does NOT check for collisions -- it generates a slug from the first string field or falls back to UUID, but doesn't verify uniqueness. For single creates this is rarely a problem (users typically use distinct titles). For bulk creates, collisions are much more likely (e.g., multiple entries with `title: "Hello"` or entries that slugify identically).

The bulk create handler must add collision detection: after generating each ID, check it against both existing entries on disk and previously-generated IDs in the current batch. On collision, append `-{uuid_v4_first_8_chars}`.

Note: this collision behavior is specific to the bulk endpoint. The existing single-create `POST /content/{schema_slug}` retains its current behavior (no collision check). Adding collision handling there could be a separate improvement but is out of scope for this spec.

### Single-file storage and concurrent requests
Two concurrent bulk operations on the same single-file schema could race. The batched write reads the file, mutates, and writes -- a concurrent write between read and write would be lost. This is the same race condition that exists for single-entry writes today. Mitigation is out of scope (would require file locking or a write queue). The risk is low because admin operations are low-concurrency by nature.

### Bulk delete with upload references
Each deleted entry has its upload references cleaned up via `db_delete_references`. For large bulk deletes, this is many sequential SQLite operations. Acceptable for now; a batched SQL `DELETE WHERE entry_id IN (...)` could be a future optimization.

### BulkForm comma-separated IDs
The existing admin `BulkForm { ids: String }` splits on commas. Entry IDs are either slugified strings or UUIDs -- neither contains commas, so this format is safe. No change needed.

### History snapshots during bulk update
Each entry being updated gets an individual snapshot. For 100 entries, this creates 100 snapshot files. This is correct -- each entry has its own history timeline. The `version_history_count` cap prevents unbounded growth.

### Import of entries with _status
Imported entries may include `_status`. If present and valid ("draft"/"published"), it is respected. If absent, defaults to "draft" (matches `save_entry` behavior for new entries).

### CSRF protection on admin import
The admin import form (`POST /apps/{app}/content/{schema_slug}/import`) must include CSRF token validation, consistent with all other admin POST routes. The existing pattern: hidden `_csrf` input in the form, verified via `auth::verify_csrf_token` in the handler.

### API route ordering
The new bulk routes use `_bulk` as a path segment that could collide with an entry ID of `_bulk`. Register `/_bulk/create`, `/_bulk/update`, `/_bulk/delete`, `/_bulk/publish`, `/_bulk/unpublish` routes **before** the existing `/{entry_id}` catch-all route in `api_app_routes()` to ensure correct matching. This matches the existing pattern where `/{entry_id}/publish` and `/{entry_id}/unpublish` are registered before `/{entry_id}`.

### Entry ID `_bulk` as content
If a user creates an entry with an ID that resolves to `_bulk` (e.g., title "Bulk"), the existing single-entry routes at `/{entry_id}` would still match for GET/PUT/DELETE since those are registered after the `_bulk` routes. However, the admin UI routes at `routes/content.rs` already use `_bulk` for the existing publish/unpublish/delete handlers (line 51-62). Entry IDs starting with `_` should be avoided. The existing `generate_entry_id` function uses `slug::slugify` which produces lowercase-hyphenated slugs (no underscores or leading special chars), so auto-generated IDs won't collide. API callers setting explicit `_id: "_bulk"` would hit the bulk route -- document `_bulk` as a reserved path segment.

## Non-Goals

- **Transactional rollback on partial failure** -- if 50 of 100 entries succeed before a disk error, the 50 stay created. The results array tells the caller exactly what happened. CMS content is not a database -- eventual consistency is acceptable.
- **Idempotency keys** -- bulk operations are not idempotent. Callers should use the results to avoid double-creation. A future enhancement could accept client-provided IDs to enable idempotent retry.
- **Dry-run / preview mode** -- validate-only without writing. Could be useful but adds complexity. The admin import flow provides a preview step; the API does not.
- **Cross-schema bulk operations** -- each bulk call targets one schema. Multi-schema imports (e.g., full-site migration) should use the existing export/import bundle system.
- **CSV import with field mapping** -- too complex for v1. JSON import is lossless and aligns with the system's data format.
- **Streaming / chunked bulk operations** -- the entire request body is parsed into memory. Bounded by `max_body_size` (default 50MB) and `MAX_BULK_ENTRIES` (500).
- **Admin UI bulk update** -- the admin UI only supports bulk status changes and delete. Bulk content editing through the UI is a spreadsheet-like feature that is out of scope.

## Implementation Sequence

1. **`save_entries_batch`** in `src/content/mod.rs` -- batched write for single-file mode, pass-through for directory mode.
2. **API bulk create endpoint** -- `POST /_bulk/create` with validation, ID generation, collision handling, results array.
3. **API bulk update endpoint** -- `POST /_bulk/update` with per-entry snapshots.
4. **API bulk delete endpoint** -- `POST /_bulk/delete` with upload/history cleanup.
5. **API bulk publish/unpublish endpoints** -- `POST /_bulk/publish` and `/_bulk/unpublish`.
6. **Admin import flow** -- route, template, two-step UI (paste/upload -> preview -> confirm).
7. **Tests** -- unit tests for `save_entries_batch`, integration tests for each API endpoint.
