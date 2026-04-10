# Markdown / Rich Text Field Type — Design Spec

## Motivation

Substrukt already supports `format: "markdown"` on string fields, rendering an EasyMDE editor in the content form. However, the current implementation is minimal:

- EasyMDE is configured without preview, with a bare toolbar, and no theme integration
- The API serves raw markdown strings only — every consumer must bring its own renderer
- There is no way to get rendered HTML from the API, making the CMS impractical for blog/docs use cases where the frontend is a static site or thin client
- The editor has no dark mode support, creating a visual mismatch in dark theme

This spec enhances markdown fields into a first-class content authoring experience: a polished editor with live preview, server-side markdown-to-HTML rendering via the API, and proper dark/light mode theming.

## Goals

1. Enhance the EasyMDE editor with side-by-side preview, configurable toolbar, and dark mode support
2. Add server-side markdown rendering via `pulldown-cmark` so the API can return pre-rendered HTML wrapped in `<div class="sk-markdown">` for CSS scoping
3. Support schema-level render default (`x-substrukt.render: "html"`) with query parameter override (`?render=html` / `?render=raw`)
4. Keep raw markdown as the canonical storage format (no dual-storage)

## What Is NOT In Scope

- **Rich text / WYSIWYG editor** (`format: "richtext"`). WYSIWYG editors (Tiptap, ProseMirror, Quill) produce HTML that is harder to version-control and diff, require significant JS bundle weight, and add complexity for marginal benefit when markdown with live preview is available. Can be revisited later as a separate feature.
- **Image upload from within the editor**. Markdown image syntax (`![alt](url)`) works with existing upload URLs. Drag-and-drop image insertion into the editor textarea is a separate enhancement.
- **Custom markdown extensions** (admonitions, callouts, math blocks). The initial implementation uses standard CommonMark/GFM. Extensions can be added later via pulldown-cmark's extensibility.
- **Server-side syntax highlighting** of code blocks. Consumers can apply their own highlighting library (Prism, highlight.js) to the rendered HTML. Adding `syntect` or similar would increase binary size substantially for a feature most consumers handle client-side.

## Architecture

### Storage: Raw Markdown (Unchanged)

Markdown fields continue to store raw markdown as a JSON string value. No schema changes, no dual-storage. The rendered HTML is computed on-the-fly at API response time.

Rationale: Storing both raw and rendered creates a synchronization problem (stale HTML on schema migration, re-rendering after pulldown-cmark upgrades). Computing on read is cheap — pulldown-cmark renders typical blog posts in microseconds — and the result can be cached alongside the existing ETag mechanism.

### Server-Side Rendering: `pulldown-cmark`

Add `pulldown-cmark` as a dependency. It is the de facto standard Rust markdown parser, supports CommonMark + GFM extensions (tables, strikethrough, task lists, autolinks), compiles to native code (no WASM/FFI), and has zero unsafe code.

```toml
pulldown-cmark = { version = "0.13", default-features = false, features = ["html"] }
```

Default features are disabled to avoid pulling in `getopts` (a CLI utility dependency that is unnecessary for library usage). The `html` feature is required for the `html::push_html` renderer. The core parser handles CommonMark + GFM tables/strikethrough/tasklists which is the right default set.

#### Raw HTML Passthrough and Security

By default, pulldown-cmark passes raw HTML in markdown through to the output unchanged. This means markdown content like `Hello <script>alert('xss')</script>` will produce `<p>Hello <script>alert('xss')</script></p>`.

This is acceptable for Substrukt's trust model: only authenticated CMS users (editors and admins) can create or edit content. The content authors are trusted. This is the same trust level as any CMS that stores HTML content.

However, to protect API consumers who may render the HTML without additional sanitization, the `render_markdown` function strips raw HTML events from the pulldown-cmark event stream. This is a defense-in-depth measure: inline HTML in the CMS editor offers a poor authoring experience anyway (no preview, easy to break), and stripping it prevents accidental XSS when consumers display the rendered HTML.

