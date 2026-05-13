
## Prometheus metrics

Substrukt exposes a `/metrics` endpoint in Prometheus text format. This endpoint is unauthenticated (intended for internal scraping by your monitoring stack).

### Available metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `http_requests_total` | Counter | `method`, `path`, `status` | Total HTTP requests |
| `http_request_duration_seconds` | Histogram | `method`, `path` | Request latency |
| `http_connections_active` | Gauge | -- | Currently active connections |
| `content_entries_total` | Gauge | `schema` | Number of entries per schema |
| `uploads_total` | Gauge | -- | Total uploaded files |
| `uploads_size_bytes` | Gauge | -- | Total size of all uploads |

Content and upload gauges are updated on each scrape (when `/metrics` is requested).

### Scraping

Add Substrukt to your Prometheus configuration:

```yaml
scrape_configs:
  - job_name: 'substrukt'
    static_configs:
      - targets: ['localhost:3000']
    metrics_path: '/metrics'
    scrape_interval: 30s
```

### Example output

```
# HELP http_requests_total Total HTTP requests
http_requests_total{method="GET",path="/",status="200"} 42
http_requests_total{method="POST",path="/content/{schema_slug}/{entry_id}",status="302"} 7

# HELP http_request_duration_seconds Request duration in seconds
http_request_duration_seconds_bucket{method="GET",path="/",le="0.01"} 40

# HELP content_entries_total Content entries per schema
content_entries_total{schema="blog-posts"} 15
content_entries_total{schema="faq"} 8

# HELP uploads_total Total uploaded files
uploads_total 23

# HELP uploads_size_bytes Total upload storage in bytes
uploads_size_bytes 15728640
```

## Audit logging

Substrukt maintains an audit log in a separate SQLite database (`audit.db` in the data directory). Audit writes are asynchronous -- they do not block request handling.

### Logged actions

| Action | Resource type | When |
|--------|--------------|------|
| `login` | session | User logs in |
| `logout` | session | User logs out |
| `user_create` | user | User account created through registration or invitation |
| `schema_create` | schema | Schema created via UI |
| `schema_update` | schema | Schema updated via UI |
| `schema_delete` | schema | Schema deleted via UI |
| `content_create` | content | Entry created via UI or API |
| `content_update` | content | Entry updated via UI or API |
| `content_delete` | content | Entry deleted via UI or API |
| `token_create` | api_token | API token created |
| `token_delete` | api_token | API token deleted |
| `import` | bundle | Content bundle imported |
| `export` | bundle | Content bundle exported |
| `webhook_fire` | webhook | Webhook fired (with success/failure status) |

### Audit log schema

Each audit entry contains:

| Column | Description |
|--------|-------------|
| `timestamp` | ISO 8601 timestamp |
| `actor` | User ID, `"api"`, or `"system"` |
| `action` | Action name (see table above) |
| `resource_type` | Type of resource affected |
| `resource_id` | Identifier of the affected resource |
| `details` | Optional JSON string with additional context |

### Viewing the audit log

The audit log is viewable in the web UI at **Settings > Audit Log**. You can also query it directly with SQLite:

```sh
sqlite3 data/audit.db "SELECT timestamp, actor, action, resource_type, resource_id FROM audit_log ORDER BY timestamp DESC LIMIT 20"
```

## Structured logging

HTTP request/response tracing is provided by `tower-http::TraceLayer`. Each request is logged with method, path, status code, and duration.

Control log verbosity with the `RUST_LOG` environment variable:

```sh
RUST_LOG=substrukt=debug,tower_http=debug ./substrukt serve
```
