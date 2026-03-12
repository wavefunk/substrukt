# Schema Editor Upgrade + Uploads Browser Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Upgrade the schema editor with vanilla-jsoneditor and add an uploads browser backed by SQLite metadata tracking.

**Architecture:** Two independent features sharing one branch. Feature 1 (schema editor) is a frontend-only change — swap the textarea for vanilla-jsoneditor loaded from CDN. Feature 2 (uploads browser) moves upload metadata from `.meta.json` sidecars to SQLite, adds reference tracking, and builds a browse/filter page. The features are independent and can be implemented in either order.

**Tech Stack:** Rust/Axum, sqlx (SQLite), minijinja templates, htmx, twind, vanilla-jsoneditor (CDN)

**Spec:** `docs/superpowers/specs/2026-03-12-schema-editor-uploads-browser-design.md`

---

## File Structure

### Feature 1: Schema Editor

| Action | File | Responsibility |
|--------|------|----------------|
| Modify | `templates/schemas/edit.html` | Replace textarea with vanilla-jsoneditor, add CDN script, hidden input, submit handler |

### Feature 2: Uploads Browser

| Action | File | Responsibility |
|--------|------|----------------|
| Create | `migrations/003_create_uploads.sql` | SQL migration for `uploads` and `upload_references` tables |
| Modify | `src/db/mod.rs` | Enable `PRAGMA foreign_keys = ON` on pool |
| Modify | `src/uploads/mod.rs` | Remove `.meta.json` sidecar logic, add SQLite operations, add `extract_upload_hashes()`, add startup migration |
| Modify | `src/routes/uploads.rs` | Add `GET /uploads` handler with list/filter, make `serve_file`/`serve_upload_by_hash` async for DB metadata lookup |
| Modify | `src/routes/content.rs` | Make `process_uploads` async, add reference tracking on content create/update/delete |
| Modify | `src/routes/api.rs` | Add reference tracking on API content create/update/delete, update `serve_upload_by_hash` call to `.await` |
| Modify | `src/sync/mod.rs` | Export with manifest instead of sidecars, import handles both formats. Keep sync versions for CLI, add async versions for API. |
| Modify | `src/main.rs` | Call startup migration, init pool for CLI import/export commands |
| Create | `templates/uploads/list.html` | Uploads browser table with filters |
| Modify | `templates/_nav.html` | Add "Uploads" link to navigation |
| Modify | `tests/integration.rs` | Tests for migration, reference tracking, uploads browser, export/import |

---

## Chunk 1: Schema Editor Upgrade

### Task 1: Replace textarea with vanilla-jsoneditor

**Files:**
- Modify: `templates/schemas/edit.html`

- [ ] **Step 1: Update the schema edit template**

Replace the full template with a vanilla-jsoneditor setup. The form structure, CSRF handling, and submit/delete buttons stay unchanged.

```html
{% extends base_template %}
{% block title %}{% if is_new %}New Schema{% else %}Edit Schema{% endif %} — Substrukt{% endblock %}
{% block content %}
<div class="flex items-center justify-between mb-6">
  <h1 class="text-2xl font-bold">{% if is_new %}New Schema{% else %}Edit Schema{% endif %}</h1>
  <a href="/schemas" class="text-gray-500 hover:text-gray-700 text-sm">Back to Schemas</a>
</div>

{% if error %}
<div class="bg-red-50 text-red-700 p-3 rounded mb-4 text-sm">{{ error }}</div>
{% endif %}

<div class="bg-white rounded-lg shadow p-6">
  <form id="schema-form" method="post" action="{% if is_new %}/schemas/new{% else %}/schemas/{{ slug }}{% endif %}">
    <input type="hidden" name="_csrf" value="{{ csrf_token }}">
    <input type="hidden" id="schema_json" name="schema_json" value="">
    <div class="mb-4">
      <label class="block text-sm font-medium text-gray-700 mb-1">JSON Schema</label>
      <div id="jsoneditor" style="height: 500px; border: 1px solid #d1d5db; border-radius: 0.375rem;"></div>
    </div>
    <div class="flex gap-3">
      <button type="submit" class="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 text-sm font-medium">
        {% if is_new %}Create Schema{% else %}Save Changes{% endif %}
      </button>
      {% if not is_new %}
      <button type="button" onclick="deleteSchema()" class="bg-red-50 text-red-600 px-4 py-2 rounded-md hover:bg-red-100 text-sm font-medium">
        Delete
      </button>
      {% endif %}
    </div>
  </form>
</div>

<script type="module">
import { createJSONEditor } from 'https://cdn.jsdelivr.net/npm/vanilla-jsoneditor@2/standalone.js';

const initialContent = {{ schema_json | tojson }};
let editor;

try {
  const parsed = JSON.parse(initialContent);
  editor = createJSONEditor({
    target: document.getElementById('jsoneditor'),
    props: {
      content: { json: parsed },
      mode: 'text',
    }
  });
} catch (e) {
  editor = createJSONEditor({
    target: document.getElementById('jsoneditor'),
    props: {
      content: { text: initialContent },
      mode: 'text',
    }
  });
}

document.getElementById('schema-form').addEventListener('submit', function(e) {
  const content = editor.get();
  let jsonStr;
  if (content.json !== undefined) {
    jsonStr = JSON.stringify(content.json, null, 2);
  } else {
    jsonStr = content.text;
  }
  document.getElementById('schema_json').value = jsonStr;
});
</script>

{% if not is_new %}
<script>
function deleteSchema() {
  if (confirm('Delete this schema? This cannot be undone.')) {
    fetch('/schemas/{{ slug }}', {
      method: 'DELETE',
      headers: { 'X-CSRF-Token': '{{ csrf_token }}' }
    }).then(() => window.location.href = '/schemas');
  }
}
</script>
{% endif %}
{% endblock %}
```

Key decisions:
- `{{ schema_json | tojson }}` outputs the JSON string safely escaped for JS (handles quotes, newlines)
- Try to parse as JSON first; if invalid, fall back to text mode (so existing malformed schemas can still be edited)
- On submit, extract content from editor — if in tree mode it's `.json`, if in text mode it's `.text`
- Editor height is 500px fixed (much better than 24-row textarea)
- Form still uses standard POST with hidden `schema_json` input — CSRF handling is unchanged