Implementation: filter the parser iterator to skip `Event::Html` and `Event::InlineHtml` events before passing to `push_html`. This turns raw HTML into invisible (stripped) content rather than passing it through. If a future need arises for raw HTML passthrough, it can be added as a separate `?render=html-unsafe` option.

### HTML Output: CSS Wrapper Class

Rendered HTML is wrapped in `<div class="sk-markdown">...</div>`. This gives API consumers a predictable selector to scope styles against — the same pattern as GitHub's `markdown-body` or Tailwind's `prose`. SSG clients (e.g., eigen) can ship default styles for `.sk-markdown h1`, `.sk-markdown p`, `.sk-markdown table`, etc. and let site authors override them.

Empty markdown fields are not wrapped (empty string stays empty string).

### API: Schema-Level Render Default with Query Parameter Override

Markdown rendering behavior is controlled at two levels:

1. **Schema-level default** via `x-substrukt.render`: when set to `"html"`, the API renders markdown fields as HTML by default for all GET responses from that schema.
2. **Query parameter override**: `?render=html` forces rendering on, `?render=raw` forces rendering off, regardless of the schema default.

Resolution order: query param > schema default > raw (backwards-compatible default).

| Schema `render` | Query param | Result |
|-----------------|-------------|--------|
| (not set)       | (not set)   | raw    |
| (not set)       | `html`      | HTML   |
| (not set)       | `raw`       | raw    |
| `"html"`        | (not set)   | HTML   |
| `"html"`        | `html`      | HTML   |
| `"html"`        | `raw`       | raw    |

Example schema with render default:

```json
{
  "x-substrukt": {
    "title": "Blog Posts",
    "slug": "posts",
    "storage": "directory",
    "render": "html"
  },
  "type": "object",
  "properties": {
    "title": { "type": "string" },
    "body": { "type": "string", "format": "markdown" }
  }
}
```

With this schema, `GET /api/v1/apps/myapp/content/posts/123` returns rendered HTML in the `body` field without needing `?render=html`. The SSG consumer doesn't need to know about the param — the schema author configures it once.

Content GET endpoints:

- `GET /api/v1/apps/{app}/content/{schema}/{id}` — uses schema default (raw if unset)
- `GET /api/v1/apps/{app}/content/{schema}/{id}?render=html` — force rendered HTML
- `GET /api/v1/apps/{app}/content/{schema}/{id}?render=raw` — force raw markdown
- Same for list and single endpoints

The rendering is applied per-field based on the schema: only fields with `"type": "string", "format": "markdown"` are transformed. All other fields pass through unchanged. Nested markdown fields (inside objects or arrays) are also handled recursively.

Rationale for query parameter over a separate endpoint or header:
- Query parameter is the most discoverable approach — visible in URLs, easy to test in browser/curl
- A separate `/rendered` endpoint would duplicate routing logic
- An `Accept` header approach would conflict with the standard `application/json` content type

### Editor Enhancements

The EasyMDE configuration in `base.html` is updated to:

1. **Side-by-side preview**: Enable the split-pane editor+preview mode via toolbar button. Default to editor-only to avoid overwhelming simple use cases.
2. **Toolbar**: Configure a curated toolbar: bold, italic, strikethrough, heading, code, quote, unordered-list, ordered-list, link, image, horizontal-rule, preview, side-by-side, fullscreen, guide.
3. **Dark mode theming**: Add CSS overrides for EasyMDE's CodeMirror editor and preview pane that respond to the `.dark` class on `<html>`, matching substrukt's existing dark mode system (CSS custom properties defined in `:root` and `.dark` blocks).
4. **`forceSync: true`**: Already set. Ensures the hidden textarea stays in sync (required for form submission).

No change to the `data-markdown` attribute convention or the `initMarkdownEditors()` function signature.

## Data Models and Types

### Schema Definition (No Change)

Markdown fields in JSON Schema remain:

```json
{
  "type": "string",
  "format": "markdown",
  "title": "Body"
}
```

No new schema properties. The `format: "markdown"` is already recognized by the form renderer (`src/content/form.rs`, line ~301) and the content list column filter.

