
Full CRUD for content entries. All endpoints are scoped to an app. Endpoints differ slightly for [collection vs single](./single-vs-collection.md) schemas.

## Collection endpoints

### List entries

```
GET /api/v1/apps/:app_slug/content/:schema_slug
```

By default, only published entries are returned. Use `?status=all` to include drafts, or `?status=draft` for drafts only. Use `?q=search` to filter entries by text.

```sh
curl -H "Authorization: Bearer $TOKEN" \
  http://localhost:3000/api/v1/apps/my-app/content/blog-posts
```

Response:

```json
[
  {
    "id": "my-first-post",
    "data": {
      "title": "My First Post",
      "body": "Hello world",
      "published": true
    }
  }
]
```

### Get an entry

```
GET /api/v1/apps/:app_slug/content/:schema_slug/:entry_id
```

```sh
curl -H "Authorization: Bearer $TOKEN" \
  http://localhost:3000/api/v1/apps/my-app/content/blog-posts/my-first-post
```

Response -- the entry data directly (no wrapper):

```json
{
  "title": "My First Post",
  "body": "Hello world",
  "published": true
}
```

Returns `404` if the entry does not exist.

### Create an entry

```
POST /api/v1/apps/:app_slug/content/:schema_slug
Content-Type: application/json
```

Requires editor role or above.

```sh
curl -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"title": "New Post", "body": "Content here"}' \
  http://localhost:3000/api/v1/apps/my-app/content/blog-posts
```

Response (`201 Created`):

```json
{
  "id": "new-post"
}
```

The entry ID is generated from the content (see [entry ID generation](./schemas.md#entry-id-generation)).

Validation errors return `400`:

```json
{
  "errors": ["title: \"title\" is a required property"]
}
```

### Update an entry

```
PUT /api/v1/apps/:app_slug/content/:schema_slug/:entry_id
Content-Type: application/json
```

Requires editor role or above. A version history snapshot is saved before updating.

```sh
curl -X PUT \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"title": "Updated Post", "body": "New content", "published": true}' \
  http://localhost:3000/api/v1/apps/my-app/content/blog-posts/new-post
```

Returns `200 OK` on success.

### Delete an entry

```
DELETE /api/v1/apps/:app_slug/content/:schema_slug/:entry_id
```

Requires editor role or above.

```sh
curl -X DELETE \
  -H "Authorization: Bearer $TOKEN" \
  http://localhost:3000/api/v1/apps/my-app/content/blog-posts/new-post
```

Returns `204 No Content` on success.

### Publish / Unpublish

```
POST /api/v1/apps/:app_slug/content/:schema_slug/:entry_id/publish
POST /api/v1/apps/:app_slug/content/:schema_slug/:entry_id/unpublish
```

See [Publish API](./api-publish.md) for details.

### Version history

List saved versions of an entry:

```
GET /api/v1/apps/:app_slug/content/:schema_slug/:entry_id/versions
```

Get a specific version by timestamp:

```
GET /api/v1/apps/:app_slug/content/:schema_slug/:entry_id/versions/:timestamp
```

Revert an entry to a previous version:

```
POST /api/v1/apps/:app_slug/content/:schema_slug/:entry_id/versions/:timestamp/revert
```

Requires editor role or above.

### Bulk operations

Create, update, or delete multiple entries in a single request. All bulk endpoints require editor role or above.

```
POST /api/v1/apps/:app_slug/content/:schema_slug/_bulk/create
POST /api/v1/apps/:app_slug/content/:schema_slug/_bulk/update
POST /api/v1/apps/:app_slug/content/:schema_slug/_bulk/delete
POST /api/v1/apps/:app_slug/content/:schema_slug/_bulk/publish
POST /api/v1/apps/:app_slug/content/:schema_slug/_bulk/unpublish
```

## Single endpoints

For schemas with `kind: "single"`, use the `/single` endpoints instead:

### Get

```
GET /api/v1/apps/:app_slug/content/:schema_slug/single
```

```sh
curl -H "Authorization: Bearer $TOKEN" \
  http://localhost:3000/api/v1/apps/my-app/content/site-settings/single
```

### Create or update

```
PUT /api/v1/apps/:app_slug/content/:schema_slug/single
Content-Type: application/json
```

```sh
curl -X PUT \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"site_name": "My Site", "tagline": "A great site"}' \
  http://localhost:3000/api/v1/apps/my-app/content/site-settings/single
```

### Delete

```
DELETE /api/v1/apps/:app_slug/content/:schema_slug/single
```

## Richtext fields

Schema fields with `format: "markdown-richtext"` are stored as `{"markdown": "...", "html": "..."}` objects. The API automatically projects these to a plain string in responses:

- **Default**: the `html` value is returned
- **`?render=raw`**: the `markdown` value is returned

For example, a stored entry:

```json
{
  "title": "My Post",
  "body": {
    "markdown": "# Hello\n\n![photo](upload:abc123/photo.jpg)",
    "html": "<h1>Hello</h1>\n<img src=\"upload:abc123/photo.jpg\">"
  }
}
```

Is returned by the API as:

```json
{
  "title": "My Post",
  "body": "<h1>Hello</h1>\n<img src=\"/api/v1/apps/my-app/uploads/abc123/photo.jpg\">"
}
```

### Upload URI resolution

Images and links in richtext HTML use the `upload:` URI scheme internally (e.g. `upload:hash/filename`). When returned via the API, these are resolved to the API upload path:

```
upload:abc123/photo.jpg  →  /api/v1/apps/:app_slug/uploads/abc123/photo.jpg
```

These paths require bearer token authentication, using the same token as the content request. The resolved paths are root-relative -- prepend your Substrukt instance URL to form the full download URL.

In raw mode (`?render=raw`), the markdown is returned as-is with `upload:` URIs unresolved.

## Working with uploads in content

When creating or updating content that includes upload fields, use the upload hash reference format:

```json
{
  "title": "Post with Image",
  "cover": {
    "hash": "a1b2c3d4e5f6...",
    "filename": "photo.jpg",
    "mime": "image/jpeg"
  }
}
```

Upload the file first via the [Uploads API](./api-uploads.md), then use the returned hash in your content.
