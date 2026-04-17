# SK-04: Content References (Relations Between Schemas)

## Motivation

Content types in a CMS are rarely isolated. Blog posts reference authors, products reference categories, pages reference media items. The ability to link entries across schemas is fundamental to modeling real-world content structures.

Substrukt needs a way to declare that a field in one schema points to an entry in another schema, render that as a usable selector in the form UI, resolve it to full data in API responses, and validate that the referenced entry actually exists.

## Existing Baseline

References are already substantially implemented. This spec documents what exists and designs the remaining gaps.

### What is already built

**Schema declaration:**
- `"type": "string", "format": "reference"` with `"x-substrukt-reference": { "schema": "target-slug" }` in the field definition.
- Documented in the schema editor help text (`templates/schemas/edit.html:41`).
- Reference fields are excluded from display columns in the content list (`routes/content.rs:209`) and from `generate_entry_id` (`content/mod.rs:416`).

**Form rendering** (`src/content/form.rs:386-410`):
- Reference fields render as a `<select>` dropdown with `-- Select --` placeholder.
- Options populated from `ReferenceOptions` map (field name -> list of (id, label) pairs).

**Option population** (`src/routes/content.rs:282-340`, `build_reference_options`):
- Walks the schema's top-level `properties` looking for reference fields.
- For each, reads `x-substrukt-reference.schema` to get the target slug.
- Queries the content cache for all entries in the target schema.
- Label is the first non-`_`-prefixed string field value from each entry.
- Options are sorted alphabetically by label.

**Dangling reference warning** (`src/routes/content.rs:342-381`, `warn_dangling_references`):
- On save (create and update), checks whether referenced entry IDs exist in the cache.
- Logs a `tracing::warn` if the target entry is not found. Does NOT prevent the save.

**API reference resolution** (`src/routes/api.rs:208-240`, `resolve_references`):
- On API read (list and get), replaces reference string IDs with the full inline entry data from the cache.
- Only resolves top-level reference fields (no nested object/array recursion).

**Integration test** (`tests/integration.rs:2001+`, `content_references_resolve_in_api`):
- Creates an authors schema and a posts schema with an author reference field.
- Creates an author entry, creates a post referencing it, and verifies API resolution inlines the full author data.

### What is NOT built (gaps this spec addresses)

1. **Searchable dropdown** -- the current `<select>` is unusable for schemas with hundreds of entries. The task explicitly requires a searchable interface.
2. **Array of references** -- `build_reference_options` only walks top-level properties; it does not recurse into array items or nested objects. A field like `{ "type": "array", "items": { "type": "string", "format": "reference", ... } }` will render as a plain text input, not a dropdown.
3. **Label field configuration** -- the label is always the first non-`_` string field, which is unpredictable (depends on JSON key ordering). No way to specify which field should be the human-readable label.
4. **Hard validation of reference existence** -- `warn_dangling_references` only logs; it does not reject invalid references. `validate_content` does not check references at all.
5. **Referential integrity on delete** -- deleting a referenced entry (e.g., an author) silently orphans all entries pointing to it.
6. **Nested reference resolution in API** -- `resolve_references` only handles top-level fields, not references inside nested objects or arrays.

## Architecture

### 1. Searchable Reference Dropdown

**Approach: htmx-powered typeahead with server-side search**

A plain HTML `<select>` doesn't scale beyond ~50 entries. Browser-native `<datalist>` has inconsistent behavior across browsers and limited styling. An htmx-powered typeahead fits the existing server-rendered + htmx pattern and provides a consistent UX.

**UI component:**
- Text input with `hx-get` that triggers on input (debounced 300ms).
- Results rendered as a dropdown list below the input.
- Hidden input stores the actual entry ID.
- Clicking a result selects it, showing the label in the visible input and the ID in the hidden input.
- Current selection shows as a chip/tag with a clear button.

**New route:**

```
GET /apps/{app}/content/{schema_slug}/_ref/search?target={target_slug}&q={query}&limit={10}
```

- Returns HTML fragment (htmx partial) with matching entries.
- Searches entry data using the existing `matches_query` function from `content/mod.rs`.
- Returns `id` and `label` for each match.
- Limited to first `limit` results (default 10).

**Fallback for small datasets:** When the target schema has fewer than 50 entries, render as a standard `<select>` (current behavior) -- no need for typeahead complexity. `build_reference_options` already computes the full list; the form renderer can check the count and choose the appropriate widget.

**Form rendering change** (`form.rs`):
- When `ref_options` for a field has <= 50 entries: render `<select>` (current behavior).
- When > 50 entries: render the typeahead component with htmx attributes.
- The threshold (50) is a constant, not configurable.

### 2. Array and Nested Object References

**Problem:** `build_reference_options` only walks top-level `schema.properties`. Array items and nested objects with reference fields are silently ignored, rendering as plain text inputs.