- [ ] **Step 2: Manually test the schema editor**

Run: `cargo run -- serve`

Test:
1. Go to `/schemas/new` — editor should load with default template, syntax highlighted
2. Type JSON — verify auto-indent on Enter, bracket matching
3. Press Tab — should insert indentation, not change focus
4. Click format button in editor toolbar
5. Toggle between text and tree modes
6. Submit the form — verify schema is created correctly
7. Go to `/schemas/{slug}/edit` — verify existing schema loads in editor
8. Edit and save — verify changes persist

- [ ] **Step 3: Commit**

```bash
git add templates/schemas/edit.html
git commit -m "feat: replace schema editor textarea with vanilla-jsoneditor

Loads vanilla-jsoneditor@2 from CDN in text mode with tree mode toggle.
Provides syntax highlighting, auto-indent, bracket matching, and formatting."
```

---

## Chunk 2: SQLite Upload Tracking (Database + Core Logic)

### Task 2: Add SQL migration for upload tables

**Files:**
- Create: `migrations/003_create_uploads.sql`

- [ ] **Step 1: Create the migration file**

```sql
CREATE TABLE IF NOT EXISTS uploads (
    hash TEXT PRIMARY KEY,
    filename TEXT NOT NULL,
    mime TEXT NOT NULL,
    size INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS upload_references (
    upload_hash TEXT NOT NULL REFERENCES uploads(hash) ON DELETE CASCADE,
    schema_slug TEXT NOT NULL,
    entry_id TEXT NOT NULL,
    PRIMARY KEY (upload_hash, schema_slug, entry_id)
);
```

Note: `ON DELETE CASCADE` on the foreign key so that if an upload is ever deleted, its references are cleaned up automatically.

- [ ] **Step 2: Verify migration runs**

Run: `cargo build`

This triggers sqlx compile-time checks. If it compiles, the migration SQL is syntactically valid.

- [ ] **Step 3: Commit**

```bash
git add migrations/003_create_uploads.sql
git commit -m "feat: add SQL migration for uploads and upload_references tables"
```

### Task 3: Enable foreign keys on SQLite pool

**Files:**
- Modify: `src/db/mod.rs:5` (the `SqliteConnectOptions` chain)

- [ ] **Step 1: Add pragma to pool options**

In `src/db/mod.rs`, add `.pragma("foreign_keys", "ON")` to the options chain:

```rust
let options = SqliteConnectOptions::from_str(&url)?
    .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
    .pragma("foreign_keys", "ON")
    .create_if_missing(true);
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`

- [ ] **Step 3: Commit**

```bash
git add src/db/mod.rs
git commit -m "feat: enable PRAGMA foreign_keys on SQLite connection pool"
```

### Task 4: Add SQLite operations and hash extraction to uploads module

**Files:**
- Modify: `src/uploads/mod.rs`

- [ ] **Step 1: Write the failing test for extract_upload_hashes**

Add a test at the bottom of `src/uploads/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_upload_hashes_flat() {
        let data = json!({
            "title": "Hello",
            "image": {
                "hash": "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
                "filename": "photo.jpg",
                "mime": "image/jpeg"
            }
        });
        let hashes = extract_upload_hashes(&data);
        assert_eq!(hashes.len(), 1);
        assert!(hashes.contains("abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"));
    }

    #[test]
    fn test_extract_upload_hashes_nested() {
        let data = json!({
            "author": {
                "name": "Alice",
                "avatar": {
                    "hash": "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
                    "filename": "avatar.png",
                    "mime": "image/png"
                }
            }
        });
        let hashes = extract_upload_hashes(&data);
        assert_eq!(hashes.len(), 1);
        assert!(hashes.contains("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"));
    }

    #[test]
    fn test_extract_upload_hashes_in_array() {
        let data = json!({
            "gallery": [
                {
                    "hash": "aaaa567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
                    "filename": "img1.jpg",
                    "mime": "image/jpeg"
                },
                {
                    "hash": "bbbb567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
                    "filename": "img2.jpg",
                    "mime": "image/jpeg"
                }
            ]
        });
        let hashes = extract_upload_hashes(&data);
        assert_eq!(hashes.len(), 2);
    }

    #[test]
    fn test_extract_upload_hashes_ignores_non_uploads() {
        let data = json!({
            "title": "Hello",
            "count": 42,
            "nested": { "hash": "not-a-valid-hex-hash" }
        });
        let hashes = extract_upload_hashes(&data);
        assert_eq!(hashes.len(), 0);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test test_extract_upload_hashes`

Expected: compilation error — `extract_upload_hashes` doesn't exist yet.

- [ ] **Step 3: Implement extract_upload_hashes**

Add to `src/uploads/mod.rs`. Note: use a stricter hash validation for extraction (64 hex chars = SHA-256) rather than the existing `is_valid_hash` which only requires 3+ hex chars:

```rust
use std::collections::HashSet;
use serde_json::Value;

/// Check if a string is a valid SHA-256 hash (exactly 64 hex characters).
fn is_sha256_hash(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Recursively walk JSON and extract hashes from upload objects.
/// Upload objects have shape: {"hash": "<64-char hex>", "filename": "...", "mime": "..."}
pub fn extract_upload_hashes(value: &Value) -> HashSet<String> {
    let mut hashes = HashSet::new();
    collect_upload_hashes(value, &mut hashes);
    hashes
}

fn collect_upload_hashes(value: &Value, hashes: &mut HashSet<String>) {
    match value {
        Value::Object(map) => {
            // Check if this object looks like an upload reference
            if let (Some(Value::String(hash)), Some(Value::String(_)), Some(Value::String(_))) =
                (map.get("hash"), map.get("filename"), map.get("mime"))
            {
                if is_sha256_hash(hash) {
                    hashes.insert(hash.clone());
                    return;
                }
            }
            // Otherwise recurse into values
            for v in map.values() {
                collect_upload_hashes(v, hashes);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                collect_upload_hashes(v, hashes);
            }
        }
        _ => {}
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test test_extract_upload_hashes`

Expected: all 4 tests pass.

- [ ] **Step 5: Add SQLite insert/query functions**

Add to `src/uploads/mod.rs`:

```rust
use sqlx::SqlitePool;

/// Insert upload metadata into SQLite. Uses INSERT OR IGNORE for dedup.
pub async fn db_insert_upload(pool: &SqlitePool, meta: &UploadMeta) -> eyre::Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO uploads (hash, filename, mime, size) VALUES (?, ?, ?, ?)"
    )
    .bind(&meta.hash)
    .bind(&meta.filename)
    .bind(&meta.mime)
    .bind(meta.size as i64)
    .execute(pool)
    .await?;
    Ok(())
}

/// Get upload metadata from SQLite by hash.
pub async fn db_get_upload_meta(pool: &SqlitePool, hash: &str) -> eyre::Result<Option<UploadMeta>> {
    let row = sqlx::query_as::<_, (String, String, String, i64)>(
        "SELECT hash, filename, mime, size FROM uploads WHERE hash = ?"
    )
    .bind(hash)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(hash, filename, mime, size)| UploadMeta {
        hash,
        filename,
        mime,
        size: size as u64,
    }))
}

/// Replace all upload references for a content entry.
/// Uses a transaction to ensure atomicity.
pub async fn db_update_references(
    pool: &SqlitePool,
    schema_slug: &str,
    entry_id: &str,
    hashes: &HashSet<String>,
) -> eyre::Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM upload_references WHERE schema_slug = ? AND entry_id = ?")
        .bind(schema_slug)
        .bind(entry_id)
        .execute(&mut *tx)
        .await?;

    for hash in hashes {
        sqlx::query(
            "INSERT OR IGNORE INTO upload_references (upload_hash, schema_slug, entry_id) VALUES (?, ?, ?)"
        )
        .bind(hash)
        .bind(schema_slug)
        .bind(entry_id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Delete all upload references for a content entry.
pub async fn db_delete_references(
    pool: &SqlitePool,
    schema_slug: &str,
    entry_id: &str,
) -> eyre::Result<()> {
    sqlx::query("DELETE FROM upload_references WHERE schema_slug = ? AND entry_id = ?")
        .bind(schema_slug)
        .bind(entry_id)
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 6: Modify store_upload to skip .meta.json and insert into SQLite**

Update `store_upload` in `src/uploads/mod.rs`. The function currently writes `.meta.json` at lines 73-77. Remove that block. The function signature needs to take `&SqlitePool` as an additional parameter and become async.

Change signature from:
```rust
pub fn store_upload(uploads_dir: &Path, filename: &str, mime: &str, data: &[u8]) -> eyre::Result<UploadMeta>
```
to:
```rust
pub async fn store_upload(uploads_dir: &Path, pool: &SqlitePool, filename: &str, mime: &str, data: &[u8]) -> eyre::Result<UploadMeta>
```

After writing the file to disk (existing logic), replace the `.meta.json` write with:
```rust
db_insert_upload(pool, &meta).await?;
```

Remove `get_upload_meta()` function (the one reading `.meta.json` from disk) — it is replaced by `db_get_upload_meta()`.

- [ ] **Step 7: Update all callers — async cascade**

This is the most involved step. Changing `store_upload` to async causes a cascade:

**`src/routes/content.rs`:**
- `process_uploads()` (~line 397) calls `store_upload` in a loop. It must become `async fn` and take `&SqlitePool` as a parameter.
- Its call sites in `create_entry()` (~line 215) and `update_entry()` (~line 284) must add `.await`.

**`src/routes/uploads.rs`:**
- `upload_file()` (~line 42): change `uploads::store_upload(...)` call to add `&state.pool` and `.await`.
- `serve_file()` (line 87): currently sync `fn`, calls `uploads::get_upload_meta()`. Must become `async fn` since `db_get_upload_meta` is async. Change the `get_upload_meta()` call to `db_get_upload_meta(&state.pool, hash).await.ok().flatten()`.
- `serve_upload()` (line 69) and `serve_upload_no_name()` (line 76): already `async fn`, just add `.await` to `serve_file()` calls.
- `serve_upload_by_hash()` (line 83): must become `pub async fn` since it calls `serve_file`. Update signature:
  ```rust
  pub async fn serve_upload_by_hash(state: &AppState, hash: &str) -> axum::response::Response {
      serve_file(state, hash).await
  }
  ```

**`src/routes/api.rs`:**
- `upload_file()` (~line 310): add `&state.pool` to `store_upload` call.
- `get_upload()` (~line 350): calls `serve_upload_by_hash` — add `.await` since it's now async.

- [ ] **Step 8: Verify it compiles**

Run: `cargo build`

Fix any remaining compile errors from the signature changes. Common issues:
- Missing `.await` on calls
- Lifetime issues when passing `&state.pool` through async boundaries (shouldn't be an issue with Axum extractors)

- [ ] **Step 9: Run all tests**

Run: `cargo test`

Expected: all 17 existing tests pass. The tests create fresh temp databases, so migrations will create the new tables automatically.

- [ ] **Step 10: Commit**

```bash
git add src/uploads/mod.rs src/routes/uploads.rs src/routes/content.rs src/routes/api.rs
git commit -m "feat: move upload metadata from .meta.json sidecars to SQLite

- store_upload now inserts into uploads table instead of writing .meta.json
- serve_file reads metadata from SQLite instead of .meta.json sidecar
- process_uploads and serve_file/serve_upload_by_hash now async
- Add extract_upload_hashes() for recursive JSON upload reference extraction
- Add db_insert_upload, db_get_upload_meta, db_update_references, db_delete_references
- Uses transactions for reference updates, stricter SHA-256 hash validation"
```

### Task 5: Add reference tracking to content create/update/delete

**Files:**
- Modify: `src/routes/content.rs` (~lines 236, 312, 340)
- Modify: `src/routes/api.rs` (~lines 200, 250, 280)

- [ ] **Step 1: Add reference tracking to web content handlers**

In `src/routes/content.rs`:

After `content::save_entry()` in `create_entry()` (~line 236), add:
```rust
let hashes = uploads::extract_upload_hashes(&data);
uploads::db_update_references(&state.pool, &schema_slug, &entry_id, &hashes).await?;
```

After `content::save_entry()` in `update_entry()` (~line 312), add the same two lines (use the `entry_id` from the path parameter).

In `delete_entry()` (~line 340), before `content::delete_entry()`, add:
```rust
uploads::db_delete_references(&state.pool, &schema_slug, &entry_id).await?;
```

Add `use crate::uploads;` at the top if not already imported.

- [ ] **Step 2: Add reference tracking to API content handlers**

In `src/routes/api.rs`:

Same pattern — after entry save in `create_entry()` (~line 200) and `update_entry()` (~line 250), extract hashes and call `db_update_references`. In `delete_entry()` (~line 280), call `db_delete_references` before deletion.

- [ ] **Step 3: Run all tests**

Run: `cargo test`

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/routes/content.rs src/routes/api.rs
git commit -m "feat: track upload references in SQLite on content create/update/delete"
```

