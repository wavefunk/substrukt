# SK-05/06: API Pagination and Content Search/Filtering

## Motivation

A headless CMS API that returns all entries at once is unusable for production frontends. A blog with 500 posts can't render them all on page load -- the frontend needs paginated access with predictable response sizes. Similarly, the admin UI needs robust search and filtering to manage content at scale.

These two features share API surface: the same endpoint that returns content entries needs to accept pagination parameters, search queries, field filters, and sorting directives. Designing them together ensures a coherent query interface.

## Existing Baseline

### Admin UI (substantially built)

**Search** (`src/routes/content.rs`):
- `ListParams { q, page }` query parameters.
- Text search via `content::filter_entries` -- case-insensitive substring match on all non-`_`-prefixed string fields, recursing into nested objects and arrays (capped at depth 32).
- htmx-triggered with 300ms debounce on the search input.
- Shows "Showing X of Y entries matching 'query'" feedback.

**Pagination** (`src/routes/content.rs:115-123`):
- Page-based with `PAGE_SIZE = 50`.
- Prev/Next links with page numbers.
- `page` query parameter (1-indexed, defaults to 1).

**Column sorting** (`templates/content/list.html:119-142`):
- Client-side JavaScript sort on table columns (click header to toggle asc/desc).
- Sorts by text content of cells.

**Status filtering:**
- Not exposed in the admin UI list page (no filter dropdown), but `filter_by_status` exists in `content/mod.rs`.

### API (partial)

**Existing `ListParams`** (`src/routes/api.rs:69-77`):
```rust
pub struct ListParams {
    pub q: String,      // text search
    pub status: String,  // "published" (default), "draft", "all"
    pub render: String,  // "html", "raw", or default from schema
}
```

**Search:** Text search via `content::filter_entries` (same as admin).

**Status filtering:** `content::filter_by_status` with default "published" for collections.

**No pagination.** All matching entries are returned as a flat JSON array. No limit, offset, total count, or cursor.

**No field-specific filtering.** Can't filter by a specific field value (e.g., `?author=john-doe`).

**No sorting.** Entries returned in filesystem/insertion order (`entries.sort_by(|a, b| a.id.cmp(&b.id))` for directory mode).

## Architecture

### 1. API Pagination

**Approach: Offset-based (limit/offset) with metadata envelope**

Offset-based pagination is simpler than cursor-based and sufficient for a CMS with in-memory caching. The full dataset is already loaded into memory -- offset/limit is a slice operation, not a database query. Cursor-based pagination would add complexity without meaningful performance benefit since there are no database cursors to leverage.

**Extended API `ListParams`:**

```rust
#[derive(Deserialize, Default)]
pub struct ListParams {
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub render: String,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub sort: String,
    #[serde(default)]
    pub order: String,
    #[serde(default)]
    pub fields: String,  // field-specific filters, see section 3
}
```

**Response format change:**

Currently the API returns a bare JSON array: `[{...}, {...}]`.

With pagination, the response becomes an envelope:

```json
{
  "data": [{...}, {...}],
  "meta": {
    "total": 147,
    "limit": 20,
    "offset": 0,
    "count": 20
  }
}
```

**Breaking change mitigation:** This changes the response shape. Options:
1. **Version the endpoint** (`/api/v2/...`) -- too heavy for one change.
2. **Opt-in via query param** -- return bare array by default, envelope when `limit` or `offset` is present.
3. **Always return envelope** -- breaking change, but the CMS is pre-1.0.

**Recommendation: Option 2 (opt-in envelope).** When neither `limit` nor `offset` is specified, return the bare array (backwards compatible). When either is present, return the envelope with metadata. This lets existing consumers continue working while new consumers opt into pagination.

**Default limit:** When `limit` is specified without a value or as 0, default to 20. Maximum allowed: 500 (prevents accidental full-dataset dumps via `?limit=999999`). When `limit` is absent, return all entries (bare array, no envelope).

**Offset:** 0-indexed. Invalid offsets (negative, beyond total) return empty `data` array with correct `total` in meta.

### 2. API Sorting

**Parameters:**
- `sort` -- field name to sort by (e.g., `sort=title`, `sort=_id`). Defaults to `_id` (entry ID).
- `order` -- `asc` (default) or `desc`.

**Implementation:** After filtering and before pagination, sort the entries vector by the specified field. String comparison for string fields, numeric comparison for number/integer fields. Missing values sort last.

**Sortable fields:** Any top-level field in the entry data. Nested field sorting (e.g., `sort=meta.title`) is a non-goal for v1.

**Special sort values:**
- `sort=_id` -- sort by entry ID (default, current behavior).
- `sort=_status` -- sort by draft/published status.

### 3. Field-Specific Filtering

**Problem:** Text search (`?q=`) matches across all string fields. There's no way to filter entries by a specific field value (e.g., all posts by a specific author, all entries where `category` equals "news").

