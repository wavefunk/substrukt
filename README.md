<p align="center">
  <img src="website/static/images/roundedicon.svg" alt="Substrukt" width="80" height="80">
</p>

<h1 align="center">Substrukt</h1>

<p align="center">A schema-driven CMS built in Rust. Define content types with JSON Schema, edit data through a web UI, store it as JSON files on disk, and serve it via a REST API.</p>

<p align="center">
  <a href="https://substrukt.wavefunk.io">Documentation</a> · <a href="https://github.com/wavefunk/substrukt">GitHub</a>
</p>

## Features

- JSON Schema-driven content types with automatic form generation
- Multi-app support: manage multiple independent content spaces from one instance
- Single and collection schema kinds for settings pages and repeatable entries
- Content stored as JSON files on disk with in-memory caching
- Per-entry draft/published status with publish and unpublish actions
- Version history with configurable retention and one-click revert
- Content-addressed file uploads with SHA-256 deduplication and uploads browser
- REST API with bearer token authentication and role-based access control (admin, editor, viewer)
- Configurable deployment targets with webhooks and optional auto-deploy
- S3-compatible backups with scheduled frequency and retention policies
- Export/import bundles for syncing content between environments
- Server-rendered UI with htmx, dark mode, and theme toggle
- Interactive JSON Schema editor (vanilla-jsoneditor)
- OpenAPI specification auto-generated from schemas
- SQLite for users, sessions, API tokens, and upload metadata
- Prometheus metrics and health check endpoints
- Audit logging to a separate SQLite database with built-in log viewer
- File watcher for automatic cache invalidation
- CSRF protection, rate limiting, input sanitization, and security headers

## Architecture

```
JSON Schema --> UI form generation --> JSON file on disk --> served via API
```

- **Apps** provide isolated content spaces within a single instance. Each app has its own schemas, content, uploads, and deployment targets.
- **Schemas** define content types. Each schema has a slug, title, storage mode (directory or single-file), and JSON Schema properties. The custom `format: "upload"` extension handles file uploads.
- **Content** is stored as JSON files under `data/<app-slug>/content/<schema-slug>/`. Entries are cached in memory on startup and kept in sync via a file watcher. Each entry has a draft/published status.
- **Uploads** use content-addressed storage: files are hashed with SHA-256 and stored at `data/<app-slug>/uploads/<prefix>/<hash>`. Upload metadata is tracked in SQLite.
- **SQLite** handles infrastructure: users, sessions, API tokens, upload metadata, and app configuration. Content is never stored in the database.
- **Deployments** are admin-managed webhook targets per app. Each deployment can auto-deploy on content changes with a configurable debounce, or be triggered manually.
- **Audit log** writes to a separate `audit.db` asynchronously, tracking all create/update/delete operations.

## Getting started

### Build from source

Requires Rust nightly (2026-01-05 or later).

```sh
git clone https://github.com/wavefunk/substrukt.git
cd substrukt
cargo build --release
./target/release/substrukt create-admin --email admin@example.com --username admin --password 'change-me-now'
./target/release/substrukt serve
```

Open `http://localhost:3000` and sign in with the admin account you created.

### Docker

```sh
docker pull ghcr.io/wavefunk/substrukt
docker run --rm -v substrukt-data:/data ghcr.io/wavefunk/substrukt create-admin --email admin@example.com --username admin --password 'change-me-now'
docker run -p 3000:3000 -v substrukt-data:/data ghcr.io/wavefunk/substrukt
```

Data persists in the `/data` volume (schemas, content, uploads, databases).

## Configuration

All options are passed as CLI flags:

| Flag | Default | Description |
|------|---------|-------------|
| `--data-dir <PATH>` | `data` | Root directory for schemas, content, and uploads |
| `--db-path <PATH>` | `<data-dir>/substrukt.db` | SQLite database file |
| `-p, --port <PORT>` | `3000` | HTTP listen port |
| `--secure-cookies` | off | Set `Secure` flag on session cookies (enable for HTTPS) |
| `--api-rate-limit <N>` | `100` | Max API requests per IP per minute |
| `--version-history-count <N>` | `10` | Max content versions to keep per entry |
| `--max-body-size <MB>` | `50` | Maximum request body size in megabytes |
| `--enable-registrations` | off | Enable public browser registration; invite links still work when disabled |
| `--trust-proxy-headers` | off | Trust `X-Forwarded-For` for rate limiting (enable behind a reverse proxy) |

Deployment webhooks are configured through the web UI per app (Settings > Deployments), not via CLI flags.

S3 backup credentials are configured via environment variables (`S3_BUCKET`, `S3_REGION`, `S3_ENDPOINT`, `S3_ACCESS_KEY`, `S3_SECRET_KEY`). Backup frequency and retention are managed through the web UI (Settings > Backups).

### Commands

```
substrukt serve                        # Start the web server (default)
substrukt import <path> --app <slug>   # Import a bundle tar.gz into an app
substrukt export <path> --app <slug>   # Export an app's data as bundle tar.gz
substrukt create-token <name> --app <slug>  # Create an API token for an app
substrukt create-admin --email <email> --username <name> --password <password>
substrukt prime                        # Output AI-optimized workflow context
substrukt onboard                      # Output a minimal snippet for AGENTS.md / CLAUDE.md
```

## API reference

All app-scoped API endpoints require a bearer token in the `Authorization` header. Tokens are created through the app's Settings > API Tokens page or via `substrukt create-token --app <slug>`.

```
Authorization: Bearer <token>
```

App-scoped endpoints are prefixed with `/api/v1/apps/:app_slug`.