### Task 6: Add startup migration from .meta.json sidecars

**Files:**
- Modify: `src/uploads/mod.rs`
- Modify: `src/main.rs` (~line 130, after cache population)

- [ ] **Step 1: Write the migration function**

Add to `src/uploads/mod.rs`. Idempotency is based on whether `.meta.json` files exist (not whether the table is empty, since new uploads may have been inserted via the new code path before old sidecars are migrated):

```rust
/// One-time migration: populate SQLite from existing .meta.json sidecars.
/// Idempotent: only runs if .meta.json files exist on disk.
/// Deletes .meta.json files after successful migration.
pub async fn migrate_meta_sidecars(
    uploads_dir: &Path,
    data_dir: &Path,
    pool: &SqlitePool,
) -> eyre::Result<()> {
    // Find all .meta.json files
    let mut meta_files = Vec::new();
    if uploads_dir.exists() {
        for prefix_entry in std::fs::read_dir(uploads_dir)? {
            let prefix_entry = prefix_entry?;
            if prefix_entry.file_type()?.is_dir() {
                for file_entry in std::fs::read_dir(prefix_entry.path())? {
                    let file_entry = file_entry?;
                    let path = file_entry.path();
                    if path.to_string_lossy().ends_with(".meta.json") {
                        meta_files.push(path);
                    }
                }
            }
        }
    }

    if meta_files.is_empty() {
        return Ok(());
    }

    tracing::info!("Found {} .meta.json sidecars to migrate", meta_files.len());

    // Insert upload metadata (INSERT OR IGNORE handles re-runs safely)
    for meta_path in &meta_files {
        let content = std::fs::read_to_string(meta_path)?;
        let meta: UploadMeta = serde_json::from_str(&content)?;
        db_insert_upload(pool, &meta).await?;
    }

    // Scan content files and populate references
    populate_references_from_content(data_dir, pool).await?;

    // Delete .meta.json sidecars
    for meta_path in &meta_files {
        std::fs::remove_file(meta_path)?;
    }

    tracing::info!("Migrated {} upload metadata files to SQLite", meta_files.len());
    Ok(())
}

/// Scan all content JSON files and populate upload_references table.
/// Used by both startup migration and import.
pub async fn populate_references_from_content(
    data_dir: &Path,
    pool: &SqlitePool,
) -> eyre::Result<()> {
    let schemas_dir = data_dir.join("schemas");
    let content_dir = data_dir.join("content");
    if !schemas_dir.exists() || !content_dir.exists() {
        return Ok(());
    }

    for schema_entry in std::fs::read_dir(&schemas_dir)? {
        let schema_entry = schema_entry?;
        let schema_path = schema_entry.path();
        if schema_path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let schema_slug = schema_path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();

        let schema_str = std::fs::read_to_string(&schema_path)?;
        let schema_val: Value = serde_json::from_str(&schema_str)?;
        let storage = schema_val.pointer("/x-substrukt/storage")
            .and_then(|v| v.as_str())
            .unwrap_or("directory");

        if storage == "directory" {
            let entry_dir = content_dir.join(&schema_slug);
            if entry_dir.exists() {
                for entry_file in std::fs::read_dir(&entry_dir)? {
                    let entry_file = entry_file?;
                    let entry_path = entry_file.path();
                    if entry_path.extension().and_then(|e| e.to_str()) != Some("json") {
                        continue;
                    }
                    let entry_id = entry_path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default()
                        .to_string();
                    let data: Value = serde_json::from_str(&std::fs::read_to_string(&entry_path)?)?;
                    let hashes = extract_upload_hashes(&data);
                    db_update_references(pool, &schema_slug, &entry_id, &hashes).await?;
                }
            }
        } else {
            // SingleFile mode
            let single_path = content_dir.join(format!("{schema_slug}.json"));
            if single_path.exists() {
                let arr: Value = serde_json::from_str(&std::fs::read_to_string(&single_path)?)?;
                if let Value::Array(entries) = &arr {
                    for entry in entries {
                        let entry_id = entry.get("_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let hashes = extract_upload_hashes(entry);
                        db_update_references(pool, &schema_slug, &entry_id, &hashes).await?;
                    }
                }
            }
        }
    }

    Ok(())
}
```

- [ ] **Step 2: Call migration from main.rs**

In `src/main.rs`, in `run_server()` after the cache is populated (~line 130), add:

```rust
uploads::migrate_meta_sidecars(
    &config.uploads_dir(),
    &config.data_dir,
    &pool,
).await?;
```

Add `use crate::uploads;` at the top of `main.rs` if not already imported.

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`

- [ ] **Step 4: Run all tests**

Run: `cargo test`

Expected: all tests pass. In tests, there are no existing `.meta.json` files (fresh temp dirs), so migration is a no-op.

- [ ] **Step 5: Commit**

```bash
git add src/uploads/mod.rs src/main.rs
git commit -m "feat: add startup migration from .meta.json sidecars to SQLite