### Stored Content (No Change)

```json
{
  "title": "My Post",
  "body": "# Hello\n\nThis is **markdown** content."
}
```

### API Response Without `render=html` (No Change)

```json
{
  "title": "My Post",
  "body": "# Hello\n\nThis is **markdown** content."
}
```

### API Response With `render=html`

```json
{
  "title": "My Post",
  "body": "<h1>Hello</h1>\n<p>This is <strong>markdown</strong> content.</p>\n"
}
```

The field name stays the same. The value changes from raw markdown to rendered HTML. This is intentional — consumers requesting `render=html` want drop-in HTML, not a parallel field to pick from.

### Schema Meta Extension

Add an optional `render` field to `SubstruktMeta` in `src/schema/models.rs`:

```rust
pub struct SubstruktMeta {
    pub title: String,
    pub slug: String,
    #[serde(default = "default_storage")]
    pub storage: StorageMode,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render: Option<String>,  // new: "html" to render markdown fields by default
}
```

### Query Parameter Type

```rust
#[derive(serde::Deserialize, Default)]
pub struct ListParams {
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub render: String,  // "html" to force render, "raw" to force raw
}
```

The `render` field is added to the existing `ListParams` struct in `src/routes/api.rs` (currently at line 69-75). The type is `String` with `#[serde(default)]` for consistency with the existing `q` and `status` fields in the same struct.

Render resolution logic in API handlers:

```rust
fn should_render(params_render: &str, schema_render: Option<&str>) -> bool {
    match params_render {
        "html" => true,
        "raw" => false,
        _ => schema_render == Some("html"),
    }
}
```

Query param takes precedence. When neither is set, raw markdown is returned (backwards-compatible).

## API Surface

### New Public Function: `render_markdown`

```rust
// src/content/mod.rs

/// Render a markdown string to sanitized HTML using pulldown-cmark with GFM extensions.
/// Raw HTML in the markdown input is stripped (not passed through) as a security measure.
/// Output is wrapped in `<div class="sk-markdown">...</div>` for CSS scoping.
/// Returns empty string for empty input (no wrapper).
pub fn render_markdown(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    use pulldown_cmark::{Event, Parser, Options, html};
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(input, options);
    // Strip raw HTML events to prevent XSS in rendered output
    let parser = parser.filter(|event| !matches!(event, Event::Html(_) | Event::InlineHtml(_)));
    let mut html_output = String::from("<div class=\"sk-markdown\">");
    html::push_html(&mut html_output, parser);
    html_output.push_str("</div>");
    html_output
}
```

This function is public so it can be used from `routes/api.rs` and potentially from templates or other modules in the future.

**Note on `Event` variant names**: The exact variant names (`Event::Html`, `Event::InlineHtml`) must be verified against the pulldown-cmark 0.13 API docs at implementation time. If the variants are named differently (e.g., `Event::Html(CowStr)` vs `Event::RawHtml`), adjust accordingly. The intent is to filter out any events that represent raw HTML passthrough.

### New Public Function: `render_markdown_fields`

```rust
// src/content/mod.rs

/// Walk a JSON value and render all markdown fields to HTML, based on the schema.
/// Only transforms fields where the schema declares `"type": "string", "format": "markdown"`.
pub fn render_markdown_fields(data: &mut Value, schema: &Value) {
    render_markdown_fields_inner(data, schema, 0);
}

fn render_markdown_fields_inner(data: &mut Value, schema: &Value, depth: usize) {
    // Reuses the existing MAX_NESTING_DEPTH constant (32) defined in this module
    if depth > MAX_NESTING_DEPTH {
        return;
    }
    let Some(props) = schema.get("properties").and_then(|p| p.as_object()) else {
        return;
    };
    let Some(obj) = data.as_object_mut() else {
        return;
    };
    for (key, prop_schema) in props {
        let field_type = prop_schema.get("type").and_then(|t| t.as_str());
        let format = prop_schema.get("format").and_then(|f| f.as_str());

        match (field_type, format) {
            (Some("string"), Some("markdown")) => {
                // Clone the markdown string first to avoid borrowing obj both
                // immutably (via .get) and mutably (via .insert) simultaneously.
                if let Some(md) = obj.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()) {
                    let html = render_markdown(&md);
                    obj.insert(key.clone(), Value::String(html));
                }
            }
            (Some("object"), _) => {
                if let Some(nested) = obj.get_mut(key) {
                    render_markdown_fields_inner(nested, prop_schema, depth + 1);
                }
            }
            (Some("array"), _) => {
                if let Some(items_schema) = prop_schema.get("items") {
                    if let Some(Value::Array(arr)) = obj.get_mut(key) {
                        for item in arr.iter_mut() {
                            render_markdown_fields_inner(item, items_schema, depth + 1);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
```

