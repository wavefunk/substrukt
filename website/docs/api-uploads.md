
Upload and retrieve files via the API.

## Upload a file

```
POST /api/v1/apps/:app_slug/uploads
Content-Type: multipart/form-data
```

Requires editor role or above.

```sh
curl -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -F "file=@photo.jpg" \
  http://localhost:3000/api/v1/apps/my-app/uploads
```

Response:

```json
{
  "hash": "a1b2c3d4e5f67890abcdef1234567890abcdef1234567890abcdef1234567890",
  "filename": "photo.jpg",
  "mime": "image/jpeg",
  "size": 245760
}
```

Use the `hash` value when referencing this upload in content entries.

If the same file is uploaded again (identical content, same SHA-256 hash), the existing file is reused. Only one copy is stored on disk.

## Download a file

```
GET /api/v1/apps/:app_slug/uploads/:hash
GET /api/v1/apps/:app_slug/uploads/:hash/:filename
```

The second form preserves the original filename in the URL, which is useful for downloads.

```sh
curl -H "Authorization: Bearer $TOKEN" \
  http://localhost:3000/api/v1/apps/my-app/uploads/a1b2c3d4e5f67890... \
  -o photo.jpg
```

Returns the file with the correct `Content-Type` header.

Returns `404` if no upload with that hash exists.

## Web UI file access

Uploads are also served at:

```
/apps/:app_slug/uploads/file/:hash/:filename
```

This path requires session authentication and is used by the web UI to display uploaded images. The filename in the URL is cosmetic -- the hash is what identifies the file.

For programmatic access, use the API endpoints above with bearer token authentication.

## The `upload:` URI scheme

Internally, Substrukt uses `upload:hash/filename` as a portable URI scheme for referencing uploads. This appears in stored richtext content (see [Field Types](./field-types.md#rich-text-markdown-richtext)).

When content is fetched via the API, `upload:` URIs in richtext HTML are automatically resolved to API paths:

```
upload:abc123/photo.jpg  →  /api/v1/apps/:app_slug/uploads/abc123/photo.jpg
```

You do not need to handle `upload:` URIs yourself when consuming the API.