### Schemas

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/apps/:app/schemas` | List all schemas |
| GET | `/api/v1/apps/:app/schemas/:slug` | Get a single schema |

### Content

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/apps/:app/content/:schema` | List entries (published by default, `?status=all` for all) |
| POST | `/api/v1/apps/:app/content/:schema` | Create an entry (JSON body) |
| GET | `/api/v1/apps/:app/content/:schema/:id` | Get a single entry |
| PUT | `/api/v1/apps/:app/content/:schema/:id` | Update an entry (JSON body) |
| DELETE | `/api/v1/apps/:app/content/:schema/:id` | Delete an entry |
| POST | `/api/v1/apps/:app/content/:schema/:id/publish` | Publish an entry |
| POST | `/api/v1/apps/:app/content/:schema/:id/unpublish` | Unpublish an entry (set to draft) |
| GET | `/api/v1/apps/:app/content/:schema/:id/versions` | List version history |
| GET | `/api/v1/apps/:app/content/:schema/:id/versions/:ts` | Get a specific version |
| POST | `/api/v1/apps/:app/content/:schema/:id/versions/:ts/revert` | Revert to a version |
| GET | `/api/v1/apps/:app/content/:schema/single` | Get a single-kind schema's entry |
| PUT | `/api/v1/apps/:app/content/:schema/single` | Upsert a single-kind schema's entry |
| DELETE | `/api/v1/apps/:app/content/:schema/single` | Delete a single-kind schema's entry |

### Bulk operations

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/v1/apps/:app/content/:schema/_bulk/create` | Create multiple entries |
| POST | `/api/v1/apps/:app/content/:schema/_bulk/update` | Update multiple entries |
| POST | `/api/v1/apps/:app/content/:schema/_bulk/delete` | Delete multiple entries |
| POST | `/api/v1/apps/:app/content/:schema/_bulk/publish` | Publish multiple entries |
| POST | `/api/v1/apps/:app/content/:schema/_bulk/unpublish` | Unpublish multiple entries |

### Uploads

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/v1/apps/:app/uploads` | Upload a file (multipart) |
| GET | `/api/v1/apps/:app/uploads/:hash` | Download a file by hash |
| GET | `/api/v1/apps/:app/uploads/:hash/:filename` | Download with original filename |

### Sync

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/v1/apps/:app/export` | Export app data as tar.gz |
| POST | `/api/v1/apps/:app/import` | Import a tar.gz bundle (multipart) |

### Deployments

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/apps/:app/deployments` | List deployment targets |
| POST | `/api/v1/apps/:app/deployments/:slug/fire` | Trigger a deployment webhook |

### Global endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/openapi.json` | Auto-generated OpenAPI specification |
| GET | `/api/v1/backups/status` | S3 backup status (admin only) |
| POST | `/api/v1/backups/trigger` | Trigger a manual backup (admin only) |
| GET | `/metrics` | Prometheus metrics (unauthenticated) |
| GET | `/healthz` | Health check |

## Schema format

Schemas are standard JSON Schema with an `x-substrukt` extension:

```json
{
  "x-substrukt": {
    "title": "Blog Posts",
    "slug": "blog-posts",
    "storage": "directory",
    "kind": "collection"
  },
  "type": "object",
  "properties": {
    "title": { "type": "string", "title": "Title" },
    "body": { "type": "string", "format": "textarea" },
    "published": { "type": "boolean", "title": "Published" },
    "cover": { "type": "string", "format": "upload", "title": "Cover Image" }
  },
  "required": ["title"]
}
```

### Storage modes

- `directory` -- one JSON file per entry in `data/content/<slug>/<id>.json`
- `single-file` -- all entries in `data/content/<slug>.json` as a JSON array

### Schema kinds

- `collection` (default) -- multiple entries, each with its own ID
- `single` -- exactly one entry per schema, ideal for site settings or configuration

### Supported field types

| JSON Schema type | Format | UI element |
|------------------|--------|------------|
| `string` | (none) | Text input |
| `string` | `textarea` | Textarea |
| `string` | `upload` | File input |
| `string` | `enum` | Select dropdown |
| `number` / `integer` | | Number input |
| `boolean` | | Checkbox |
| `object` | | Nested fieldset |
| `array` | | Repeatable fields |

## Import/export

Export creates a `tar.gz` bundle containing all schemas, content, and uploads for an app. Import unpacks the bundle into the app's data directory and rebuilds the cache.

```sh
# CLI
substrukt export backup.tar.gz --app my-app
substrukt import backup.tar.gz --app my-app

# API
curl -X POST -H "Authorization: Bearer $TOKEN" http://localhost:3000/api/v1/apps/my-app/export -o backup.tar.gz
curl -X POST -H "Authorization: Bearer $TOKEN" -F "bundle=@backup.tar.gz" http://localhost:3000/api/v1/apps/my-app/import
```

This enables a workflow where content is edited locally and pushed to a production instance via CI.

## Data directory layout

```
data/
  substrukt.db           # Users, sessions, API tokens, apps
  audit.db               # Audit log, deployments, backup config
  <app-slug>/            # Per-app data directory
    schemas/             # JSON Schema files (<slug>.json)
    content/             # Content entries
      <slug>/            # Per-schema directories (directory mode)
        <id>.json        # Individual entries
      <slug>.json        # Single-file mode entries
    uploads/             # Content-addressed files
      <prefix>/          # First 2 hex chars of SHA-256
        <rest>           # Remaining hash chars (file data)
    _history/            # Version history snapshots
```

## License

MIT