Scans for existing .meta.json files, populates uploads and
upload_references tables, then deletes sidecars.
Idempotent: only runs if .meta.json files exist on disk.
Extracted populate_references_from_content() for reuse in import."
```

---

## Chunk 3: Uploads Browser Page

### Task 7: Add uploads list handler and template

**Files:**
- Modify: `src/routes/uploads.rs`
- Create: `templates/uploads/list.html`
- Modify: `templates/_nav.html`

- [ ] **Step 1: Add query struct and list handler to uploads routes**

In `src/routes/uploads.rs`, add imports and the handler. Key points from the codebase patterns:
- Must accept `HxRequest(is_htmx): HxRequest` and `session: Session` like every other page handler
- Must pass `base_template => base_for_htmx(is_htmx)` and `csrf_token` to the template context
- Use a single JOIN query instead of N+1 queries
- Do filename filtering in SQL, not in application code
- Pass simplified `{slug, title}` for schemas (not `SchemaFile` which doesn't implement `Serialize`)

```rust
use axum::extract::Query;
use axum::response::Html;
use axum_htmx::HxRequest;
use tower_sessions::Session;
use crate::auth;
use crate::schema;
use crate::templates::base_for_htmx;

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

pub async fn list_uploads(
    HxRequest(is_htmx): HxRequest,
    State(state): State<AppState>,
    session: Session,
    Query(filter): Query<UploadFilter>,
) -> Result<Html<String>, StatusCode> {
    let csrf_token = auth::ensure_csrf_token(&session).await;

    // Single JOIN query — fetch all uploads with their references
    let rows = match (&filter.q, &filter.schema) {
        (Some(q), Some(schema_slug)) => {
            let pattern = format!("%{q}%");
            sqlx::query_as::<_, (String, String, String, i64, String, Option<String>, Option<String>)>(
                "SELECT u.hash, u.filename, u.mime, u.size, u.created_at, r.schema_slug, r.entry_id
                 FROM uploads u
                 LEFT JOIN upload_references r ON u.hash = r.upload_hash
                 WHERE u.filename LIKE ? AND r.schema_slug = ?
                 ORDER BY u.created_at DESC"
            )
            .bind(&pattern)
            .bind(schema_slug)
            .fetch_all(&*state.pool)
            .await
        }
        (Some(q), None) => {
            let pattern = format!("%{q}%");
            sqlx::query_as::<_, (String, String, String, i64, String, Option<String>, Option<String>)>(
                "SELECT u.hash, u.filename, u.mime, u.size, u.created_at, r.schema_slug, r.entry_id
                 FROM uploads u
                 LEFT JOIN upload_references r ON u.hash = r.upload_hash
                 WHERE u.filename LIKE ?
                 ORDER BY u.created_at DESC"
            )
            .bind(&pattern)
            .fetch_all(&*state.pool)
            .await
        }
        (None, Some(schema_slug)) => {
            sqlx::query_as::<_, (String, String, String, i64, String, Option<String>, Option<String>)>(
                "SELECT u.hash, u.filename, u.mime, u.size, u.created_at, r.schema_slug, r.entry_id
                 FROM uploads u
                 LEFT JOIN upload_references r ON u.hash = r.upload_hash
                 WHERE r.schema_slug = ?
                 ORDER BY u.created_at DESC"
            )
            .bind(schema_slug)
            .fetch_all(&*state.pool)
            .await
        }
        (None, None) => {
            sqlx::query_as::<_, (String, String, String, i64, String, Option<String>, Option<String>)>(
                "SELECT u.hash, u.filename, u.mime, u.size, u.created_at, r.schema_slug, r.entry_id
                 FROM uploads u
                 LEFT JOIN upload_references r ON u.hash = r.upload_hash
                 ORDER BY u.created_at DESC"
            )
            .fetch_all(&*state.pool)
            .await
        }
    }.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Group rows by upload hash (JOIN produces multiple rows per upload if multiple refs)
    let mut upload_map: std::collections::IndexMap<String, UploadRow> = std::collections::IndexMap::new();
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
            entry.references.push(UploadRef { schema_slug, entry_id });
        }
    }
    let upload_rows: Vec<UploadRow> = upload_map.into_values().collect();

    // Get schema list for filter dropdown — pass simplified structs
    let schemas: Vec<SchemaOption> = schema::list_schemas(&state.config.schemas_dir())
        .unwrap_or_default()
        .into_iter()
        .map(|s| SchemaOption { slug: s.meta.slug, title: s.meta.title })
        .collect();

    let env = state.templates.acquire_env().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let tmpl = env.get_template("uploads/list.html").map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let html = tmpl.render(minijinja::context! {
        base_template => base_for_htmx(is_htmx),
        csrf_token => csrf_token,
        uploads => upload_rows,
        schemas => schemas,
        filter_q => filter.q.unwrap_or_default(),
        filter_schema => filter.schema.unwrap_or_default(),
    }).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
```

Note: This uses `IndexMap` to preserve insertion order (by `created_at DESC`) while grouping. Add `indexmap` to `Cargo.toml` if not already a dependency, or use a `Vec` with manual dedup if you prefer to avoid the dependency.

- [ ] **Step 2: Wire up the route**

In the upload route builder in `src/routes/uploads.rs` (the `pub fn routes()` function, line 13), add `get(list_uploads)` to the root route:

```rust
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_uploads).post(upload_file))
        .route("/file/{hash}/{filename}", get(serve_upload))
        .route("/file/{hash}", get(serve_upload_no_name))
}
```

Add `use axum::routing::get;` if not already imported (it is — line 7).

- [ ] **Step 3: Create the uploads list template**

Create `templates/uploads/list.html`:

```html
{% extends base_template %}
{% block title %}Uploads — Substrukt{% endblock %}
{% block content %}
<div class="flex items-center justify-between mb-6">
  <h1 class="text-2xl font-bold">Uploads</h1>
</div>

