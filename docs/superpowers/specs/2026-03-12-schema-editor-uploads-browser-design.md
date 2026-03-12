# Schema Editor Upgrade + Uploads Browser

Date: 2026-03-12

## Overview

Two features for the Substrukt CMS admin UI:

1. **Schema Editor Upgrade** — Replace the plain textarea with `vanilla-jsoneditor` for a proper code editing experience
2. **Uploads Browser** — New page to browse all uploads, backed by SQLite for metadata and reference tracking

---

## Feature 1: Schema Editor Upgrade

### Problem

The schema edit page (`/schemas/{slug}/edit`) uses a plain `<textarea>` for JSON editing. This means no auto-indent on newline, no tab key support, no syntax highlighting, no bracket matching, and no formatting. Editing JSON Schemas is painful.

### Solution

Replace the textarea with [vanilla-jsoneditor](https://github.com/josdejong/svelte-jsoneditor) — a standalone JSON editor component with text and tree modes, syntax highlighting, validation, and formatting built in.

### Integration

- Load the standalone bundle from CDN with a pinned major version: `cdn.jsdelivr.net/npm/vanilla-jsoneditor@2/standalone.js`
- Load in the schema edit template only — not in `base.html`
- Create a `<div>` target element where the textarea currently is
- Initialize with `createJSONEditor` in **text mode** by default (raw JSON with proper editor behavior)
- User can toggle to **tree mode** for visual structure navigation
- The existing `<form>` with hidden `_csrf` field and standard POST submission stays unchanged
- Replace `<textarea name="schema_json">` with `<input type="hidden" name="schema_json">` + editor div
- On form submit: a JS handler copies editor content into the hidden input, then the form submits normally (preserving CSRF handling)
- Skip `createAjvValidator` for now — server-side validation on submit is sufficient. Can add real-time meta-schema validation later.

### Behavior

- **Text mode** (default): syntax highlighting, auto-indent, bracket matching, format button
- **Tree mode** (toggle): visual browse/edit of schema structure
- Server-side validation remains as a safety net
- No changes to schema storage or API — purely a UI enhancement

### Files Changed

- `templates/schemas/edit.html` — replace textarea with editor div + hidden input + initialization script

---

## Feature 2: Uploads Browser

### Problem

There is no way to see all uploads in the system. Uploads are only visible within the content items that reference them. There's no way to find orphaned uploads or filter uploads by type.

### Solution

1. Track upload metadata and content references in SQLite (replacing `.meta.json` sidecars)
2. Add a new uploads browser page at `GET /uploads`

### Database Schema

```sql
CREATE TABLE uploads (
    hash TEXT PRIMARY KEY,
    filename TEXT NOT NULL,
    mime TEXT NOT NULL,
    size INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE upload_references (
    upload_hash TEXT NOT NULL REFERENCES uploads(hash),
    schema_slug TEXT NOT NULL,
    entry_id TEXT NOT NULL,
    PRIMARY KEY (upload_hash, schema_slug, entry_id)
);
```

Notes:
- `entry_id` (not `content_slug`) to match the existing `ContentEntry.id` naming convention
- `field_name` dropped from the primary key — for the purpose of "which content items use this upload," the (hash, schema, entry) triple is sufficient. Uploads can appear in nested objects or arrays, and tracking the exact field path adds complexity without clear value for the browser use case.
- Enable `PRAGMA foreign_keys = ON` on the SQLite connection pool to enforce referential integrity.

### Extracting Upload References from Content

Upload objects in content JSON have the shape `{"hash": "...", "filename": "...", "mime": "..."}`. These can appear:
- At the top level of a content entry
- Nested inside object-type fields
- Inside array items

**Strategy**: recursively walk the content JSON tree. Any object containing a `hash` key with a string value that matches the hex SHA-256 pattern is treated as an upload reference. Collect all unique hashes found.

### Behavior Changes

**On upload** (`store_upload`):
- Write file to disk (same content-addressed path as today, no `.meta.json` sidecar)
- `INSERT OR IGNORE` into `uploads` table (content-addressed dedup)

**On content save** (create or update):
- Extract upload hashes from the content JSON (recursive walk)
- Delete all existing `upload_references` rows for this (schema_slug, entry_id)
- Insert new refs for all hashes found
- This replace-all approach is simpler and safer than diffing

**On content delete**:
- Delete all `upload_references` rows for this (schema_slug, entry_id)

**Migration** (on startup):
- Check if `uploads` table is empty — if populated, skip migration (idempotency guard)
- Scan existing `.meta.json` files, `INSERT OR IGNORE` into `uploads` table
- Scan content files, populate `upload_references`
- Delete `.meta.json` sidecars only after all inserts succeed
- If migration crashes partway through, `INSERT OR IGNORE` makes it safe to re-run

### Uploads Browser Page

**Route:** `GET /uploads`

**Layout:** Table with columns:
- Filename (linked to view)
- MIME type
- Size (human-readable)
- Created date
- Used By (content item title + schema name, linked to edit page; multiple if shared; "orphaned" indicator if none)
- Actions: view / download links

**Filtering:**
- Text input for filename search
- Dropdown for schema type filter (populated from existing schemas)
- Server-side via query params: `?q=filename&schema=slug`
- htmx partial rendering on filter change for responsive UX

**Not included:**
- No delete functionality (risk of breaking content references; can add later)
- No upload from this page (uploads happen through content forms)
- No pagination initially — typical deployments expected to have fewer than a few hundred uploads. Add if needed.

### Export/Import Adjustments

- **Export**: query `uploads` table to write a `uploads-manifest.json` file into the tarball (array of `{hash, filename, mime, size}` objects). Upload files on disk are included as before.
- **Import**: read `uploads-manifest.json` from tarball, `INSERT OR IGNORE` into `uploads` table. Populate `upload_references` by scanning imported content. For backward compatibility, also handle tarballs containing `.meta.json` sidecars (pre-migration format).

### Files Changed

- `src/uploads/mod.rs` — remove `.meta.json` read/write, add SQLite operations, add recursive hash extraction
- `src/routes/uploads.rs` — add `GET /uploads` handler with list/filter logic
- `src/routes/content.rs` — update reference tracking on content save/delete
- `src/routes/api.rs` — same reference tracking for API content CRUD and upload endpoints
- `src/sync/mod.rs` — update export to write manifest, import to read manifest + handle legacy sidecars
- `src/db/mod.rs` — enable `PRAGMA foreign_keys = ON` on connection pool
- `templates/uploads/list.html` — new template for uploads browser
- `templates/base.html` — add "Uploads" link to navigation
- SQL migration file for new tables
- Startup migration logic for existing `.meta.json` files

### Testing

- Integration test: migration from `.meta.json` sidecars to SQLite (set up sidecars, run migration, verify DB state, verify sidecars deleted)
- Integration test: reference tracking lifecycle (create content with uploads, verify refs; update content changing uploads, verify refs updated; delete content, verify refs removed)
- Integration test: `GET /uploads` returns 200 with correct data
- Integration test: export/import roundtrip with new manifest format
- Schema editor: manual testing sufficient (pure frontend change)
