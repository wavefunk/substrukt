# Substrukt — Remaining Work

## What's Built

Everything compiles and the server starts. The code covers phases 1–9 of the original plan
in skeleton form. Here's the honest status of each area:

### Fully Working
- [x] CLI with clap: `serve`, `import`, `export`, `create-token`
- [x] Config struct with data dir, DB path, port, directory creation
- [x] AppState with SQLite pool, template env, DashMap cache
- [x] SQLite database init with migrations (users, api_tokens tables)
- [x] User model with argon2 hashing/verification
- [x] API token model (CRUD, hash-based lookup)
- [x] Tracing with env filter
- [x] Graceful shutdown
- [x] Schema file store (list, get, save, delete, validate)
- [x] Content file store (directory and single-file modes, CRUD)
- [x] Content validation against JSON Schema
- [x] Form generation from JSON Schema (all types: string, number, boolean, enum, textarea, upload, object, array)
- [x] Form data parsing back to JSON
- [x] Content-addressed upload storage (SHA-256, sidecar metadata)
- [x] Cache population on startup
- [x] Export/import tar.gz bundles
- [x] All templates (base layout, login, setup, dashboard, schemas, content, settings)

### Built But Untested End-to-End
- [ ] Auth middleware (reads Session from request extensions — may not work since session layer is applied after the middleware in the layer stack)
- [ ] Login/logout flow
- [ ] First-run setup redirect
- [ ] Schema CRUD via UI
- [ ] Content CRUD via UI (multipart form handling)
- [ ] Upload widget in content forms
- [ ] API routes with bearer token auth
- [ ] Token management UI
- [ ] Import/export via API

### Not Built Yet
- [ ] File watcher (notify) for cache invalidation
- [ ] minijinja-autoreload for dev hot-reload
- [ ] htmx partial rendering (HxRequest detection)
- [ ] Content nav links in sidebar (dynamic per-schema)
- [ ] Error pages (404, 500)
- [ ] Flash messages
- [ ] CSRF protection
- [ ] Input sanitization (upload filenames, schema slugs)
- [ ] Rate limiting
- [ ] Structured logging for operations
- [ ] Tests

---

## Known Issues to Fix First

### 1. Session layer ordering (CRITICAL)
The session layer is applied *after* `build_router()` returns, via `.layer(session_layer)`.
The auth middleware uses `from_fn_with_state` *inside* the router. In axum, layers are
applied outside-in, so the session layer runs first and the auth middleware should see the
Session in extensions. This needs manual verification — start the server, visit `/`, confirm
the redirect to `/setup` works.

### 2. Schema editor uses PUT but form doesn't send PUT
`schemas/edit.html` has `<input type="hidden" name="_method" value="PUT">` but there's no
method override middleware. The form will POST, not PUT. Either:
- Add a method override middleware, or
- Change the update route to accept POST at `/{slug}/edit`

### 3. Content delete uses fetch() DELETE but route expects DELETE
The content list template calls `fetch('/content/slug/id', { method: 'DELETE' })` which
returns a redirect. fetch won't follow redirects for non-GET. Either:
- Return 204 and handle in JS, or
- Use a POST form like the token delete does

### 4. Schema delete same issue
Same as above for schema delete.

### 5. `form_fields` HTML is raw in template
`content/edit.html` renders `{{ form_fields }}` but minijinja auto-escapes by default.
Need `{{ form_fields|safe }}` or mark the value as safe in Rust.

---

## Remaining Work — Ordered by Priority

### P0: Make the core loop work (schema → content → view)

- [ ] **Fix template escaping** — form_fields must render as raw HTML
- [ ] **Fix schema update routing** — POST to `/{slug}/edit` or add method override
- [ ] **Fix delete operations** — return JSON/204 for fetch-based deletes
- [ ] **Manually test auth flow** — setup → login → session → redirect
- [ ] **Manually test schema CRUD** — create, list, edit, delete
- [ ] **Manually test content CRUD** — create entry, edit, view in list, delete
- [ ] **Fix sidebar nav** — add dynamic content type links per schema