**Design notes**:
- The `MAX_NESTING_DEPTH` constant (value: 32) already exists in `src/content/mod.rs` (line 295) and is reused here. No new constant needed.
- The recursive structure mirrors the pattern used by `matches_query_inner` in the same module and `resolve_references` in `routes/api.rs`, but unlike `resolve_references` (which only handles top-level properties), this function recurses into nested objects and arrays to handle markdown fields at any depth.
- The markdown string is cloned via `.as_str().map(|s| s.to_string())` before calling `render_markdown` to satisfy the borrow checker — `obj.get(key)` borrows `obj` immutably and `obj.insert()` borrows it mutably. This clone is cheap relative to the markdown rendering itself.

### Modified API Routes

In `src/routes/api.rs`, the following handlers are modified to accept and act on `render=html`:

#### `list_entries` (line 278)

Already has `Query(params): Query<ListParams>`. Add rendering in the map closure after reference resolution:

```rust
let data: Vec<serde_json::Value> = entries
    .iter()
    .map(|e| {
        let mut d = content::strip_internal_status(&e.data);
        resolve_references(&mut d, &schema_file.schema, &state.cache, &app.app.slug);
        if params.render == "html" {
            content::render_markdown_fields(&mut d, &schema_file.schema);
        }
        d
    })
    .collect();
```

The list endpoint already passes `None` as the ETag cache key, so no cache key changes needed.

#### `get_entry` (line 330)

Currently does NOT have a `Query<ListParams>` extractor. The signature must be extended:

```rust
async fn get_entry(
    State(state): State<AppState>,
    token: BearerToken,
    app: ApiAppContext,
    Path((_app_slug, schema_slug, entry_id)): Path<(String, String, String)>,
    Query(params): Query<ListParams>,  // NEW
    headers: HeaderMap,
) -> impl IntoResponse {
```

Then after reference resolution:

```rust
Ok(Some(entry)) => {
    let mut data = content::strip_internal_status(&entry.data);
    resolve_references(&mut data, &schema_file.schema, &state.cache, &app.app.slug);
    if params.render == "html" {
        content::render_markdown_fields(&mut data, &schema_file.schema);
    }
    // When rendering HTML, bypass ETag cache to avoid serving a cached
    // raw-markdown ETag for a rendered response or vice versa.
    let cache_key = if params.render == "html" {
        None
    } else {
        Some(format!("{}/{}/{}", app.app.slug, schema_slug, entry_id))
    };
    json_with_etag(&data, &headers, &state.etag_cache, cache_key.as_deref())
}
```

**Important**: The `Query(params)` extractor must be placed **before** `headers: HeaderMap` in the function signature. Axum extracts parameters in order, and `Query` must come before catch-all extractors. The existing `get_entry` signature has `Path` then `headers` — inserting `Query` between them is correct.

#### `get_single` (line 550)

Already has `Query(params): Query<ListParams>`. Add rendering after reference resolution:

```rust
let mut data = content::strip_internal_status(&entry.data);
resolve_references(&mut data, &schema_file.schema, &state.cache, &app.app.slug);
if params.render == "html" {
    content::render_markdown_fields(&mut data, &schema_file.schema);
}
Json(data).into_response()
```

