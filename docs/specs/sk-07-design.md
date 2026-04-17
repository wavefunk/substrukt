# SK-07: Media Management in Admin UI

## Motivation

Substrukt has content-addressed file uploads but lacks the media management capabilities expected by content-heavy sites, particularly those migrating from CMS platforms like Payload CMS. The current upload experience is functional but minimal:

- Uploading is tied to content forms -- there's no way to pre-upload or browse existing media.
- The upload list (`/apps/{app}/uploads`) is a flat table with no image previews, no gallery view, and no pagination.
- There's no way to reuse an existing upload from the media library when editing a content entry -- every upload field requires a new file.
- No image transformation support (crop, resize, focal-point selection) for responsive images.
- No way to delete orphaned uploads.

This feature adds gallery-style browsing, a media picker for content forms, image crop/resize with focal-point selection, and upload deletion.

## Existing Baseline

### Upload storage

- **Content-addressed**: files stored at `data/<app>/uploads/<first-2-hex>/<remaining-hex>`, keyed by SHA-256 of raw bytes (`src/uploads/mod.rs:84-124`, `store_upload` function).
- **SQLite metadata**: `uploads` table with `(app_id, hash)` composite PK, storing `filename`, `mime`, `size`, `created_at`. `upload_references` table tracks which content entries reference each upload (`migrations/006_create_apps.sql`).
- **MIME allowlist**: `DEFAULT_ALLOWED_MIMES` in `uploads/mod.rs:9-28` -- images, PDFs, video, audio, text, archives.

### Upload UI

- **Upload field in forms** (`form.rs:335-385`): drag-and-drop zone with current-file preview (image thumbnail for image types, filename/mime for others). Hidden `__current` input preserves existing upload reference.
- **Upload list page** (`routes/uploads.rs`, `templates/uploads/list.html`): table with Filename, Type, Size, Created, Used By (references), Actions (View, Download). Filterable by filename search and schema. Shows "Orphaned" badge for unreferenced uploads.
- **File serving**: `GET /apps/{app}/uploads/file/{hash}/{filename}` with ETag support (hash is natural ETag).
- **API upload**: `POST /api/v1/apps/{app}/uploads` for programmatic uploads.

### What's missing (gaps this spec addresses)

1. **Gallery/grid view** -- visual browsing with thumbnails instead of a table.
2. **Media picker** -- reuse existing uploads from content entry forms without re-uploading.
3. **Image transformations** -- crop, resize for responsive images.
4. **Focal-point selection** -- define a center of interest for CSS `object-position` or server-side crop.
5. **Upload deletion** -- no route to delete uploads, orphaned files accumulate.
6. **Pagination** -- current list loads all uploads at once, no LIMIT.

## Architecture

### Core principle: immutable originals, on-demand derivatives

The content-addressed storage model is the architectural pivot. Since files are keyed by SHA-256, any byte-level transformation (crop, resize) produces a new hash. Two approaches:

1. **Replace**: transform modifies the file, producing a new hash. Original becomes orphaned. Loses source of truth.
2. **Derivatives** (chosen): original is immutable. Transforms generate derived images cached alongside the original. Focal point is metadata, not baked into pixels.

**Rationale for derivatives:**
- The original upload is always recoverable -- no destructive operations on source material.
- Multiple content entries can reference the same original with different crops/sizes.
- Derivatives are regeneratable from the original, so cache eviction is safe.
- Focal point as metadata allows different consumers to apply it differently (CSS `object-position` vs. server-side crop).

### 1. Gallery View

Replace the table-only upload list with a switchable table/grid view.

**Grid mode**: card layout with image thumbnail (for image types), filename, size, and an "Orphaned" badge. Cards are clickable to open a detail modal. Non-image files show a file-type icon instead of a thumbnail.

**Implementation**: add a toggle button (table/grid) in the uploads list header. Grid view uses the same data as the table, rendered differently. Toggle state persists in `localStorage`.

**Pagination**: add `LIMIT/OFFSET` to the four SQL query branches in `routes/uploads.rs:66-161`. Default page size of 50. Pagination controls at the bottom (matching the existing content list pattern from `routes/content.rs` which uses `PAGE_SIZE: usize = 50`).