### P1: File uploads working end-to-end

- [ ] **Test upload widget** — file input in content form, multipart handling
- [ ] **Test upload dedup** — same file uploaded twice, verify no duplicate
- [ ] **Test upload serving** — GET `/uploads/file/{hash}/{filename}` returns file
- [ ] **Handle missing upload in edit** — keep existing upload when no new file selected

### P2: API with bearer tokens

- [ ] **Test token creation** — UI and CLI
- [ ] **Test all API endpoints** — schemas, content CRUD, uploads
- [ ] **Test export/import via API** — multipart upload for import
- [ ] **Test CLI import/export** — `substrukt export/import`

### P3: Caching and file watching

- [ ] **Add file watcher** — use `notify` to watch `data/content/` and `data/schemas/`
- [ ] **Wire watcher to cache** — on file change, reload affected entries
- [ ] **Verify cache invalidation** — edit JSON on disk, see change in UI

### P4: Developer experience

- [ ] **minijinja-autoreload** — hot-reload templates in dev mode
- [ ] **htmx partial rendering** — detect `HX-Request` header, return content block only
- [ ] **Add justfile commands** — `just dev`, `just build`, `just test`

### P5: Polish

- [ ] **Error pages** — styled 404, 500
- [ ] **Flash messages** — success/error after create/update/delete actions
- [ ] **CSRF protection** — session-bound token for form submissions
- [ ] **Input sanitization** — validate schema slugs, sanitize upload filenames
- [ ] **Structured logging** — trace spans for request handling, DB queries

### P6: Tests

- [ ] **Unit tests** — schema parsing, form generation, content store, upload store
- [ ] **Integration tests** — full request/response cycle with test server
- [ ] **Export/import round-trip test**

---

## Dependency Notes

Versions that work together (discovered through trial and error):
- `tower-sessions = "0.14"` + `tower-sessions-sqlx-store = "0.15"` — both use `tower-sessions-core 0.14` which depends on `axum-core 0.5` (matching `axum 0.8`)
- `rand = "0.9"` — required for Rust 2024 edition (`gen` is a reserved keyword in 0.8's API)
- `argon2` uses `rand_core 0.6` internally — use `argon2::password_hash::rand_core::OsRng` not `rand::rngs::OsRng` to avoid version conflict

## File Map

```
src/
  main.rs              — CLI, server startup, shutdown
  config.rs            — Config struct, directory helpers
  state.rs             — AppState (pool, config, templates, cache)
  templates.rs         — minijinja Environment setup
  cache.rs             — DashMap cache: populate, reload, rebuild
  db/
    mod.rs             — SQLite pool init, run migrations
    models.rs          — User, ApiToken structs and queries
  auth/
    mod.rs             — Session helpers, require_auth middleware
    token.rs           — Bearer token generation, hashing, extractor
  schema/
    mod.rs             — Schema file CRUD, validation
    models.rs          — SubstruktMeta, StorageMode, SchemaFile
  content/
    mod.rs             — Content entry CRUD (directory + single-file modes)
    form.rs            — JSON Schema → HTML form, form data → JSON
  uploads/
    mod.rs             — Content-addressed file storage
  sync/
    mod.rs             — tar.gz export/import
  routes/
    mod.rs             — Router assembly, dashboard handler
    auth.rs            — Login, logout, setup pages
    schemas.rs         — Schema CRUD routes
    content.rs         — Content CRUD routes (multipart)
    uploads.rs         — Upload/serve routes
    settings.rs        — Token management UI
    api.rs             — REST API (/api/v1/*)
templates/
  base.html            — Layout with twind + htmx + nav
  _nav.html            — Sidebar navigation
  dashboard.html       — Schema/entry counts
  login.html           — Login form
  setup.html           — First-run admin creation
  schemas/
    list.html          — Schema table
    edit.html          — JSON editor textarea
  content/
    list.html          — Entry table with dynamic columns
    edit.html          — Generated form wrapper
  settings/
    tokens.html        — API token management
migrations/
  001_create_users.sql
  002_create_tokens.sql
```