**Fix:** Make `build_reference_options` recursive.

```rust
fn build_reference_options(
    schema: &Value,
    cache: &ContentCache,
    prefix: &str,
    app_slug: &str,
) -> ReferenceOptions
```

Changes:
1. When encountering a `type: "object"` field, recurse into it with the field name appended to `prefix`.
2. When encountering a `type: "array"` field with `items` that has `properties`, recurse into `items` with `{prefix}.{key}[__INDEX__]` pattern -- or, more practically, use the bare field name without index and have the form renderer look up reference options by stripping array indices from the field name.

**Lookup strategy for array items:** The form renderer generates names like `tags[0]`, `tags[1]`, etc. Rather than pre-populating options for every possible index, store options under the base field path (e.g., `tags`) and have the reference field renderer strip the `[N]` suffix when looking up options. This requires a small change to the `ref_options.get(name)` lookup in `form.rs:389`.

Similarly, `resolve_references` in `api.rs` and `warn_dangling_references` in `content.rs` need the same recursive treatment. They currently only walk top-level properties.

### 3. Configurable Label Field

**Problem:** The label for referenced entries is the first non-`_`-prefixed string field. This is unpredictable and often wrong (e.g., showing a slug instead of a display name).

**Solution:** Add an optional `label_field` to the reference configuration.

**Schema declaration (extended):**
```json
{
  "type": "string",
  "format": "reference",
  "x-substrukt-reference": {
    "schema": "authors",
    "label_field": "display_name"
  }
}
```

When `label_field` is specified, use that field from the referenced entry as the label. When absent, fall back to the current behavior (first non-`_` string field).

**Changes to `build_reference_options`:** Read `x-substrukt-reference.label_field` and use it in the label extraction logic instead of the generic first-string-field scan.

### 4. Reference Existence Validation

**Current state:** `warn_dangling_references` logs warnings but does not prevent saving. `validate_content` runs JSON Schema validation only -- it doesn't check reference IDs.

**Decision: Keep warn-only as default, add opt-in strict mode.**

Rationale:
- Hard validation would prevent saving drafts that reference entries not yet created (circular dependency during content creation).
- Migration workflows often create entries in arbitrary order; hard validation would force a specific creation sequence.
- The warning is already logged and visible in structured logs.

**New behavior:**
- **On save (admin + API):** Continue warn-only via `warn_dangling_references`. No change.
- **On publish:** When publishing an entry (changing status to "published"), validate that all referenced entries exist. Block the publish if any are missing, with an error message listing the dangling references. Rationale: published content should have valid references; drafts are allowed to be incomplete.

This requires adding a new function:

```rust
pub fn validate_references(
    data: &Value,
    schema: &Value,
    cache: &ContentCache,
    app_slug: &str,
) -> Vec<String>  // list of error messages for dangling references
```

Called from `set_entry_status` (or the publish route handlers) when `status == "published"`.

### 5. Referential Integrity on Delete

**Problem:** Deleting a referenced entry silently orphans referencing entries.

**Approach: Warn on delete, do not block.**

When an entry is about to be deleted, check whether any other entries reference it. If so:
- **Admin UI:** Show a confirmation dialog listing the referencing entries. "This entry is referenced by 3 entries in Posts. Deleting it will leave dangling references. Continue?"
- **API:** Include a `warnings` field in the delete response listing affected entries. Do not block the delete.

**Implementation:** Before deletion, scan the content cache for entries that contain this entry's ID in reference fields. This requires:

```rust
pub fn find_referencing_entries(
    cache: &ContentCache,
    schemas_dir: &Path,
    app_slug: &str,
    target_schema_slug: &str,
    target_entry_id: &str,
) -> Vec<(String, String)>  // (schema_slug, entry_id) pairs
```

This function walks all schemas, checks which ones have reference fields pointing to `target_schema_slug`, then scans cached entries for matching IDs.

**Why not block?** Blocking deletes based on references creates tight coupling that makes content management frustrating. The CMS should be permissive by default -- the dangling reference warning on save (and hard check on publish) provides sufficient protection.

### 6. Nested Reference Resolution in API

**Problem:** `resolve_references` in `api.rs:208` only resolves top-level reference fields. References inside nested objects or arrays are returned as raw string IDs.

**Fix:** Make `resolve_references` recursive, matching the pattern of `render_markdown_fields`.

Walk the schema recursively. For `type: "object"`, recurse into nested properties. For `type: "array"` with object items, iterate array elements and recurse. Depth-limited to `MAX_NESTING_DEPTH` (32).

## Data Models

### Schema Declaration (extended)

```json
{
  "type": "string",
  "format": "reference",
  "title": "Author",
  "x-substrukt-reference": {
    "schema": "authors",
    "label_field": "display_name"  // optional, defaults to first string field
  }
}
```

No new on-disk data structures. References are stored as plain string IDs in entry JSON -- the existing format is unchanged.