**Refactor note**: the four SQL branches (no filter, query only, schema only, query+schema) should be consolidated using dynamic query building with `sqlx::query_builder` or a shared base query with optional WHERE clauses. The current duplication will get worse with pagination.

### 2. Upload Detail / Preview Modal

Clicking an upload (in grid or table view) opens a detail modal showing:
- Full-size image preview (for image types), or a download link for non-images.
- Metadata: filename, MIME type, size, created date.
- Content references: links to entries that use this upload.
- Focal point editor (for images only -- see Section 5).
- Delete button (see Section 6).

Implementation: htmx-powered modal. Clicking an upload triggers `hx-get="/apps/{app}/uploads/{hash}/detail"` which returns an HTML fragment for the modal content.

**New route** in `routes/uploads.rs`:

```
GET /apps/{app}/uploads/{hash}/detail
```

Returns an HTML partial with the upload detail view.

### 3. Media Picker

When editing a content entry with an upload field, the editor should be able to choose from existing uploads instead of uploading a new file.

**UI**: add a "Browse Media" button next to the existing drag-and-drop zone in the upload field renderer (`form.rs:335`). Clicking it opens a modal showing the media gallery (grid view, with search/filter, paginated). Selecting an upload writes its `{hash, filename, mime}` object to the hidden `__current` input and updates the preview.

**Implementation**:
1. New route `GET /apps/{app}/uploads/picker` returns the gallery grid as an htmx partial (same data, selection mode).
2. Each upload card in picker mode has a "Select" button.
3. JavaScript handler on selection: writes the upload object to the field's hidden input, updates the preview, closes the modal.

**Integration with form.rs**: the `render_field` function for upload fields (`form.rs:335-385`) gains a "Browse Media" button alongside the drop zone. The button triggers an htmx-powered modal.

### 4. Image Transformations (Crop/Resize)

**On-demand derivatives**: transforms are applied lazily on first request and cached to disk.

**URL shape**:

```
GET /apps/{app}/uploads/file/{hash}/{filename}?w=400&h=300&fit=cover&focal=auto
```

Query parameters:
- `w`: target width in pixels (optional).
- `h`: target height in pixels (optional).
- `fit`: resize strategy -- `contain` (letterbox), `cover` (fill and crop), `scale` (stretch). Default: `contain`.
- `focal`: `auto` (use stored focal point), or `{x},{y}` (explicit 0.0-1.0 coordinates), or omitted (center crop). Only applicable when `fit=cover`.

If no transform parameters are provided, the original file is served as-is (existing behavior, no change).

**Derivative storage**:

```
data/<app>/uploads/_derived/<original-hash>/<params-hash>
```

Where `<params-hash>` is the SHA-256 of the canonical transform parameter string (e.g., `w=400&h=300&fit=cover&focal_x=0.5&focal_y=0.3`). A `.meta.json` sidecar stores the parameter set for debugging.

**Serving flow**:
1. Parse transform parameters from query string.
2. Compute `params-hash`.
3. Check if derivative exists at `_derived/<original-hash>/<params-hash>`.
4. If yes: serve from disk with ETag = `"{params-hash}"`.
5. If no: load original, apply transform, write derivative to disk, serve.

**Transform logic** (Rust, pure-Rust image crate):
- Load image from original file path.
- Resize to target dimensions using the specified fit mode.
- For `fit=cover` with focal point: shift the crop window toward the focal point coordinates.
- Encode as the original format (JPEG -> JPEG, PNG -> PNG, WebP -> WebP).
- Write to derivative path.

**Image processing library**: the spec does not prescribe a specific crate. The `image` crate (pure Rust, no native deps) is the safe default. `fast_image_resize` offers better performance if needed. Avoiding native dependencies (ImageMagick, libvips) keeps the Nix build simple.

**Constraints**:
- Only raster image types (`image/jpeg`, `image/png`, `image/gif`, `image/webp`). SVG and non-image types return 400 for transform requests.
- Animated GIFs: transforms only apply to the first frame. The result is a static image. Document this limitation.
- Maximum dimensions: cap at 4096x4096 to prevent memory abuse.
- If the requested dimensions exceed the original, no upscaling. Return original dimensions.