**Approach: Dot-prefix field filters**

Add support for field-specific filters as query parameters:

```
GET /api/v1/apps/{app}/content/{schema}?filter.author=john-doe&filter.category=news
```

**Parsing:** Query parameters prefixed with `filter.` are extracted as field filters. Each is an exact match (case-sensitive) against the specified top-level field.

**Implementation in Rust:**

```rust
#[derive(Deserialize, Default)]
pub struct ListParams {
    // ... existing fields ...
    #[serde(flatten)]
    pub extra: HashMap<String, String>,
}
```

Then extract filter params by iterating `extra` for keys starting with `filter.`:

```rust
let filters: Vec<(&str, &str)> = params.extra.iter()
    .filter_map(|(k, v)| k.strip_prefix("filter.").map(|field| (field, v.as_str())))
    .collect();
```

**Filter semantics:**
- Exact string match for string fields.
- Numeric equality for number/integer fields (parse both sides).
- Boolean: `filter.active=true` or `filter.active=false`.
- Multiple filters are AND-combined.
- Unknown field names are silently ignored (returns no entries matching that filter, which is correct -- no entries have that field).

**New content function:**

```rust
pub fn filter_by_fields(
    entries: Vec<ContentEntry>,
    filters: &[(&str, &str)],
) -> Vec<ContentEntry>
```

### 4. Admin UI Enhancements

The admin UI already has search and pagination. The gaps are minor:

**Status filter dropdown:**
Add a `<select>` dropdown next to the search input for filtering by status (All / Published / Draft). Currently, the admin content list shows all statuses with no filter control.

Implementation: Add `status` to `ListParams` in `routes/content.rs`, render a dropdown, apply `filter_by_status` before pagination.

**Sort persistence:**
The current client-side JavaScript sorting resets on page navigation. Replace with server-side sorting via query parameters (`?sort=title&order=desc`) that persist across pagination. The sort arrows in column headers become links that add/toggle sort parameters.

### 5. Processing Pipeline

Both API and admin UI apply the same logical pipeline. The order matters:

```
1. Load all entries from disk/cache
2. Filter by status (?status=published)
3. Filter by field values (?filter.author=john-doe)
4. Filter by text search (?q=hello)
5. Count total (for pagination meta)
6. Sort (?sort=title&order=asc)
7. Paginate (offset/limit or page)
8. Transform (resolve references, render markdown, strip internal fields)
```

Step 8 (transforms) must happen after pagination to avoid wasted work on entries that won't be returned.

**Shared implementation:** Extract the pipeline into a reusable function in `src/content/mod.rs`:

```rust
pub struct QueryParams {
    pub status: String,
    pub q: String,
    pub filters: Vec<(String, String)>,
    pub sort_field: String,
    pub sort_order: SortOrder,
    pub offset: usize,
    pub limit: Option<usize>,
}

pub struct QueryResult {
    pub entries: Vec<ContentEntry>,
    pub total: usize,
}

pub fn query_entries(
    entries: Vec<ContentEntry>,
    params: &QueryParams,
) -> QueryResult
```

Both the API and admin UI handlers use this function, eliminating the duplicated filter/paginate logic.

## Data Models

### API Query Parameters (final)

| Parameter | Type | Default | Description |
|---|---|---|---|
| `q` | string | "" | Full-text search across all string fields |
| `status` | string | "published" (API) / "all" (admin) | Filter: "published", "draft", "all" |
| `limit` | integer | none (all) | Max entries per page. When present, enables envelope response. Max 500. |
| `offset` | integer | 0 | Number of entries to skip |
| `sort` | string | "_id" | Field name to sort by |
| `order` | string | "asc" | Sort direction: "asc" or "desc" |
| `filter.{field}` | string | -- | Exact match filter on a specific field |
| `render` | string | schema default | "html" or "raw" for markdown fields |

### API Response (with pagination)

```json
{
  "data": [
    { "title": "Hello", "body": "..." },
    { "title": "World", "body": "..." }
  ],
  "meta": {
    "total": 147,
    "limit": 20,
    "offset": 0,
    "count": 2
  }
}
```

### API Response (without pagination, backwards compatible)

```json
[
  { "title": "Hello", "body": "..." },
  { "title": "World", "body": "..." }
]
```

### Internal Types

```rust
#[derive(Debug, Clone, Default)]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

pub struct QueryParams {
    pub status: String,
    pub q: String,
    pub filters: Vec<(String, String)>,
    pub sort_field: String,
    pub sort_order: SortOrder,
    pub offset: usize,
    pub limit: Option<usize>,
}

pub struct QueryResult {
    pub entries: Vec<ContentEntry>,
    pub total: usize,
}
```

## Error Handling

