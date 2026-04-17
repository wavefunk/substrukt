# SK-01: Content Versioning and History

## Status

Mostly implemented. This spec documents the existing system and designs the remaining gaps: version diffing, version preview, API surface for history, and authorship metadata.

## Motivation

Content edits in Substrukt overwrite JSON files on disk. A misguided edit, accidental field clearing, or bulk operation can destroy data with no way to recover. The versioning system preserves previous states of each entry so editors can review what changed, compare versions, and revert to any prior state.

## Existing Baseline

A significant portion of this feature is already implemented. The spec below documents what exists and designs only the gaps.

### What is already built

**Storage layer** (`src/history.rs`):
- `snapshot_entry(data_dir, schema_slug, entry_id, current_data, max_versions)` -- saves a timestamped JSON snapshot at `data/<app>/_history/<schema_slug>/<entry_id>/<timestamp_ms>.json` before overwriting. Prunes to keep at most `max_versions` snapshots (oldest removed first).
- `list_versions(data_dir, schema_slug, entry_id)` -- returns `Vec<VersionInfo { timestamp, size }>` sorted newest-first.
- `get_version(data_dir, schema_slug, entry_id, timestamp)` -- loads a specific snapshot by timestamp.
- `delete_history(data_dir, schema_slug, entry_id)` -- removes all history for an entry (called on delete).
- `prune_versions(dir, max_versions)` -- internal; removes oldest files beyond the cap.

**Configuration** (`src/config.rs`, `src/main.rs`):
- `Config.version_history_count` (default: 10, CLI flag `--version-history-count`). Global -- applies to all apps and schemas.
- `Config.app_history_dir()` helper, `ensure_app_dirs()` creates `_history/`.

**Admin routes** (`src/routes/content.rs`):
- `GET /{schema_slug}/{entry_id}/history` -- renders `templates/content/history.html` with version list (date, size, revert button).
- `POST /{schema_slug}/{entry_id}/revert/{timestamp}` -- CSRF-protected revert. Snapshots current state before reverting (so the revert itself is undoable). Uses `save_entry` to write the historical data, reloads cache, audit-logs as `content_update` with `"reverted to version {timestamp}"` detail.

**Snapshot trigger points**:
- Admin UI: `update_entry` handler (`routes/content.rs:751`).
- API: `update_entry` (`routes/api.rs:486`), `upsert_single` (`routes/api.rs:653`).
- Revert itself snapshots current before overwriting (`routes/content.rs:1410`).

**Export/import** (`src/sync/mod.rs`):
- `_history` directory is included in export bundles (line 50) and restored on import.

**Template** (`templates/content/history.html`):
- Table with Date, Size, Revert button per row. Empty state message when no versions exist.

### What is NOT built (gaps this spec addresses)

1. **Diff between versions** -- the task explicitly requires comparing versions, but the current UI only shows a list with no way to see what changed.
2. **Version preview** -- no way to inspect a version's contents before reverting.
3. **API endpoints for history** -- history is admin-only; no REST API access.
4. **Author attribution** -- snapshots are bare JSON; no metadata about who made the change or through what channel.

## Architecture

### 1. Version Diff (core logic + UI)

**Approach: Schema-aware field-level JSON diff, server-side**

Since Substrukt is schema-driven, the diff should be schema-aware: show each field by its human-readable title, with old and new values side-by-side. This is more useful than a raw JSON patch for CMS editors who think in terms of fields, not JSON paths.

**Rationale for field-level over raw JSON diff:**
- Users editing content through forms think in terms of "title", "body", "published" -- not `$.properties.title`.
- A raw unified diff of pretty-printed JSON is noisy for deeply nested objects and arrays.
- The schema provides field titles and types, enabling meaningful presentation (e.g., showing upload filenames instead of hash objects).
- Server-side rendering keeps the frontend simple -- no diff library in the browser.

