
## Build from source

Substrukt requires Rust nightly (2026-01-05 or later).

```sh
git clone https://github.com/wavefunk/substrukt.git
cd substrukt
cargo build --release
./target/release/substrukt create-admin --email admin@example.com --username admin --password 'change-me-now'
```

The binary is at `./target/release/substrukt`.

## Run it

```sh
./target/release/substrukt serve
```

Or during development:

```sh
cargo run -- serve
```

The server starts on `http://localhost:3000` by default.

## Create the first admin

Create the first admin from the CLI before starting the server. Browser registration is disabled by default, so unattended production instances do not expose a public signup form. A default app is created automatically on startup.

1. Run `substrukt create-admin --email admin@example.com --username admin --password 'change-me-now'`
2. Start the server with `substrukt serve`
3. Open `http://localhost:3000` and sign in

## Create your first schema

Navigate to your app, then click **Schemas** in the sidebar and click **New Schema**. Paste in a JSON Schema definition:

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
    "body": { "type": "string", "format": "textarea", "title": "Body" },
    "published": { "type": "boolean", "title": "Published" }
  },
  "required": ["title"]
}
```

Click **Save**. The new content type appears in the sidebar.

## Create content

Click **Blog Posts** in the sidebar. Click **New Entry**. Fill in the form fields that were generated from your schema and save. The entry is stored as a JSON file at `data/default/content/blog-posts/<id>.json`.

## Access via API

Create an API token in your app's **Settings > API Tokens** page. Use it to fetch content:

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" http://localhost:3000/api/v1/apps/default/content/blog-posts
```