| Scenario | Behavior |
|---|---|
| `limit` > 500 | Clamped to 500. No error. |
| `limit` = 0 or negative | Treated as default (20 when pagination is active). |
| `offset` > total entries | Returns empty `data` array with correct `total` in meta. |
| Invalid `sort` field name | Ignored, falls back to `_id` sort. |
| Invalid `order` value | Ignored, falls back to `asc`. |
| `filter.` on non-existent field | Returns empty result set (no entry has that field, so none match). |
| Conflicting filters (e.g., `status=published&filter._status=draft`) | Both applied. `status` param filters first (strips `_status` from data), `filter._status` is a no-op since `_status` is stripped. The `status` param always wins. |

## Edge Cases

### ETag cache and paginated responses
The current ETag cache keys are `{app}/{schema}/{entry_id}` for single entries. List responses compute ETags from the full response body via `json_with_etag` (`api.rs:32-67`) with `cache_key: None` -- meaning ETags are recomputed on every list request. Paginated responses have different ETags for different page/offset/sort combos. The current behavior is correct and doesn't change. No ETag caching is needed for list responses since the computation (SHA-256 of response body) is cheap relative to the serialization cost.

### `serde(flatten)` for filter params
The proposed `#[serde(flatten)] pub extra: HashMap<String, String>` on `ListParams` captures all unknown query params, including `filter.{field}` keys. This works with Axum's `Query` extractor but has a caveat: if any unknown param is present that isn't a filter (e.g., a CDN cache-busting `_t=12345`), it silently appears in `extra`. The implementation should only process keys with the `filter.` prefix and ignore everything else in `extra`.

### Empty search results with pagination
`?q=nonexistent&limit=20` returns `{ "data": [], "meta": { "total": 0, "limit": 20, "offset": 0, "count": 0 } }`.

### Status default differs between API and admin
API defaults to `status=published` (consumers see only published content). Admin defaults to showing all entries. This is intentional and should be preserved.

### Single-kind schemas
`GET /content/{slug}` redirects to the edit page in the admin UI. For the API, single-kind schemas are accessed via `GET /content/{slug}/single` which doesn't need pagination. No change needed.

### Sort stability
When multiple entries have the same value for the sort field, secondary sort by `_id` ensures stable ordering across requests. Without this, pagination could skip or duplicate entries. The implementation should use `sort_by(|a, b| primary.then_with(|| a.id.cmp(&b.id)))` pattern.

### Admin UI pagination interaction with search and status filter
When a search query or status filter is applied, the page number should reset to 1. Otherwise, changing a filter while on page 5 could show an empty page. The admin UI's htmx form should either omit the `page` param on filter change, or the backend should clamp `page` to `total_pages`.

### Filter values with special characters
Query parameters are URL-encoded by default. `filter.title=hello%20world` matches entries where `title` equals `hello world`. No additional escaping needed.

### Large datasets (10k+ entries)
All operations work on the in-memory cache. Filtering is O(n) per filter, sorting is O(n log n). For 10k entries with all filters + sort, this is sub-millisecond. Performance is not a concern at CMS scale.

## Non-Goals

- **Cursor-based pagination** -- adds complexity without benefit when the full dataset is in memory. Cursor pagination is valuable for databases where offset queries get expensive; that doesn't apply here.
- **Full-text search / fuzzy matching** -- the current substring search is adequate. Levenshtein distance, stemming, or indexed search are over-engineered for a CMS content list.
- **Field projection** -- returning only specific fields (`?fields=title,author`). The entry data is small enough that this adds complexity without meaningful payload savings.
- **Nested field filtering** -- `filter.meta.category=news` for nested objects. Top-level field filtering covers the common case; nested filtering adds parsing complexity.
- **Range filters** -- `filter.price.gt=10&filter.price.lt=100`. Exact match only for v1. Range queries can be added later without breaking changes.
- **OR logic in filters** -- all filters are AND-combined. `filter.category=news,sports` could mean either "news OR sports" or "literally the string 'news,sports'". Avoiding ambiguity; keep it simple.
- **Saved filters / views** -- admin UI filter bookmarking. URL query params serve this purpose (users can bookmark filtered URLs).
- **Real-time search** -- search updates as entries change. The cache already provides near-real-time data (200ms file watcher debounce).

## Implementation Sequence

1. **`QueryParams` / `query_entries`** in `src/content/mod.rs` -- shared pipeline for filter, sort, paginate.
2. **`filter_by_fields`** function -- field-specific exact match filtering.
3. **API pagination** -- extend `ListParams`, implement envelope response when `limit`/`offset` present.
4. **API sorting** -- `sort` and `order` parameters, stable secondary sort by `_id`.
5. **API field filters** -- `filter.{field}` parameter parsing and application.
6. **Admin UI status filter dropdown** -- add to content list page with htmx-driven updates.
7. **Admin UI server-side sorting** -- replace client-side JS sort with query parameter-driven sort.
8. **Tests** -- unit tests for `query_entries`, integration tests for paginated API responses.