**Note**: `get_single` currently does NOT use `json_with_etag` — it returns `Json(data).into_response()` directly (no ETag caching). This means there is no ETag cache key conflict to worry about for this handler. Adding ETag support to `get_single` is out of scope for this feature.

No new routes. No changes to write endpoints (create, update, upsert). The rendering is read-side only.

### No Changes to Content Routes (UI)

The web UI (`src/routes/content.rs`) does not use rendered HTML. The form always works with raw markdown, and EasyMDE handles client-side preview. No changes needed.

### OpenAPI Spec Update

The OpenAPI generator in `src/openapi.rs` documents the `render` query parameter on content GET endpoints. The parameter JSON object is added to the `parameters` array in three places:

1. **`add_collection_content_routes`** — the list endpoint (alongside existing `q` and `status` parameters, around line 631)
2. **`add_collection_content_routes`** — the get-by-id endpoint (alongside existing `entry_id` path parameter, around line 710)
3. **`add_single_content_routes`** — the get-single endpoint (alongside existing `status` parameter, around line 553)

The parameter definition:

```json
{
  "name": "render",
  "in": "query",
  "required": false,
  "schema": { "type": "string", "enum": ["html", "raw"] },
  "description": "Override markdown rendering: 'html' to render as HTML (wrapped in <div class=\"sk-markdown\">), 'raw' to return raw markdown. When omitted, uses the schema's x-substrukt.render default (raw if unset)."
}
```

## Error Handling

### Malformed Markdown

`pulldown-cmark` does not produce errors — it always produces output, even for malformed input. Worst case, the output is a literal rendering of the input text wrapped in `<p>` tags. No error handling needed for the rendering step.

### Missing Schema Information

If the schema cannot be loaded when serving a `render=html` request, the existing error handling in the API routes already returns 404 or 500. No additional error paths.

### Invalid `render` Parameter Value

Non-`"html"`/`"raw"` values for `render` are silently ignored (falls through to schema default, then raw). This is the principle of least surprise — unknown parameter values don't break anything.

### Deserialization Failure

The `ListParams` struct uses `#[serde(default)]` on all fields. If `render` appears with a non-string value (e.g., `?render[]=html`), serde will fail to deserialize and Axum will return a 400 Bad Request. This is standard Axum behavior for `Query` extractors and requires no custom handling.

## Edge Cases and Failure Modes

### Empty Markdown String

`render_markdown("")` returns `""`. An empty field stays empty. Verified experimentally with pulldown-cmark 0.13.

### Markdown Field with Null Value

If a markdown field is `null` in the stored JSON (e.g., optional field not filled in), `render_markdown_fields` skips it because `obj.get(key).and_then(|v| v.as_str())` returns `None` for null values. The value remains `null` in the response.

### Markdown Fields in Nested Objects/Arrays

Handled recursively by `render_markdown_fields_inner` with depth limiting (MAX_NESTING_DEPTH = 32). A markdown field inside an array of objects (e.g., a "sections" array where each section has a "body" markdown field) is correctly rendered. If nesting exceeds 32 levels, deeper fields are silently left as raw markdown.

### Very Large Markdown Content

pulldown-cmark is a streaming parser — it processes input incrementally without building a full AST in memory. A 1MB markdown document renders in ~10ms on modest hardware. No special handling needed.

### Upload References in Markdown

Users may write markdown like `![photo](/apps/myapp/uploads/file/abc123/photo.png)`. The rendering does not rewrite URLs — they pass through as-is. This is correct because the URLs are already absolute paths that work in the served context.

### Concurrent `render=html` Requests

No shared mutable state. Each request creates its own `pulldown_cmark::Parser` and output buffer. Thread-safe by construction.

### ETag Cache and Rendered Responses

Only `get_entry` uses ETag caching with a per-entry cache key (`"{app}/{schema}/{id}"`). Rendered responses (`render=html`) must not share this cache entry with raw responses. When `render=html` is requested, pass `None` as the cache key to `json_with_etag` so the ETag is computed fresh from the response body. This is the simplest correct approach — rendered responses are not expected to be the high-frequency path, and computing SHA-256 of the response body is fast.

