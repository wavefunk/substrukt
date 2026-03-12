# Substrukt — Status & Remaining Work

## What's Working (Tested End-to-End)

All core functionality is working and verified with 17 integration tests:

- [x] CLI with clap: `serve`, `import`, `export`, `create-token`
- [x] Config, AppState, SQLite pool, template env, content cache
- [x] Auth: first-run setup redirect, login/logout, session management
- [x] Session layer ordering verified (session layer outer, auth middleware inner)
- [x] Schema CRUD via UI (create, list, edit via POST, delete via DELETE → 204)
- [x] Content CRUD via UI (create, list, edit, delete with multipart forms)
- [x] Form generation from JSON Schema (string, number, boolean, enum, textarea, upload, object, array)
- [x] Form data parsing back to JSON
- [x] Content-addressed upload storage (SHA-256, dedup verified)
- [x] Upload serving with correct MIME types
- [x] Upload preservation on edit (existing upload kept when no new file selected)
- [x] Content validation against JSON Schema (with upload field patching)
- [x] Dynamic sidebar nav (content type links populated from schemas)
- [x] API: all endpoints with bearer token auth (schemas, content CRUD, uploads)
- [x] API: export/import round-trip verified
- [x] Token management UI (create, list, delete)
- [x] Flash messages (success feedback on create/update actions)
- [x] Error pages (styled 404 fallback)
- [x] Cache population on startup
- [x] Export/import tar.gz bundles
- [x] Graceful shutdown
- [x] File watcher — `notify` watches content/schema dirs with debounced cache invalidation
- [x] Template autoreload — `minijinja-autoreload` for hot-reload during development

## Remaining Work

All features complete.

### P4: Developer experience (nice-to-have)

- [x] **htmx partial rendering** — detect `HX-Request` header, return content block only

### P5: Security hardening

- [x] **CSRF protection** — session-bound token for form submissions
- [x] **Input sanitization** — validate schema slugs, sanitize upload filenames
- [x] **Secure flag for sessions** — configurable via `--secure-cookies` flag

### P5: More polish

- [x] **500 error page** — CatchPanic layer returns styled error
- [x] **Structured logging** — tower-http TraceLayer for request/response tracing
- [x] **Rate limiting** — per-IP sliding window for login (10/min) and API (100/min)

## File Map

```
src/
  lib.rs               — Public module exports
  main.rs              — CLI, server startup, shutdown
  config.rs            — Config struct, directory helpers
  state.rs             — AppState (pool, config, templates, cache, rate limiters)
  templates.rs         — minijinja AutoReloader setup with nav function + htmx helpers
  cache.rs             — DashMap cache: populate, reload, rebuild + file watcher
  rate_limit.rs        — Per-IP sliding window rate limiter
  db/
    mod.rs             — SQLite pool init, run migrations
    models.rs          — User, ApiToken structs and queries
  auth/
    mod.rs             — Session helpers, flash messages, CSRF, require_auth middleware
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
    mod.rs             — Router assembly, dashboard, 404/500 fallback
    auth.rs            — Login, logout, setup pages
    schemas.rs         — Schema CRUD routes
    content.rs         — Content CRUD routes (multipart)
    uploads.rs         — Upload/serve routes
    settings.rs        — Token management UI
    api.rs             — REST API (/api/v1/*)
tests/
  integration.rs       — 17 integration tests (auth, CRUD, uploads, API, export/import)
templates/
  base.html            — Layout with twind + htmx + nav + flash messages + CSRF
  _partial.html        — Partial layout for htmx responses (content only)
  _nav.html            — Sidebar navigation (dynamic content links)
  error.html           — Error page (404, 500)
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
```
