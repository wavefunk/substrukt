# Substrukt

A schema-driven CMS built in Rust. Define content types with JSON Schema, edit data through a web UI, store it as JSON files on disk, and serve it via a REST API.

## Features

- JSON Schema-driven content types with automatic form generation
- Content stored as JSON files on disk with in-memory caching
- Content-addressed file uploads with SHA-256 deduplication
- REST API with bearer token authentication
- Export/import bundles for syncing content between environments
- Server-rendered UI with htmx for interactivity
- SQLite for users, sessions, and API tokens
- Prometheus metrics endpoint
- Audit logging to a separate SQLite database
- File watcher for automatic cache invalidation
- CSRF protection, rate limiting, and input sanitization

## Architecture

```
JSON Schema --> UI form generation --> JSON file on disk --> served via API
```

- **Schemas** define content types. Each schema has a slug, title, storage mode (directory or single-file), and JSON Schema properties. The custom `format: "upload"` extension handles file uploads.
- **Content** is stored as JSON files under `data/content/<schema-slug>/`. Entries are cached in memory on startup and kept in sync via a file watcher.
- **Uploads** use content-addressed storage: files are hashed with SHA-256 and stored at `data/uploads/<prefix>/<hash>` with a `.meta.json` sidecar.
- **SQLite** handles infrastructure: users, sessions, API tokens. Content is never stored in the database.
- **Audit log** writes to a separate `audit.db` asynchronously, tracking all create/update/delete operations.

## Getting started

### Build from source

Requires Rust nightly (2026-01-05 or later).

```sh
git clone https://github.com/wavefunk/substrukt.git
cd substrukt
cargo build --release
./target/release/substrukt serve
```

Open `http://localhost:3000`. On first visit you will be prompted to create an admin account.

### Docker

```sh
docker build -t substrukt .
docker run -p 3000:3000 -v substrukt-data:/data substrukt
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

### Commands

```
substrukt serve              # Start the web server (default)
substrukt import <path>      # Import a bundle tar.gz
substrukt export <path>      # Export a bundle tar.gz
substrukt create-token <name> # Create an API token from the command line
```

## API reference

All API endpoints require a bearer token in the `Authorization` header. Tokens are created through the Settings > API Tokens page or via `substrukt create-token`.

```
Authorization: Bearer <token>
```

### Schemas

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/schemas` | List all schemas |
| GET | `/api/v1/schemas/:slug` | Get a single schema |

### Content

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/content/:schema` | List entries for a schema |
| POST | `/api/v1/content/:schema` | Create an entry (JSON body) |
| GET | `/api/v1/content/:schema/:id` | Get a single entry |
| PUT | `/api/v1/content/:schema/:id` | Update an entry (JSON body) |
| DELETE | `/api/v1/content/:schema/:id` | Delete an entry |

### Uploads

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/v1/uploads` | Upload a file (multipart) |
| GET | `/api/v1/uploads/:hash` | Download a file by hash |

### Sync

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/v1/export` | Export all data as tar.gz |
| POST | `/api/v1/import` | Import a tar.gz bundle (multipart) |

### Metrics

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/metrics` | Prometheus metrics (unauthenticated) |

## Schema format

Schemas are standard JSON Schema with an `x-substrukt` extension:

```json
{
  "x-substrukt": {
    "title": "Blog Posts",
    "slug": "blog-posts",
    "storage": "directory"
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

Export creates a `tar.gz` bundle containing all schemas, content, and uploads. Import unpacks the bundle into the data directory and rebuilds the cache.

```sh
# CLI
substrukt export backup.tar.gz
substrukt import backup.tar.gz

# API
curl -X POST -H "Authorization: Bearer $TOKEN" http://localhost:3000/api/v1/export -o backup.tar.gz
curl -X POST -H "Authorization: Bearer $TOKEN" -F "bundle=@backup.tar.gz" http://localhost:3000/api/v1/import
```

This enables a workflow where content is edited locally and pushed to a production instance via CI.

## Data directory layout

```
data/
  substrukt.db       # Users, sessions, API tokens
  audit.db           # Audit log
  schemas/           # JSON Schema files (<slug>.json)
  content/           # Content entries
    <slug>/          # Per-schema directories (directory mode)
      <id>.json      # Individual entries
    <slug>.json      # Single-file mode entries
  uploads/           # Content-addressed files
    <prefix>/        # First 2 hex chars of SHA-256
      <rest>         # Remaining hash chars (file data)
      <rest>.meta.json  # Upload metadata (filename, MIME type, size)
```

## License

MIT