`list_entries` already passes `None` for the cache key (no per-list caching), so no change needed there. `get_single` does not use `json_with_etag` at all.

### Raw HTML in Markdown Input

Markdown content may contain raw HTML tags (e.g., `<div>`, `<script>`, `<iframe>`). The `render_markdown` function strips these by filtering out `Event::Html` and `Event::InlineHtml` events from the parser stream. This means:
- `Hello <b>bold</b> world` renders as `<p>Hello bold world</p>` (the `<b>` tags are stripped)
- `<script>alert('xss')</script>` is stripped entirely
- Standard markdown syntax is unaffected — `**bold**` still produces `<strong>bold</strong>`

This is a deliberate trade-off: raw HTML in markdown is a niche feature with poor editor UX, and stripping it prevents XSS for API consumers who render the HTML output without additional sanitization.

## Files Changed

### New dependency

- `Cargo.toml` — add `pulldown-cmark = { version = "0.13", default-features = false, features = ["html"] }`

### Rust source

- `src/schema/models.rs` — add optional `render` field to `SubstruktMeta`
- `src/content/mod.rs` — add `render_markdown()` (with `sk-markdown` wrapper) and `render_markdown_fields()` functions with unit tests
- `src/routes/api.rs` — add `render` field to `ListParams`, add `should_render()` helper, add `Query(params)` extractor to `get_entry`, apply rendering in `get_entry`, `list_entries`, `get_single` using schema default + query override
- `src/openapi.rs` — add `render` query parameter to content GET endpoint specs in `add_collection_content_routes` and `add_single_content_routes`

### Frontend

- `templates/base.html` — update EasyMDE configuration (toolbar, preview options) and add dark mode CSS overrides for the editor

### No changes to

- `src/content/form.rs` — the form renderer already handles `format: "markdown"` correctly
- `src/routes/content.rs` — the UI routes do not serve rendered HTML
- Content storage format — raw markdown remains the canonical format

## Testing

### Unit Tests (in `src/content/mod.rs`)

- `render_markdown` with basic markdown (headings, bold, links) produces correct HTML wrapped in `<div class="sk-markdown">`
- `render_markdown` with empty string returns empty string (no wrapper)
- `render_markdown` with GFM table produces `<table>` HTML
- `render_markdown` with GFM strikethrough produces `<del>` HTML
- `render_markdown` with GFM tasklist produces checkbox HTML
- `render_markdown` strips raw HTML tags (verify `<script>` is removed, `<b>` is removed)
- `render_markdown_fields` transforms only markdown fields, leaves other fields (string, number, boolean) untouched
- `render_markdown_fields` handles nested objects (markdown field inside an object field)
- `render_markdown_fields` handles arrays of objects with markdown fields
- `render_markdown_fields` skips null markdown values (null stays null)
- `render_markdown_fields` skips non-markdown string fields (e.g., `format: "textarea"` or no format)
- `render_markdown_fields` respects depth limit (does not stack overflow on deeply nested schemas)

### Integration Tests (in `tests/integration.rs`)

- API GET with `?render=html` returns HTML-rendered markdown fields with `sk-markdown` wrapper
- API GET without `render` parameter returns raw markdown (backwards compat)
- API GET with `?render=html` on a schema with no markdown fields returns data unchanged
- API list endpoint with `?render=html` renders all entries in the array
- API GET single with `?render=html` renders markdown fields
- Schema with `x-substrukt.render: "html"` renders by default (no query param needed)
- Schema with `x-substrukt.render: "html"` + `?render=raw` returns raw markdown
- Schema without `render` default + no query param returns raw (backwards compat)

## Implementation Order