<div class="bg-white rounded-lg shadow p-6">
  <form class="flex gap-4 mb-6" hx-get="/uploads" hx-target="#uploads-table" hx-select="#uploads-table" hx-push-url="true" hx-swap="outerHTML">
    <input type="text" name="q" value="{{ filter_q }}" placeholder="Search by filename..."
      class="px-3 py-2 border border-gray-300 rounded-md text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 flex-1">
    <select name="schema" class="px-3 py-2 border border-gray-300 rounded-md text-sm focus:outline-none focus:ring-2 focus:ring-blue-500">
      <option value="">All schemas</option>
      {% for s in schemas %}
      <option value="{{ s.slug }}" {% if filter_schema == s.slug %}selected{% endif %}>{{ s.title }}</option>
      {% endfor %}
    </select>
    <button type="submit" class="bg-blue-600 text-white px-4 py-2 rounded-md hover:bg-blue-700 text-sm font-medium">Filter</button>
  </form>

  <div id="uploads-table">
    {% if uploads %}
    <table class="w-full text-sm">
      <thead>
        <tr class="border-b text-left text-gray-500">
          <th class="pb-2 font-medium">Filename</th>
          <th class="pb-2 font-medium">Type</th>
          <th class="pb-2 font-medium">Size</th>
          <th class="pb-2 font-medium">Created</th>
          <th class="pb-2 font-medium">Used By</th>
          <th class="pb-2 font-medium">Actions</th>
        </tr>
      </thead>
      <tbody>
        {% for upload in uploads %}
        <tr class="border-b hover:bg-gray-50">
          <td class="py-2 font-mono text-xs">{{ upload.filename }}</td>
          <td class="py-2 text-gray-600">{{ upload.mime }}</td>
          <td class="py-2 text-gray-600">{{ upload.size }}</td>
          <td class="py-2 text-gray-600">{{ upload.created_at }}</td>
          <td class="py-2">
            {% if upload.references %}
              {% for ref in upload.references %}
              <a href="/content/{{ ref.schema_slug }}/{{ ref.entry_id }}/edit"
                 class="text-blue-600 hover:underline text-xs">{{ ref.schema_slug }}/{{ ref.entry_id }}</a>{% if not loop.last %}, {% endif %}
              {% endfor %}
            {% else %}
              <span class="text-amber-600 text-xs font-medium">Orphaned</span>
            {% endif %}
          </td>
          <td class="py-2">
            <a href="/uploads/file/{{ upload.hash }}/{{ upload.filename }}" target="_blank"
               class="text-blue-600 hover:underline text-xs mr-2">View</a>
            <a href="/uploads/file/{{ upload.hash }}/{{ upload.filename }}" download
               class="text-blue-600 hover:underline text-xs">Download</a>
          </td>
        </tr>
        {% endfor %}
      </tbody>
    </table>
    {% else %}
    <p class="text-gray-500 text-sm">No uploads found.</p>
    {% endif %}
  </div>
</div>
{% endblock %}
```

- [ ] **Step 4: Add "Uploads" to navigation**

In `templates/_nav.html`, add after the "API Tokens" link (line 12):

```html
  <a href="/uploads" class="block px-3 py-2 rounded hover:bg-gray-700">Uploads</a>
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo build`

If `IndexMap` is needed, add to `Cargo.toml`:
```toml
indexmap = { version = "2", features = ["serde"] }
```

- [ ] **Step 6: Manually test the uploads browser**

Run: `cargo run -- serve`

Test:
1. Navigate to `/uploads` — page should render (empty if no uploads)
2. Check the "Uploads" link appears in sidebar navigation
3. Create a content entry with an upload
4. Go back to `/uploads` — upload should appear with reference
5. Test filename search filter
6. Test schema dropdown filter
7. Click View/Download links
8. Verify htmx partial rendering works (filter changes update table without full page reload)

- [ ] **Step 7: Commit**

```bash
git add src/routes/uploads.rs templates/uploads/list.html templates/_nav.html Cargo.toml Cargo.lock
git commit -m "feat: add uploads browser page with filtering