**New types** in `src/history.rs`:

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct FieldDiff {
    pub path: String,       // dot-separated, e.g. "meta.description"
    pub label: String,      // human-readable label from schema "title" property
    pub kind: DiffKind,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DiffKind {
    Changed { old: serde_json::Value, new: serde_json::Value },
    Added { value: serde_json::Value },
    Removed { value: serde_json::Value },
}
```

**New function** in `src/history.rs`:

```rust
pub fn diff_entries(
    old: &Value,
    new: &Value,
    schema: &Value,
) -> Vec<FieldDiff>
```

Algorithm:
1. Walk `schema.properties` recursively.
2. For each field: if present in both and equal, skip. If present in both and different, emit `Changed`. If present only in new, emit `Added`. If present only in old, emit `Removed`.
3. For nested objects (`type: "object"`), recurse with dot-separated paths.
4. For arrays (`type: "array"`), compare element-by-element positionally (arrays in JSON Schema don't have stable IDs).
5. Internal fields (`_status`, `_id`) are excluded from diff output.
6. Upload fields display `filename (hash[:8])` instead of the raw object.
7. Depth limit matches `MAX_NESTING_DEPTH` (32) to prevent stack overflow.

**After walking schema properties**, also scan for keys present in the data but missing from the current schema (schema drift). Emit these as `Removed` or `Added` with path and a generic label derived from the key name.

**New route** in `routes/content.rs`:

```
GET /apps/{app}/content/{schema_slug}/{entry_id}/diff?from={timestamp}&to={timestamp|current}
```

- `from` (required): timestamp of the older version.
- `to` (optional): timestamp of the newer version. Defaults to current entry data.
- Renders `content/diff.html`.

**Template** (`templates/content/diff.html`):
- Header: "Comparing: {from_date} vs {to_label}".
- Table with columns: Field, Previous, Current.
- Changed rows highlighted with a subtle background color. Unchanged rows dimmed or hidden with a toggle.
- For long string values (>200 chars), truncate with "Show more" expansion.
- Back link to history page.

**Integration with history list:**
- Each version row in `history.html` gains a "Compare" link pointing to the diff view comparing that version vs current.

**Scope limits for diffing:**
- No character-level text diffing within string values. Field-level granularity is sufficient for a CMS.
- No visual diff for upload fields (would require rendering images side-by-side). Show metadata changes only.
- Only version-vs-current comparison. Pairwise historical diffs (comparing two arbitrary past versions) adds UI complexity with minimal practical value -- users can revert first, then compare.

### 2. Version Preview

**Route:**

```
GET /apps/{app}/content/{schema_slug}/{entry_id}/history/{timestamp}
```

Renders a read-only view of the historical entry data. Uses the schema's form field renderer with all inputs disabled, or a simpler key-value display for fields.

**Template** (`content/version_preview.html`):
- Read-only rendering of all fields.
- "Revert to this version" button (existing revert flow).
- "Compare with current" link (goes to diff view).
- Back link to history page.

### 3. API Endpoints for History

Add three endpoints to `api_app_routes()` in `src/routes/api.rs`. Axum routes by path specificity (longer/more-specific paths win), not registration order. The existing `/content/{schema_slug}/{entry_id}/publish` and `/unpublish` routes already demonstrate this pattern working correctly, so no ordering concern.

```rust
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
```

**`GET /content/{schema_slug}/{entry_id}/versions`**

List available versions for an entry.

Auth: Any role (viewer+). Read-only.

Response (200):
```json
[
  {
    "timestamp": 1713300000000,
    "date": "2026-04-17T00:00:00Z",
    "size": 1234,
    "user_id": null,
    "username": null,
    "source": null
  }
]
```

Returns empty array `[]` if no history exists. Sorted newest-first.

**Note on phasing:** In Phase 3 (API endpoints), the metadata fields (`user_id`, `username`, `source`) are always `null` because `VersionInfo` doesn't yet have this data. Phase 4 (authorship metadata) adds the sidecar files that populate these fields. The response shape is defined up front so that consumers don't face a schema change between phases. `Option<String>` serializes to JSON `null` in serde.

**`GET /content/{schema_slug}/{entry_id}/versions/{timestamp}`**

Retrieve a specific historical version's data.

Auth: Any role (viewer+). Read-only.

Response (200): The raw JSON entry data as stored at that point in time.

Response (404): Version not found.

**`POST /content/{schema_slug}/{entry_id}/versions/{timestamp}/revert`**

Revert an entry to a historical version.

Auth: Editor+ role required.

Behavior: Identical to the existing admin UI revert -- snapshots the current state first (with metadata recording the revert), then overwrites with the historical version via `save_entry`. Reloads cache. Audit-logged.

Response (200):
```json
{
  "status": "reverted",
  "entry_id": "my-entry",
  "reverted_to": 1713300000000
}
```

Response (404): Version not found.

### 4. Authorship Metadata

**Approach: Sidecar files alongside data snapshots**

Each snapshot gets an optional companion file `<timestamp>.meta.json`:

```
data/<app>/_history/<schema>/<entry>/
  1713300000000.json       # entry data snapshot (unchanged format)
  1713300000000.meta.json  # metadata (who, how)
```

Sidecar contents:
```json
{
  "user_id": "550e8400-e29b-41d4-a716-446655440000",
  "username": "alice",
  "source": "admin_ui"
}
```

**Rationale for sidecar over embedding in the data file:**
- Data snapshots are exact copies of the entry JSON. Mixing metadata into the data file would mean reverted entries contain version metadata fields that don't belong in the content.
- Sidecars are independently optional -- missing `.meta.json` is fine (backwards compatible with existing snapshots that have no metadata).
- The data file remains valid entry JSON that can be loaded directly by `save_entry` during revert without stripping metadata fields.

**New types** in `src/history.rs`:

```rust
pub struct SnapshotMeta {
    pub user_id: String,
    pub username: String,
    pub source: SnapshotSource,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotSource {
    AdminUi,
    Api,
    Import,
    Revert,
}
```

**Updated `snapshot_entry` signature:**

```rust
pub fn snapshot_entry(
    data_dir: &Path,
    schema_slug: &str,
    entry_id: &str,
    current_data: &Value,
    max_versions: usize,
    meta: Option<&SnapshotMeta>,
) -> eyre::Result<()>
```

When `meta` is `Some`, writes `<ts>.meta.json` alongside `<ts>.json`. When `None`, only the data file is written (backwards compatible).

**Updated `VersionInfo`:**

```rust
pub struct VersionInfo {
    pub timestamp: u64,
    pub size: u64,
    pub user_id: Option<String>,
    pub username: Option<String>,
    pub source: Option<String>,
}
```

`list_versions` reads the `.meta.json` sidecar if present, falling back to `None` for legacy snapshots.

**Updated `prune_versions`:** Must also delete `<ts>.meta.json` alongside `<ts>.json` when pruning.

**Updated `delete_history`:** Already does `remove_dir_all`, so sidecars are deleted automatically.

**Callers:**
- `routes/content.rs` update handler: passes `SnapshotMeta { user_id: user.id, username, source: AdminUi }`.
- `routes/content.rs` revert handler: passes `SnapshotMeta { ..., source: Revert }`.
- `routes/api.rs` update/upsert handlers: passes `SnapshotMeta { user_id: "api", username: "api", source: Api }`.
- Export/import: sidecars transfer automatically since `_history` tree is archived whole.

**UI updates:**
- History table (`history.html`): add "Author" and "Source" columns. Show "Unknown" for legacy snapshots.
- Diff view: show author info in the header.

## Data Models

### Existing (unchanged)

```rust
// src/history.rs -- current
pub struct VersionInfo {
    pub timestamp: u64,
    pub size: u64,
}
```

### New/Updated

```rust
// Updated VersionInfo
pub struct VersionInfo {
    pub timestamp: u64,
    pub size: u64,
    pub user_id: Option<String>,
    pub username: Option<String>,
    pub source: Option<String>,
}

// New diff types
#[derive(Debug, Clone, serde::Serialize)]
pub struct FieldDiff {
    pub path: String,
    pub label: String,
    pub kind: DiffKind,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DiffKind {
    Changed { old: serde_json::Value, new: serde_json::Value },
    Added { value: serde_json::Value },
    Removed { value: serde_json::Value },
}

// New metadata types
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SnapshotMeta {
    pub user_id: String,
    pub username: String,
    pub source: SnapshotSource,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotSource {
    AdminUi,
    Api,
    Import,
    Revert,
}
```

### On-Disk Format

```
data/<app-slug>/_history/<schema-slug>/<entry-id>/
  <timestamp-ms>.json       # full entry data snapshot (unchanged)
  <timestamp-ms>.meta.json  # optional authorship sidecar (new)
```

Each `.json` file is a plain JSON snapshot of the entry data including `_status` and internal fields. No wrapper, no metadata envelope.

Each `.meta.json` file contains `{ "user_id", "username", "source" }`. Optional -- legacy snapshots have no sidecar.

## Error Handling

| Scenario | Behavior |
|---|---|
| Version timestamp not found | Admin: flash error + redirect to history. API: 404. |
| Entry does not exist | Admin: "Entry not found" error. API: 404. |
| History dir missing/unreadable | `list_versions` returns empty vec (no error). |
| Diff with invalid `from`/`to` param | Admin: 400 with descriptive message. |
| Revert with `version_history_count = 0` | Revert still works but no pre-revert snapshot is taken (existing behavior via `snapshot_entry` early return). |
| Snapshot write fails (disk full, permissions) | Logged as warning; the content update itself proceeds (existing behavior -- snapshot failure is non-fatal). |
| Meta sidecar read fails | Metadata fields returned as `None`. Non-fatal. |
| Meta sidecar write fails | Logged as warning. Data snapshot still succeeds. |

## Edge Cases

### Schema drift on revert

`revert_entry` calls `save_entry` directly without `validate_content` (`routes/content.rs:1422`). Validation and persistence are separate in Substrukt -- `save_entry` is a pure write, while `validate_content` is called explicitly by the admin `update_entry` handler (`routes/content.rs:713`) and API update handlers before calling `save_entry`. Revert intentionally skips validation: if the schema has changed since the snapshot was taken, the reverted data may not conform to the current schema. Blocking reverts when the schema has evolved would prevent legitimate recovery. The reverted data can be edited afterward to conform.

The diff view should handle schema drift gracefully by scanning both old/new data keys, not just the current schema properties. Fields present in the old data but missing from the current schema are shown as "removed field". Fields in the current schema but absent from old data are shown as "new field (not in this version)".

The UI should show a warning banner on the revert confirmation when the version's fields don't align with the current schema: "This version was saved under a different schema version. Some fields may be missing or invalid after revert."

### File watcher and `_history` directory

The file watcher in `cache.rs` watches `data_dir` recursively. Every `snapshot_entry` write to `_history/` sends an event through the debounce channel. However, the current event callback (`cache::spawn_watcher`, `cache.rs:160`) does not inspect `event.paths` at all -- it only checks `event.kind` and sends `()` through the channel.

**Severity: Low.** In practice, a content update writes both the content file (legitimate cache trigger) and the `_history/` snapshot. The 200ms debounce window coalesces both events into a single cache rebuild, so the `_history/` write rarely causes additional work. It only matters when snapshots are created without a content write (which doesn't happen today -- snapshots always precede a `save_entry` call).

**Recommended fix (micro-optimization):** Add a path filter in the event callback to skip `_history/` paths. This prevents the (rare) case where a pruning operation or manual history file manipulation triggers an unnecessary rebuild:

```rust
if event.paths.iter().any(|p| p.to_string_lossy().contains("/_history/")) {
    return; // skip history-only changes
}
```

### Status field on revert

Reverting restores the `_status` field from the snapshot. If an entry was published and then reverted to a draft-era snapshot, it becomes a draft again. This is correct behavior -- revert restores the full state. The diff view should prominently show status changes so the user is aware before reverting.

### Concurrent edits (last-write-wins)

There is no file locking. If two editors save the same entry in quick succession:
1. Editor A reads entry, editor B reads entry (same version).
2. Editor A saves -- snapshots current, writes new data.
3. Editor B saves -- snapshots A's data (not the version B started from), writes new data.

B's save overwrites A's changes. A's version is preserved in history, so it can be recovered via revert. The existing "unsaved changes" warning partially mitigates this. This is adequate for a CMS of this scale.

### Single-file storage mode

History works per-entry. For `StorageMode::SingleFile` with `Kind::Collection`, each entry within the single file gets its own history directory (keyed by `_id`). The snapshot captures just that entry's data, not the entire file. Correct in the current implementation.

### version_history_count = 0

Disables snapshotting entirely. History list returns empty. Revert route returns "Version not found" (nothing to revert to). Diff route returns 404. Valid configuration for disk-constrained deployments.

### Bulk operations

Bulk publish/unpublish use `set_entry_status`, which does NOT create snapshots (metadata-only change, per docstring at `content/mod.rs:232-233`). Bulk delete calls `delete_history` per entry. Correct behavior.

### Export/import

The `_history` directory is already included in export bundles and restored on import. Authorship `.meta.json` sidecars will transfer automatically since the entire `_history` tree is archived. No changes needed.

### Bug: delete_single cache key (pre-existing)

In `src/routes/api.rs:725`, the cache key for deleting a single-kind entry is `format!("{}/_single/{schema_slug}", app.app.slug)` (key: `{app}/_single/{schema}`), but `cache::reload_entry` (`cache.rs:119`) constructs keys as `format!("{}/{}/{}", app_slug, schema.meta.slug, entry_id)` (key: `{app}/{schema}/_single`). The `_single` and `schema_slug` segments are swapped in the delete path. This is a pre-existing bug unrelated to versioning -- the `cache.remove` call doesn't match the key that was inserted, so the stale cache entry persists until the next full rebuild or file-watcher trigger. The fix is trivial: change line 725 to `format!("{}/{schema_slug}/_single", app.app.slug)`.

## Non-Goals

- **Branching histories** -- this is a linear version history, not a VCS.
- **Named versions / tags** -- no ability to label a version as "v1.0" or "pre-launch". The timestamp is the only identifier.
- **Diff-based compression** -- snapshots are full copies. Delta encoding saves disk but adds complexity for marginal benefit given small CMS entry sizes and pruning limits.
- **Per-schema or per-app retention overrides** -- `version_history_count` is global. Per-schema overrides via `x-substrukt` metadata could be added later.
- **Real-time collaborative editing / conflict resolution** -- out of scope.
- **Comparison between two arbitrary historical versions** -- only version-vs-current.
- **Undelete via history** -- when an entry is deleted, its history is also deleted. Restoring deleted entries is handled by backups, not versioning.
- **Webhook/deployment triggers on revert** -- revert uses `save_entry` + `reload_entry` which updates the cache but does not fire deployment webhooks. Could be a follow-up.

## Implementation Sequence

### Phase 1: Version diffing (core + UI)

1. Add `FieldDiff`, `DiffKind` types and `diff_entries()` function to `src/history.rs` -- pure logic, easy to unit test.
2. Add `GET /{schema_slug}/{entry_id}/diff?from=&to=` route in `routes/content.rs`.
3. Create `templates/content/diff.html` template showing field-by-field changes.
4. Add "Compare" link to each version row in `templates/content/history.html`.

### Phase 2: Version preview

1. Add `GET /{schema_slug}/{entry_id}/history/{timestamp}` route for read-only preview.
2. Create `templates/content/version_preview.html` template.
3. Add "View" link to each version row in `templates/content/history.html`.

### Phase 3: API endpoints

1. Add `api_list_versions`, `api_get_version`, `api_revert_version` handlers in `routes/api.rs`.
2. Register routes in `api_app_routes()` before the entry catch-all.
3. Add version endpoints to OpenAPI spec generation in `src/openapi.rs`.

### Phase 4: Authorship metadata

1. Add `SnapshotMeta`, `SnapshotSource` types to `src/history.rs`.
2. Update `snapshot_entry` signature to accept optional metadata; write `.meta.json` sidecar.
3. Update `list_versions` to read sidecar metadata.
4. Update `prune_versions` to clean up `.meta.json` files alongside data files.
5. Update all callers in `routes/content.rs` and `routes/api.rs` to pass metadata.
6. Show author/source in history table and diff views.

### Phase 5: Polish

1. Filter `_history/` paths from file watcher to avoid unnecessary cache rebuilds (low-priority micro-optimization -- the 200ms debounce already coalesces most cases).
2. Add schema-drift warning to the diff/revert UI.
3. Fix `delete_single` cache key bug (`api.rs:725`): swap `_single` and `{schema_slug}` to match the key format used by `cache::reload_entry`.

## Testing Strategy

- **Unit tests** for `diff_entries()`: identical entries (empty diff), changed fields, added/removed fields, nested objects, arrays, depth limit, internal field exclusion (`_status`, `_id`), upload field display, schema-drift (field in data but not schema).
- **Unit tests** for `SnapshotMeta` sidecar: write + read round-trip, missing sidecar graceful fallback (returns `None` metadata), prune deletes both `.json` and `.meta.json`.
- **Integration tests** for API endpoints: list versions (empty, populated), get specific version (200, 404), revert (verify snapshot-before-revert, cache reload, audit log), role-based access control (viewer can list/get, editor can revert, viewer cannot revert).
- **Integration test** for schema drift: change a schema, view diff of pre-change snapshot, verify fields shown correctly. Attempt revert, verify it succeeds.
- **Integration test** for file watcher filter: write to `_history/`, verify no cache rebuild triggered.
