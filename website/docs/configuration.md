
Server settings are passed as CLI flags. Deployment webhooks and S3 backups are configured through the web UI.

## CLI flags

| Flag | Default | Description |
|------|---------|-------------|
| `--data-dir <PATH>` | `data` | Root directory for schemas, content, uploads, and databases |
| `--db-path <PATH>` | `<data-dir>/substrukt.db` | Path to the main SQLite database |
| `-p, --port <PORT>` | `3000` | HTTP listen port |
| `--secure-cookies` | off | Set the `Secure` flag on session cookies (required for HTTPS) |
| `--api-rate-limit <N>` | `100` | Max API requests per IP per minute |
| `--version-history-count <N>` | `10` | Max content versions to keep per entry |
| `--max-body-size <MB>` | `50` | Maximum request body size in megabytes |
| `--trust-proxy-headers` | off | Trust `X-Forwarded-For` headers for rate limiting (enable only behind a trusted reverse proxy) |

## Environment variables

S3 backup credentials are configured via environment variables:

| Variable | Description |
|----------|-------------|
| `S3_BUCKET` | S3 bucket name |
| `S3_REGION` | AWS region (or custom region name) |
| `S3_ENDPOINT` | Custom S3-compatible endpoint URL (for Minio, R2, B2, etc.) |
| `S3_ACCESS_KEY` | Access key ID |
| `S3_SECRET_KEY` | Secret access key |

Backup frequency and retention are managed through the web UI at Settings > Backups.

## Commands

If no command is specified, `serve` is the default.

```
substrukt serve                              # Start the web server
substrukt import <path.tar.gz> --app <slug>  # Import a content bundle into an app
substrukt export <path.tar.gz> --app <slug>  # Export an app's content as a bundle
substrukt create-token <name> --app <slug>   # Create an API token for an app
substrukt prime                              # Output AI-optimized workflow context
substrukt onboard                            # Output a snippet for AGENTS.md / CLAUDE.md
```

### serve

Starts the web server. All flags listed above apply.

```sh
substrukt serve --port 8080 --data-dir /var/lib/substrukt --secure-cookies
```

### import

Imports a tar.gz bundle into an app's data directory. Overwrites existing schemas and content. Validates all imported content against its schema and prints warnings for any validation errors. The `--app` flag specifies which app to import into.

```sh
substrukt import backup.tar.gz --app my-app --data-dir /var/lib/substrukt
```

### export

Exports all schemas, content, and uploads for an app into a tar.gz bundle.

```sh
substrukt export backup.tar.gz --app my-app --data-dir /var/lib/substrukt
```

### create-token

Creates an API token for a specific app without starting the server. Requires at least one user to exist (run `substrukt create-admin` first).

```sh
substrukt create-token "CI deploy" --app my-app
```

The raw token is printed to stdout. Save it -- it cannot be retrieved again.

### create-admin

Creates the initial admin user without starting the server. This command only works while the user table is empty.

```sh
substrukt create-admin --email admin@example.com --username admin --password 'change-me-now'
```

## Logging

Substrukt uses the `RUST_LOG` environment variable for log filtering. The default level is `substrukt=info,tower_http=info`.

```sh
# Debug logging
RUST_LOG=substrukt=debug ./substrukt serve

# Trace everything
RUST_LOG=trace ./substrukt serve

# Only errors
RUST_LOG=error ./substrukt serve
```

The server listens on `0.0.0.0` (all interfaces) by default. The listen address is not configurable via CLI -- bind to a specific interface using a reverse proxy.