New page at /uploads lists all uploads with filename, type, size, date,
and content references. Supports filtering by filename and schema type.
Uses single JOIN query with IndexMap grouping. htmx partial rendering."
```

---

## Chunk 4: Export/Import Updates + Integration Tests

### Task 8: Update export/import to use SQLite instead of sidecars

**Files:**
- Modify: `src/sync/mod.rs`
- Modify: `src/routes/api.rs`
- Modify: `src/main.rs`

The CLI `Command::Export` and `Command::Import` currently call sync functions synchronously and have no DB pool. Two approaches:

**Approach chosen:** Keep the sync `export_bundle`/`import_bundle` for CLI use (they don't need upload metadata since the tarball contains the raw files). Add new async variants `export_bundle_async`/`import_bundle_async` for the API endpoints that handle the manifest + SQLite. The CLI paths will work with whatever is on disk.

Actually, the simpler approach: initialize a DB pool in the CLI export/import commands (like `Command::CreateToken` already does at line 100 of `main.rs`).

- [ ] **Step 1: Update export to write uploads-manifest.json**

In `src/sync/mod.rs`, change `export_bundle` to async and add `pool: &SqlitePool` parameter:

```rust
pub async fn export_bundle(data_dir: &Path, pool: &SqlitePool, output: &Path) -> eyre::Result<()> {
    let file = std::fs::File::create(output)?;
    let enc = GzEncoder::new(file, Compression::default());
    let mut tar = tar::Builder::new(enc);

    // Write uploads-manifest.json from SQLite
    let upload_rows = sqlx::query_as::<_, (String, String, String, i64, String)>(
        "SELECT hash, filename, mime, size, created_at FROM uploads"
    )
    .fetch_all(pool)
    .await?;

    let manifest: Vec<serde_json::Value> = upload_rows.iter().map(|(hash, filename, mime, size, created_at)| {
        serde_json::json!({
            "hash": hash,
            "filename": filename,
            "mime": mime,
            "size": size,
            "created_at": created_at,
        })
    }).collect();

    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    let manifest_bytes = manifest_json.as_bytes();
    let mut header = tar::Header::new_gnu();
    header.set_path("uploads-manifest.json")?;
    header.set_size(manifest_bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append(&header, manifest_bytes)?;

    // Add directories (schemas, content, uploads — excluding .meta.json)
    let dirs = ["schemas", "content", "uploads"];
    for dir_name in &dirs {
        let dir = data_dir.join(dir_name);
        if dir.exists() {
            tar.append_dir_all(*dir_name, &dir)?;
        }
    }

    tar.finish()?;
    Ok(())
}
```

- [ ] **Step 2: Update import to read manifest and handle legacy format**

Change `import_bundle` and `import_bundle_from_bytes` to async with `pool: &SqlitePool`:

```rust
pub async fn import_bundle(data_dir: &Path, pool: &SqlitePool, input: &Path) -> eyre::Result<Vec<String>> {
    let file = std::fs::File::open(input)?;
    let dec = GzDecoder::new(file);
    let mut archive = tar::Archive::new(dec);
    archive.unpack(data_dir)?;

    import_upload_metadata(data_dir, pool).await?;

    let warnings = validate_imported_content(data_dir);
    Ok(warnings)
}

pub async fn import_bundle_from_bytes(data_dir: &Path, pool: &SqlitePool, data: &[u8]) -> eyre::Result<Vec<String>> {
    let dec = GzDecoder::new(data);
    let mut archive = tar::Archive::new(dec);
    archive.unpack(data_dir)?;

    import_upload_metadata(data_dir, pool).await?;

    let warnings = validate_imported_content(data_dir);
    Ok(warnings)
}

/// Handle upload metadata after import — manifest or legacy sidecars.
async fn import_upload_metadata(data_dir: &Path, pool: &SqlitePool) -> eyre::Result<()> {
    let manifest_path = data_dir.join("uploads-manifest.json");
    if manifest_path.exists() {
        // New format: read manifest
        let manifest_str = std::fs::read_to_string(&manifest_path)?;
        let manifest: Vec<crate::uploads::UploadMeta> = serde_json::from_str(&manifest_str)?;
        for meta in &manifest {
            crate::uploads::db_insert_upload(pool, meta).await?;
        }
        std::fs::remove_file(&manifest_path)?;
    } else {
        // Legacy format: migrate .meta.json sidecars
        let uploads_dir = data_dir.join("uploads");
        crate::uploads::migrate_meta_sidecars(&uploads_dir, data_dir, pool).await?;
    }

    // Rebuild upload references from imported content
    crate::uploads::populate_references_from_content(data_dir, pool).await?;

    Ok(())
}
```

- [ ] **Step 3: Update CLI export/import commands in main.rs**

In `src/main.rs`, the CLI `Command::Export` and `Command::Import` need a DB pool. Follow the same pattern as `Command::CreateToken` (line 100):

```rust
Command::Import { path } => {
    let pool = db::init_pool(&config.db_path).await?;
    let warnings = sync::import_bundle(&config.data_dir, &pool, &path).await?;
    // ... rest unchanged
}
Command::Export { path } => {
    let pool = db::init_pool(&config.db_path).await?;
    sync::export_bundle(&config.data_dir, &pool, &path).await?;
    tracing::info!("Exported to {}", path.display());
    Ok(())
}
```

- [ ] **Step 4: Update API callers in api.rs**

In `src/routes/api.rs`:
- `export_bundle()` handler: pass `&state.pool` to `sync::export_bundle` (currently uses `sync::export_bundle_to_bytes` — check if this exists or if the API uses `export_bundle` with a temp file). Update the call accordingly.
- `import_bundle()` handler: pass `&state.pool` to `sync::import_bundle_from_bytes`.

Both calls need `.await` added.

- [ ] **Step 5: Run all tests**

Run: `cargo test`

Expected: all existing tests pass, including the export/import test (`api_export_import`).

- [ ] **Step 6: Commit**

```bash
git add src/sync/mod.rs src/routes/api.rs src/main.rs
git commit -m "feat: update export/import to use uploads-manifest.json

Export writes uploads-manifest.json from SQLite into the tarball.
Import reads the manifest, or falls back to legacy .meta.json sidecars
for backward compatibility. CLI commands now init DB pool for export/import.
Extracted populate_references_from_content for reuse after import."
```

### Task 9: Add integration tests

**Files:**
- Modify: `tests/integration.rs`

- [ ] **Step 1: Add test for upload reference tracking**

```rust
#[tokio::test]
async fn upload_reference_tracking() {
    let server = TestServer::start().await;
    server.setup_admin().await;

    // Create schema with upload field
    let schema = serde_json::json!({
        "x-substrukt": { "title": "Photos", "slug": "photos", "storage": "directory" },
        "type": "object",
        "properties": {
            "title": { "type": "string", "title": "Title" },
            "image": { "type": "string", "title": "Image", "format": "upload" }
        },
        "required": ["title"]
    });
    server.create_schema(&schema.to_string()).await;

    // Create entry with upload
    let csrf = server.get_csrf("/content/photos/new").await;
    let form = reqwest::multipart::Form::new()
        .text("_csrf", csrf)
        .text("title", "Test Photo")
        .part("image", reqwest::multipart::Part::bytes(b"fake image data".to_vec())
            .file_name("test.jpg")
            .mime_str("image/jpeg").unwrap());
    let resp = server.client.post(server.url("/content/photos/new"))
        .multipart(form)
        .send().await.unwrap();
    assert!(resp.status().is_redirection() || resp.status().is_success());

    // Check uploads page shows the upload with reference
    let resp = server.client.get(server.url("/uploads"))
        .send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("test.jpg"));
    assert!(body.contains("photos"));
    assert!(!body.contains("Orphaned"));
}
```

- [ ] **Step 2: Add test for uploads browser filtering**

```rust
#[tokio::test]
async fn uploads_browser_filtering() {
    let server = TestServer::start().await;
    server.setup_admin().await;

    let schema = serde_json::json!({
        "x-substrukt": { "title": "Photos", "slug": "photos", "storage": "directory" },
        "type": "object",
        "properties": {
            "title": { "type": "string", "title": "Title" },
            "image": { "type": "string", "title": "Image", "format": "upload" }
        },
        "required": ["title"]
    });
    server.create_schema(&schema.to_string()).await;

    // Upload a file via content creation
    let csrf = server.get_csrf("/content/photos/new").await;
    let form = reqwest::multipart::Form::new()
        .text("_csrf", csrf)
        .text("title", "Beach")
        .part("image", reqwest::multipart::Part::bytes(b"beach data".to_vec())
            .file_name("beach.jpg")
            .mime_str("image/jpeg").unwrap());
    server.client.post(server.url("/content/photos/new"))
        .multipart(form)
        .send().await.unwrap();

    // Filter by filename — should match
    let resp = server.client.get(server.url("/uploads?q=beach"))
        .send().await.unwrap();
    let body = resp.text().await.unwrap();
    assert!(body.contains("beach.jpg"));

    // Filter by non-matching filename — should not match
    let resp = server.client.get(server.url("/uploads?q=mountain"))
        .send().await.unwrap();
    let body = resp.text().await.unwrap();
    assert!(!body.contains("beach.jpg"));

    // Filter by schema
    let resp = server.client.get(server.url("/uploads?schema=photos"))
        .send().await.unwrap();
    let body = resp.text().await.unwrap();
    assert!(body.contains("beach.jpg"));
}
```

- [ ] **Step 3: Add test for sidecar migration**

This test creates `.meta.json` files directly, then starts a server (which triggers migration), and verifies the data appears in the uploads browser:

```rust
#[tokio::test]
async fn upload_sidecar_migration() {
    // Create a temp dir and manually place .meta.json sidecar + upload file
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path();

    // Create uploads directory structure
    let uploads_dir = data_dir.join("uploads").join("ab");
    std::fs::create_dir_all(&uploads_dir).unwrap();

    let hash = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
    let file_path = uploads_dir.join(&hash[2..]);
    std::fs::write(&file_path, b"test file data").unwrap();

    let meta = serde_json::json!({
        "hash": hash,
        "filename": "migrated.jpg",
        "mime": "image/jpeg",
        "size": 14
    });
    let meta_path = uploads_dir.join(format!("{}.meta.json", &hash[2..]));
    std::fs::write(&meta_path, serde_json::to_string(&meta).unwrap()).unwrap();

    // Also create a schema + content referencing this upload
    let schemas_dir = data_dir.join("schemas");
    std::fs::create_dir_all(&schemas_dir).unwrap();
    let schema = serde_json::json!({
        "x-substrukt": { "title": "Gallery", "slug": "gallery", "storage": "directory" },
        "type": "object",
        "properties": {
            "title": { "type": "string", "title": "Title" },
            "image": { "type": "string", "title": "Image", "format": "upload" }
        }
    });
    std::fs::write(schemas_dir.join("gallery.json"), serde_json::to_string(&schema).unwrap()).unwrap();

    let content_dir = data_dir.join("content").join("gallery");
    std::fs::create_dir_all(&content_dir).unwrap();
    let entry = serde_json::json!({
        "title": "Test",
        "image": { "hash": hash, "filename": "migrated.jpg", "mime": "image/jpeg" }
    });
    std::fs::write(content_dir.join("test.json"), serde_json::to_string(&entry).unwrap()).unwrap();

    // Start server with this data dir (migration runs on startup)
    let server = TestServer::start_with_data_dir(data_dir).await;
    server.setup_admin().await;

    // Verify .meta.json was deleted
    assert!(!meta_path.exists());

    // Verify upload appears in browser
    let resp = server.client.get(server.url("/uploads"))
        .send().await.unwrap();
    let body = resp.text().await.unwrap();
    assert!(body.contains("migrated.jpg"));
    assert!(body.contains("gallery"));
}
```

Note: This test requires a `TestServer::start_with_data_dir()` variant that uses a pre-populated data directory instead of an empty temp dir. If this doesn't exist, add it as a small helper to the `TestServer` impl. It should be a straightforward copy of `start()` that takes a `&Path` instead of creating a new `TempDir`.

- [ ] **Step 4: Add test for export/import roundtrip with manifest**

```rust
#[tokio::test]
async fn export_import_with_upload_manifest() {
    let server = TestServer::start().await;
    server.setup_admin().await;
    let token = server.create_api_token("test").await;

    // Create schema + content with upload via API
    let schema = serde_json::json!({
        "x-substrukt": { "title": "Docs", "slug": "docs", "storage": "directory" },
        "type": "object",
        "properties": {
            "title": { "type": "string", "title": "Title" },
            "file": { "type": "string", "title": "File", "format": "upload" }
        },
        "required": ["title"]
    });
    server.create_schema(&schema.to_string()).await;

    // Upload via web UI
    let csrf = server.get_csrf("/content/docs/new").await;
    let form = reqwest::multipart::Form::new()
        .text("_csrf", csrf)
        .text("title", "Manual")
        .part("file", reqwest::multipart::Part::bytes(b"pdf content".to_vec())
            .file_name("manual.pdf")
            .mime_str("application/pdf").unwrap());
    server.client.post(server.url("/content/docs/new"))
        .multipart(form)
        .send().await.unwrap();

    // Export via API
    let resp = server.client.post(server.url("/api/v1/export"))
        .bearer_auth(&token)
        .send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let bundle = resp.bytes().await.unwrap();

    // Import into a fresh server
    let server2 = TestServer::start().await;
    server2.setup_admin().await;
    let token2 = server2.create_api_token("test").await;

    let resp = server2.client.post(server2.url("/api/v1/import"))
        .bearer_auth(&token2)
        .body(bundle.to_vec())
        .header("content-type", "application/gzip")
        .send().await.unwrap();
    assert!(resp.status().is_success());

    // Verify upload appears in the new server's uploads browser
    let resp = server2.client.get(server2.url("/uploads"))
        .send().await.unwrap();
    let body = resp.text().await.unwrap();
    assert!(body.contains("manual.pdf"));
}
```

- [ ] **Step 5: Run all tests**

Run: `cargo test`

Expected: all tests pass including the new ones.

- [ ] **Step 6: Commit**

```bash
git add tests/integration.rs
git commit -m "test: add integration tests for uploads browser, migration, and export/import

- upload_reference_tracking: verify uploads appear in browser with refs
- uploads_browser_filtering: verify filename and schema filters work
- upload_sidecar_migration: verify .meta.json files are migrated on startup
- export_import_with_upload_manifest: verify roundtrip preserves upload metadata"
```

---

## Final Steps

### Task 10: Final verification and merge

- [ ] **Step 1: Run the full test suite**

Run: `cargo test`

Expected: all tests pass.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy`

Expected: no warnings.

- [ ] **Step 3: Run fmt**

Run: `cargo fmt`

- [ ] **Step 4: Manual smoke test**

Run: `cargo run -- serve`

Walk through:
1. Schema editor: create new schema, edit existing, verify vanilla-jsoneditor works
2. Uploads browser: view page, create content with uploads, verify uploads appear with references, test filters
3. Existing functionality: verify content CRUD, API, export/import still work
4. Check "Uploads" link in sidebar navigation

- [ ] **Step 5: Merge branch to main**

```bash
git checkout main
git merge <branch-name>
```