1. **Add `pulldown-cmark` dependency** to `Cargo.toml` and verify it compiles
2. **Implement `render_markdown`** in `src/content/mod.rs` with `sk-markdown` wrapper and unit tests
3. **Implement `render_markdown_fields`** in `src/content/mod.rs` with unit tests
4. **Add `render` field to `SubstruktMeta`** in `src/schema/models.rs`
5. **Add `render` field to `ListParams`** and `should_render()` helper in `src/routes/api.rs`
6. **Wire up rendering in `list_entries`**, `get_entry` (including adding `Query(params)` extractor), and `get_single` using `should_render()`
7. **Update OpenAPI spec** in `src/openapi.rs`
8. **Update EasyMDE configuration** in `templates/base.html` (toolbar, sideBySideFullscreen, previewClass)
9. **Add dark mode CSS** for EasyMDE in `templates/base.html`
10. **Add integration tests**

Steps 1-3 can be one commit (backend rendering logic). Step 4 is one commit (schema model). Steps 5-7 can be one commit (API wiring). Steps 8-9 can be one commit (editor enhancements). Step 10 is a final commit.

## EasyMDE Dark Mode CSS

The following CSS block is added to `base.html` inside a `<style>` tag, after the existing CSS variables block (after the `.dark { ... }` block that ends around line 50). It targets the `.dark` class on the root element to override EasyMDE/CodeMirror defaults:

```css
.dark .EasyMDEContainer .CodeMirror {
  background: var(--input-bg);
  color: var(--text-primary);
  border-color: var(--border);
}
.dark .EasyMDEContainer .CodeMirror-cursor {
  border-left-color: var(--text-primary);
}
.dark .EasyMDEContainer .editor-toolbar {
  background: var(--card);
  border-color: var(--border);
}
.dark .EasyMDEContainer .editor-toolbar button {
  color: var(--text-secondary) !important;
}
.dark .EasyMDEContainer .editor-toolbar button:hover,
.dark .EasyMDEContainer .editor-toolbar button.active {
  background: var(--card-alt);
  color: var(--text-primary) !important;
}
.dark .EasyMDEContainer .editor-preview,
.dark .EasyMDEContainer .editor-preview-side {
  background: var(--card);
  color: var(--text-primary);
}
.dark .EasyMDEContainer .editor-preview pre,
.dark .EasyMDEContainer .editor-preview-side pre {
  background: var(--card-alt);
}
.dark .EasyMDEContainer .CodeMirror-selected {
  background: var(--accent-soft) !important;
}
.dark .EasyMDEContainer .editor-toolbar.disabled-for-preview button:not(.no-disable) {
  opacity: 0.4;
}
```

All CSS custom properties referenced above (`--input-bg`, `--text-primary`, `--border`, `--card`, `--card-alt`, `--text-secondary`, `--accent-soft`) are already defined in both the `:root` (light) and `.dark` (dark) blocks in `base.html`.

## EasyMDE Configuration Update

Replace the current `initMarkdownEditors` function body (currently at line 130 of `templates/base.html`):

```javascript
function initMarkdownEditors() {
  document.querySelectorAll('[data-markdown]:not(.easymde-attached)').forEach(function(el) {
    el.classList.add('easymde-attached');
    new EasyMDE({
      element: el,
      spellChecker: false,
      status: false,
      forceSync: true,
      toolbar: [
        'bold', 'italic', 'strikethrough', '|',
        'heading-1', 'heading-2', 'heading-3', '|',
        'code', 'quote', 'unordered-list', 'ordered-list', '|',
        'link', 'image', 'horizontal-rule', '|',
        'preview', 'side-by-side', 'fullscreen', '|',
        'guide'
      ],
      previewClass: ['editor-preview', 'markdown-body'],
      sideBySideFullscreen: false
    });
  });
}
```

Key additions:
- `toolbar`: explicit toolbar with logical groupings (replaces EasyMDE's default toolbar which includes some items not relevant for a CMS context)
- `sideBySideFullscreen: false`: allows side-by-side without going fullscreen (stays inline in the form)
- `previewClass`: adds `markdown-body` class for consistent preview styling

Existing call sites (`initMarkdownEditors()` is called on page load at line 250, after htmx swaps at line 255, and after array item addition at line 127) remain unchanged — no signature or behavioral contract changes.