### New Types

None. The existing `ReferenceOptions = HashMap<String, Vec<(String, String)>>` is sufficient.

### Search Endpoint Response (HTML Fragment)

Not a data model per se, but the search endpoint returns an HTML partial:

```html
<div class="ref-option" data-id="john-doe" data-label="John Doe">
  <span class="font-medium">John Doe</span>
  <span class="text-muted text-xs">john-doe</span>
</div>
```

## Error Handling

| Scenario | Behavior |
|---|---|
| Reference to non-existent schema | `build_reference_options` silently skips (no target entries found). Form shows empty dropdown. |
| Reference to non-existent entry (save) | `warn_dangling_references` logs a warning. Save proceeds. |
| Reference to non-existent entry (publish) | Publish blocked with error: "Field 'author' references entry 'deleted-user' which does not exist." |
| Target schema has zero entries | Dropdown/typeahead shows empty state: "No entries available." |
| Circular references (A -> B -> A) | Allowed. References are just string IDs; resolution is one level deep. No infinite recursion risk. |
| Self-reference (schema references itself) | Allowed and functional. Useful for hierarchical content (categories with parent category). |
| Deleted entry has incoming references | Admin: confirmation dialog lists affected entries. API: `warnings` in response. Delete proceeds. |

## Edge Cases

### Cache staleness
Reference options are populated from the in-memory content cache. If the cache is stale (e.g., file watcher hasn't triggered yet), the dropdown may show outdated entries. The cache is eventually consistent -- file watcher rebuilds within 200ms of changes. Acceptable for a CMS.

### Cross-app references
Not supported. References can only point to schemas within the same app. `x-substrukt-reference.schema` is interpreted as a schema slug within the current app. Explicit non-goal.

### Single-file collections as reference targets
References resolve by entry ID. For single-file collections, entry IDs are stored in the `_id` field. This works correctly -- `build_reference_options` iterates cache entries which already have `_id`-derived keys.

### Reference field in array items
Today: broken (silently renders as plain text input). After fix: works like top-level references, with options looked up by base field name (index stripped). Each array element gets its own reference selector, all sharing the same option set.

### Label field doesn't exist
If `x-substrukt-reference.label_field` names a field that doesn't exist in the target entry, fall back to first string field (current behavior). Log a warning.

### Interaction with SK-03 advanced validation
SK-03 proposes `validate_for_publish` which checks required-if-published rules when publishing. Reference existence validation on publish (Section 4 of this spec) should integrate into the same validation pass. Both specs propose validation that runs at publish time -- implementers should combine these into a single publish validation pipeline rather than having two separate validation checks.

### Typeahead debounce and empty state
The typeahead search should not fire on empty input (no `q` parameter). An empty search should show the most recently created entries (up to `limit`) as a starting point, not all entries. This keeps the initial dropdown useful without requiring the user to type first.

### Large target schemas (10,000+ entries)
The typeahead search queries the cache and returns limited results. The cache scan is O(n) but operates on an in-memory DashMap, which is fast for typical CMS sizes (< 100k entries). For extreme cases, the `limit` parameter on the search endpoint keeps response times bounded.

## Non-Goals

- **Cross-app references** -- apps are isolated content spaces. Cross-app linking would break the isolation model.
- **Multi-hop resolution** -- API resolves references one level deep. If author.company is also a reference, it stays as a string ID. Deep resolution is a separate feature.
- **Reverse reference fields** -- showing "which entries reference this one" on the edit page. The delete warning covers the practical need; a dedicated reverse-references view is future work.
- **Reference to specific version** -- references point to the current entry, not a historical version.
- **Polymorphic references** -- referencing multiple possible target schemas from one field. Would require a more complex format (e.g., `schema_slug:entry_id` storage format).
- **Cascade delete** -- deleting a referenced entry does not automatically delete or modify referencing entries. Too destructive for a CMS default.
- **Required reference validation on draft save** -- intentionally permissive. Only publish enforces reference integrity.

## Implementation Sequence

1. **Recursive `build_reference_options`** -- fix the array/nested-object gap. Update form.rs lookup to strip array indices.
2. **Configurable `label_field`** -- extend `build_reference_options` to read from `x-substrukt-reference.label_field`.
3. **Searchable dropdown component** -- new route `/_ref/search`, htmx-powered typeahead in form.rs, threshold-based rendering (select vs typeahead).
4. **Recursive `resolve_references`** -- fix API resolution for nested/array references.
5. **Recursive `warn_dangling_references`** -- fix warning for nested/array references.
6. **Publish-time reference validation** -- `validate_references` function, integrate into publish handlers.
7. **Delete warning** -- `find_referencing_entries` function, integrate into delete handlers (admin confirmation, API warnings).
8. **Tests** -- unit tests for recursive traversal, integration tests for typeahead and validation.
