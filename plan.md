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

## Completed

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

## Remaining Work

### P6: Observability

- [x] **Prometheus metrics** — `/metrics` endpoint (unauthenticated, for internal scraping)
  - Request count by method/path/status
  - Request duration histogram
  - Active connections gauge
  - Content entries gauge (per schema)
  - Upload count and total size
  - Uses `metrics` + `metrics-exporter-prometheus` crates
  - Endpoint sits outside auth middleware stack

### P7: Audit logging

- [x] **Audit log system** — separate SQLite database for action tracking
  - Separate SQLite DB file (`audit.db`), own pool and migrations
  - Actions logged: login, logout, user creation, schema create/update/delete, content create/update/delete, token create/delete, import, export
  - Each entry: timestamp, actor (user ID or "system"), action, resource type, resource ID, optional details JSON
  - No UI — queryable via SQL for now (UI can be added later)
  - Async writes (don't block request handling)

### P8: Deployment

- [x] **Dockerfile** — multi-stage build for production deployment
  - Stage 1: Rust builder (cargo build --release)
  - Stage 2: Minimal runtime image (debian-slim)
  - Copies binary + templates directory
  - Creates data volume mount points (data/, uploads/)
  - No docker-compose — self-contained app with embedded SQLite
  - Build and run commands documented in README

### P9: Documentation

- [x] **README** — comprehensive project documentation
  - Project overview and motivation
  - Feature list (all implemented features)
  - Architecture overview (data flow, storage model)
  - Getting started (build from source, Docker)
  - Configuration (CLI flags, environment)
  - API reference (endpoints, auth, examples)
  - Schema format (JSON Schema extensions, upload type)
  - Import/export workflow
  - No emojis anywhere

- [x] **Landing page** — `website/index.html` for GitHub Pages
  - Single standalone HTML file, no frameworks or build step
  - Developer-focused minimal aesthetic
  - Subtle CSS animations (no JS frameworks)
  - Explains all features with clear sections
  - Links to https://github.com/wavefunk/substrukt
  - Responsive design
  - Dark/neutral color palette

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