### 5. Focal Point Selection

**Data model**: add `focal_x` and `focal_y` columns to the `uploads` table. Nullable REALs, default NULL (center-crop). Values range 0.0-1.0 where (0,0) is top-left.

**Migration**:
```sql
ALTER TABLE uploads ADD COLUMN focal_x REAL;
ALTER TABLE uploads ADD COLUMN focal_y REAL;
```

**UI**: the upload detail modal for image types includes a focal-point editor. Shows the image with a draggable crosshair overlay. Clicking/dragging sets the focal point coordinates. A "Reset" button clears to NULL (center-crop).

**Saving**: `PUT /apps/{app}/uploads/{hash}/focal` accepts `{ "x": 0.35, "y": 0.6 }` and updates the SQLite record.

**Consumption**: when `focal=auto` is used in a transform URL, the focal point is loaded from SQLite. If NULL, defaults to center (0.5, 0.5).

**The focal point does NOT modify entry JSON.** The stored upload reference in content entries remains `{hash, filename, mime}`. The focal point lives in SQLite on the upload record and is resolved at serve time. This keeps the entry data stable and avoids a breaking format change.

### 6. Upload Deletion

**Route** in `routes/uploads.rs`:

```
DELETE /apps/{app}/uploads/{hash}
POST  /apps/{app}/uploads/{hash}/delete  (for HTML form fallback)
```

**Auth**: editor+ role required.

**Behavior**:
- **If referenced**: reject with 409 Conflict. Return the list of referencing entries so the user can remove references first. Do not cascade-delete referenced uploads -- too destructive.
- **If orphaned**: delete the file from disk, remove the SQLite record, and delete any derivatives in `_derived/<hash>/`.

**UI**: the upload detail modal shows a "Delete" button. If the upload is referenced, the button is disabled with a tooltip: "Remove all references before deleting." If orphaned, clicking shows a confirmation dialog.

**API equivalent**: `DELETE /api/v1/apps/{app}/uploads/{hash}` with same behavior (409 if referenced, 204 on success).

## Data Models

### Updated `uploads` table

```sql
ALTER TABLE uploads ADD COLUMN focal_x REAL;
ALTER TABLE uploads ADD COLUMN focal_y REAL;
```

