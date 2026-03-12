# Singles: Single-Object Content Type

## Summary

Add support for "single" objects in Substrukt — one-off content items (site settings, homepage hero, about page) where there's exactly one instance per schema, no list view, and no ID management. Singles are implemented as a thin wrapper on the existing collection system, reusing all content CRUD, validation, form rendering, and upload handling.

## Schema Metadata

Add an optional `kind` field to the `x-substrukt` metadata block:

```json
{
  "x-substrukt": {
    "title": "Site Settings",
    "slug": "site-settings",
    "storage": "directory",
    "kind": "single"
  }
}
```

- `kind` is optional, defaults to `"collection"` (fully backward compatible).
- Valid values: `"single"`, `"collection"`.
- `SubstruktMeta` struct gets a new `Kind` enum field (`Single`, `Collection`).
- Schema create/edit UI gets a dropdown or toggle for selecting the kind.
- Schema validation checks that `kind` is a valid value.

## Content Storage & CRUD

Singles reuse the existing content entry system with a fixed entry ID:

- **Fixed ID**: When `kind` is `"single"`, the entry uses the fixed ID `"_single"`. No ID generation from fields, no slugification.
- **Storage mode**: Uses whichever `storage` mode the schema specifies (directory or single-file). In practice, a single with directory mode creates `data/content/site-settings/_single.json`.
- **Create vs Update**: Saving a single checks if `_single` exists — updates if yes, creates if no. The form always behaves like an edit form.
- **Lazy creation**: No file is written until the user first saves. An empty form is shown for singles that haven't been saved yet.
- **List entries**: Still works (returns 0 or 1 entries). The UI just never shows the list view for singles.
- **Delete**: Deleting a single's entry resets it to the "never edited" state (empty form on next visit).

The content module itself requires no changes. All behavioral differences live in the routes and UI layer.

## Web Routes & UI

**Routing behavior:**

- `GET /content/{schema_slug}` — for singles, redirects to `/content/{schema_slug}/_single/edit` instead of showing the list view.
- `GET /content/{schema_slug}/_single/edit` — shows the edit form (empty if no entry exists yet).
- `POST /content/{schema_slug}/_single` — saves the single (creates or updates the `_single` entry).

**UI changes:**

- The "New Entry" button is hidden for singles.
- Schema list/dashboard indicates which schemas are singles vs collections.
- The edit form works identically to collection entry editing — same form rendering, validation, upload handling.

## API Routes

Dedicated `/single` sub-path within the existing content namespace:

- `GET /api/v1/content/{schema_slug}/single` — returns the `_single` entry's data directly (unwrapped, not `{id, data}`), or 404 if not yet created.
- `PUT /api/v1/content/{schema_slug}/single` — creates or updates the `_single` entry. Validates against schema.
- `DELETE /api/v1/content/{schema_slug}/single` — deletes the single entry.

Existing collection endpoints still technically work on the underlying `_single` entry, but the `/single` endpoint is the intended interface for consumers.

## Export/Import

No special handling needed:

- The `_single` entry is a regular content entry — bundled and restored like any other.
- The schema's `kind` field is part of the schema JSON and round-trips naturally.

## Edge Cases

- **Changing kind from `collection` to `single`**: Existing entries remain on disk. Only `_single` is accessible via the single UI/API. No destructive behavior.
- **Changing kind from `single` to `collection`**: The `_single` entry becomes a regular entry with ID `_single` in the list view.
- **API collision**: The fixed path segment `/single` cannot collide with entry IDs since it's a reserved path. Entry IDs generated from content fields are slugified and would not produce `single` as an ID (and if manually set, the route takes precedence).