### Updated `UploadMeta` struct

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UploadMeta {
    pub hash: String,
    pub filename: String,
    pub mime: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focal_x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focal_y: Option<f64>,
}
```

**Note:** The current `UploadMeta` (`uploads/mod.rs:42-48`) does not include `created_at`, even though the SQL table has it and the export manifest includes it. The import path (`sync/mod.rs:107`) deserializes the manifest as `Vec<UploadMeta>` and `serde` ignores unknown fields by default, so `created_at` is silently dropped during import -- uploads get a fresh `created_at` from the SQL `DEFAULT (datetime('now'))`. This is pre-existing behavior unrelated to this feature. Adding `focal_x`/`focal_y` to `UploadMeta` with `serde(default)` follows the same pattern: old manifests without these fields import cleanly (defaulting to `None`).

### Derivative metadata (on-disk sidecar)

```json
{
  "original_hash": "abc123...",
  "params": { "w": 400, "h": 300, "fit": "cover", "focal_x": 0.35, "focal_y": 0.6 },
  "created_at": "2026-04-17T12:00:00Z"
}
```

### New API endpoints summary

| Method | Path | Description | Auth |
|--------|------|-------------|------|
| GET | `/uploads/picker` | Media picker gallery (htmx partial) | viewer+ |
| GET | `/uploads/{hash}/detail` | Upload detail modal (htmx partial) | viewer+ |
| PUT | `/uploads/{hash}/focal` | Set focal point | editor+ |
| DELETE | `/uploads/{hash}` | Delete upload | editor+ |
| POST | `/uploads/{hash}/delete` | Delete upload (form fallback) | editor+ |

Transform parameters are query params on the existing file serving route, not new endpoints.

## Error Handling

| Scenario | Behavior |
|---|---|
| Transform on non-image type | 400: "Transforms only supported for raster images" |
| Transform dimensions > 4096 | 400: "Maximum dimension is 4096px" |
| Invalid focal point coordinates | 400: "Focal point must be between 0.0 and 1.0" |
| Delete referenced upload | 409: `{ "error": "Upload is referenced by N entries", "references": [...] }` |
| Delete non-existent upload | 404 |
| Derivative generation fails (corrupt image) | 500, log error, serve original as fallback |
| Disk full during derivative write | 500, log error, generate and serve in-memory without caching |

## Edge Cases

### SVG files

SVG is a vector format. Crop/resize transforms don't apply in the same way as raster images. Transform requests on SVG files return 400. SVGs are served as-is at their original size. The gallery shows the SVG rendered at a fixed thumbnail size via `<img>` tag.

### Animated GIF

The `image` crate decodes animated GIFs but transforms apply to the first frame only. The result is a static image. The original animated GIF is always accessible at the original URL (no transform params). Document this limitation in the UI: "Note: resizing animated GIFs produces a static image."

### Non-image file types

PDFs, videos, audio, text, archives: no thumbnails, no transforms. The gallery grid shows a file-type icon (SVG icon based on MIME category). The detail modal shows metadata and a download link. Video types could potentially extract a thumbnail frame as a future enhancement.

### Content-addressed deduplication

If two different content entries upload the same file (same bytes), they share the same hash and disk file. The `uploads` table has `(app_id, hash)` as PK, so one metadata record. Both entries reference it via `upload_references`. Deletion is safe because it checks references -- both entries must remove their references before the upload can be deleted.

### Focal point on non-image uploads

Focal point columns are nullable. For non-image uploads, the focal point editor is not shown in the UI. If somehow set via API, the values are stored but ignored (no transforms on non-images).

### Export/import

The `_derived/` directory should NOT be included in export bundles. Derivatives are regeneratable from originals and would bloat bundles. Update `sync/mod.rs:50` to exclude `_derived` from the tar archive (currently archives `["schemas", "content", "uploads", "_history"]`).

Focal point data lives in SQLite (`uploads` table), which is already included in the upload manifest during export. Two things need updating for round-trip:
1. **`UploadMeta` struct** (`uploads/mod.rs:42-48`): add `focal_x: Option<f64>` and `focal_y: Option<f64>` fields. The import path at `sync/mod.rs:107` deserializes `UploadMeta` from the manifest, so this is required for import.
2. **Export query and manifest builder** (`sync/mod.rs:21-48`): the export does NOT use `UploadMeta` serialization -- it builds JSON manually from a raw SQL query `SELECT hash, filename, mime, size, created_at`. The query must be extended to `SELECT hash, filename, mime, size, created_at, focal_x, focal_y` and the `serde_json::json!` block updated to include these fields. Without this, focal points are silently dropped during export.

### Backup

The `_derived/` directory adds disk usage that is 100% regeneratable. S3 backups (`src/backup.rs`) archive the full data directory via `tar.gz`, which would include `_derived/`. For large media-heavy sites, this could be significant. Options:
- Excluding `_derived/` from backups (would require modifying the backup archive creation to filter it out).
- Accepting the extra size (simpler, ensures fast restores without a regeneration step).

Recommend accepting the extra size for simplicity, with a note that operators can add `_derived/` to backup exclusions if size becomes an issue.

### File watcher

Writes to `_derived/` should not trigger cache rebuilds (same issue as `_history/` from SK-01). The watcher path filter should also exclude `/_derived/`.

### Orphan cleanup

Currently there's no automated orphan detection or cleanup. With the delete route, admins can manually delete orphaned uploads. A future enhancement could add a "Clean up orphans" button that deletes all unreferenced uploads in one action. For now, the "Orphaned" badge in the upload list and individual delete is sufficient.

### Concurrent derivative generation

Two requests for the same transform can race. Both compute and write the same derivative. This is safe because the content is identical (deterministic transform of the same original + params). The second write overwrites the first with the same bytes. No locking needed.

### SQL query consolidation

The four SQL branches in `routes/uploads.rs:66-161` should be refactored into a single query builder when adding pagination. Using `sqlx::QueryBuilder` or a dynamic string approach with `sqlx::AssertSqlSafe()` (consistent with the existing pattern noted in NOTES.md). All user-supplied values use bind parameters.

## Non-Goals

- **Pixel-level image editing**: drawing, filters, color correction, overlays. This is a CMS, not Photoshop.
- **Video transcoding or thumbnail extraction**: only image formats support transforms. Video thumbnails could be a future enhancement.
- **AI-assisted tagging, alt-text generation, auto-crop**: out of scope for core CMS.
- **Folder/tag hierarchy for organizing uploads**: uploads remain flat, filtered by metadata (filename search, schema filter). Tagging could be added later.
- **EXIF rotation/stripping**: the `image` crate handles EXIF orientation on read by default. EXIF metadata is not stripped from originals (privacy consideration is the uploader's responsibility). Derivatives inherit the rotation-corrected orientation.
- **Upload versioning / replace-in-place**: replacing an upload with new content changes the hash, which means new references everywhere. True version replacement (same hash, different content) violates the content-addressing model.
- **Cross-app media sharing**: uploads are scoped per-app. Sharing media across apps requires re-uploading or API-level copy.
- **Client-side image manipulation before upload**: all transforms are server-side. Client-side crop-before-upload could be a future enhancement.

## Implementation Sequence

### Phase 1: Gallery view + pagination

1. Refactor SQL queries in `routes/uploads.rs` into a single query builder with pagination.
2. Add grid/table toggle to `templates/uploads/list.html`.
3. Add pagination controls (matching existing content list pattern).
4. Image thumbnails in grid cards (use existing file serving route at a constrained `<img>` size).

### Phase 2: Upload detail modal + delete

1. Add `GET /uploads/{hash}/detail` route returning htmx partial.
2. Create `templates/uploads/detail.html` modal template.
3. Add `DELETE /uploads/{hash}` and `POST /uploads/{hash}/delete` routes.
4. Reference checking before deletion (query `upload_references`).
5. Disk cleanup: delete file, derivatives, SQLite records.

### Phase 3: Media picker

1. Add `GET /uploads/picker` route returning gallery in selection mode.
2. Create `templates/uploads/picker.html` partial.
3. Update `form.rs` upload field renderer to include "Browse Media" button.
4. JavaScript handler: open modal, select upload, write to hidden input, update preview.

### Phase 4: Focal point

1. SQLite migration: add `focal_x`, `focal_y` to `uploads` table.
2. Update `UploadMeta` struct.
3. Add `PUT /uploads/{hash}/focal` route.
4. Focal point editor UI: draggable crosshair on image in detail modal.
5. Update export/import manifest to include focal point data.

### Phase 5: Image transforms

1. Add image processing dependency to `Cargo.toml`.
2. Implement transform parameter parsing on the file serving route.
3. Implement resize/crop logic with focal point support.
4. Derivative storage at `_derived/<hash>/<params-hash>`.
5. Cache check + serve flow.
6. Exclude `_derived/` from file watcher and export bundles.

### Phase 6: Polish

1. File-type icons for non-image uploads in gallery grid.
2. Animated GIF handling documentation.
3. SVG handling (no transforms, rendered preview).
4. Maximum dimension validation.
5. API endpoint for upload deletion.

## Testing Strategy

- **Unit tests**: transform parameter parsing, focal-point clamp validation, derivative path generation, params-hash computation.
- **Unit tests**: upload deletion -- referenced (reject), orphaned (succeed), file cleanup, derivative cleanup.
- **Integration tests**: gallery pagination (page 1/2/N), search + schema filter + pagination combined.
- **Integration tests**: media picker flow -- open picker, select upload, verify hidden input is set.
- **Integration tests**: focal point -- set, get, reset, verify derivative uses correct focal point.
- **Integration tests**: image transform -- request derivative, verify cached on disk, verify ETag, verify second request serves from cache.
- **Integration tests**: transform on non-image (400), transform with dimensions > 4096 (400).
- **Integration tests**: export bundle excludes `_derived/`, import restores focal point metadata.
