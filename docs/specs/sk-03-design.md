# SK-03: Advanced Content Validation

## Motivation

Substrukt currently validates content entries against their JSON Schema using the `jsonschema` crate (`content::validate_content` in `src/content/mod.rs:362`). This handles standard constraints (required, type, minLength, pattern, enum, etc.) but can't express:

1. **Uniqueness** -- no two blog posts should have the same `slug` within a collection.
2. **Cross-field rules** -- `end_date` must be after `start_date`.
3. **Required-if-published** -- certain fields are optional while drafting but mandatory before publishing.

These are common CMS requirements that JSON Schema alone cannot express. The feature adds custom validation rules defined via `x-substrukt` schema extensions, validated server-side, and surfaced as per-field errors in the admin form UI.

## Existing Baseline

### Current validation pipeline

1. **Schema-level**: `schema::validate_schema()` checks JSON Schema is parseable and has `x-substrukt` metadata (`src/schema/mod.rs:80-96`).
2. **Content-level**: `content::validate_content(schema, data) -> Result<(), Vec<String>>` patches upload types, compiles the schema with `jsonschema::validator_for`, collects errors as `Vec<String>` with format `"{instance_path}: {message}"` (`src/content/mod.rs:362-380`).
3. **Callers**: admin UI create/update (`routes/content.rs:590, 713`), API create/update/upsert (`routes/api.rs:417, 477, 644`), import validation (`sync/mod.rs:153`).
4. **Error display**: admin UI renders errors as a flat list in a red box at the top of the form (`templates/content/edit.html:23-32`). No per-field inline errors.
5. **Status handling**: `save_entry` resolves `_status` (lines 115-136) *after* validation runs. Publish/unpublish use `set_entry_status` which bypasses validation entirely (metadata-only change, `content/mod.rs:232-292`).

### Extension point pattern

The project uses `x-substrukt` as a top-level schema extension for metadata (title, slug, storage, kind, id_field, render) and `x-substrukt-reference` on individual properties for cross-schema references.

## Architecture

### Extension syntax

Two extension points, following existing patterns:

**Per-field rules** on individual properties:

```json
{
  "properties": {
    "slug": {
      "type": "string",
      "x-substrukt-unique": true
    },
    "summary": {
      "type": "string",
      "x-substrukt-required-if-published": true
    }
  }
}
```

**Cross-field rules** in the top-level `x-substrukt` block:

```json
{
  "x-substrukt": {
    "title": "Events",
    "slug": "events",
    "validate": [
      {
        "rule": "after",
        "field": "end_date",
        "reference": "start_date",
        "message": "End date must be after start date"
      }
    ]
  }
}
```

### Validation context

The current `validate_content(schema, data)` signature lacks the information needed for advanced rules: it doesn't know the entry ID (for uniqueness exclusion), target status (for required-if-published), or have access to other entries (for uniqueness checking).

**New type** in `src/content/mod.rs`:

```rust
pub struct ValidationContext<'a> {
    pub entry_id: Option<&'a str>,
    pub target_status: &'a str,
    pub cache: &'a crate::state::ContentCache,
    pub app_slug: &'a str,
    pub schema_slug: &'a str,
}
```

**Updated signature**:

```rust
pub fn validate_content(
    schema: &SchemaFile,
    data: &Value,
    ctx: &ValidationContext<'_>,
) -> Result<(), Vec<ValidationError>>
```

The function runs JSON Schema validation first, then applies advanced rules. All errors are collected and returned together.

### Structured error type

Replace the current `Vec<String>` with a structured error:

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct ValidationError {
    pub path: String,      // field path, e.g. "slug", "meta.title", "" for cross-field
    pub message: String,   // human-readable error message
    pub rule: String,      // rule identifier: "required", "type", "unique", "after", "required_if_published"
}
```

This enables:
- **Per-field inline errors**: the template can match errors to specific form fields by path.
- **Backwards compatibility**: the API can serialize these as before (just the message), or return the full structure for clients that want it.

**Migration of callers**: All 6 call sites must be updated to pass `ValidationContext`. The admin UI callers already have `entry_id`, `session`, and `state` in scope. API callers have `entry_id` and `state`. Import validation passes a minimal context (no cache-based uniqueness checking for imports -- see Edge Cases).

### Status resolution before validation

Currently, `save_entry` resolves `_status` after validation. For required-if-published rules, the target status must be known before validation. The resolution logic (explicit > existing > default) must be extracted into a shared helper:

```rust
pub fn resolve_target_status(
    data: &Value,
    content_dir: &Path,
    schema: &SchemaFile,
    entry_id: Option<&str>,
) -> String
```

This function replicates the logic from `save_entry` lines 115-136 but runs before validation. For updates without an explicit `_status` in the data, it reads the existing entry from disk to determine the current status -- the same disk read that `save_entry` will repeat later. Callers:
1. Call `resolve_target_status()` to determine the effective status.
2. Create `ValidationContext` with the resolved status.
3. Call `validate_content(schema, data, &ctx)`.
4. If valid, call `save_entry()` (which re-resolves status independently -- same result absent concurrent modification, which is the existing last-write-wins behavior).

## Validation Rules

### 1. Unique constraint

**Schema syntax**: `"x-substrukt-unique": true` on a property.

**Semantics**: No two entries in the same collection can have the same value for this field. Comparison is case-insensitive for strings, exact for numbers/booleans.

**Implementation**: Check the `ContentCache` (in-memory `DashMap`) for all entries in the same schema. Filter out the current entry by `entry_id` (for updates). If any other entry has the same value at the same field path, emit a `ValidationError { path: field_name, message: "Value must be unique...", rule: "unique" }`.

**Lookup source**: `ContentCache`, not disk. The cache key format is `"{app_slug}/{schema_slug}/{entry_id}"`. Iterate entries matching the `"{app_slug}/{schema_slug}/"` prefix.

```rust
fn validate_unique(
    field: &str,
    value: &Value,
    ctx: &ValidationContext<'_>,
) -> Option<ValidationError>
```

**Constraints**:
- Top-level fields only. Nested paths (e.g., `meta.slug`) are not supported for uniqueness because the cache stores full entry JSON and traversing nested paths in DashMap iteration adds complexity for minimal gain.
- Only `string`, `number`, `integer`, and `boolean` types. Upload and reference fields are excluded.
- `null` / missing values are not checked for uniqueness (multiple entries can omit the field).

### 2. Cross-field rules

**Schema syntax**: Array of rule objects in `x-substrukt.validate`:

```json
{
  "rule": "after",
  "field": "end_date",
  "reference": "start_date",
  "message": "End date must be after start date"
}
```

**Supported rules** (Rust enum):

```rust
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum CrossFieldRule {
    After {
        field: String,
        reference: String,
        #[serde(default)]
        message: Option<String>,
    },
    Before {
        field: String,
        reference: String,
        #[serde(default)]
        message: Option<String>,
    },
    RequiredWith {
        field: String,
        when: String,
        #[serde(default)]
        message: Option<String>,
    },
    NotEqual {
        field: String,
        reference: String,
        #[serde(default)]
        message: Option<String>,
    },
}
```

**`after`**: `field` value must be strictly greater than `reference` value. Works for strings (lexicographic comparison -- correct for ISO 8601 dates in consistent `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM:SS` format), numbers, and booleans (false < true). Both fields must be present and non-null; if either is missing, the rule is silently skipped (not an error). For mixed types (e.g., one is a string and the other a number), the rule is skipped with a warning log.

**`before`**: Inverse of `after`. `field` value must be strictly less than `reference` value.

**`required_with`**: If `when` field is non-null and non-empty, then `field` is required. Useful for conditional fields like "if you provide a URL, also provide a link title."

**`not_equal`**: `field` and `reference` must have different values. Useful for "start_date must not equal end_date."

Default messages are generated from the rule type if not provided: e.g., `"{field} must be after {reference}"`.

### 3. Required-if-published

**Schema syntax**: `"x-substrukt-required-if-published": true` on a property.

**Semantics**: The field is treated as optional when the target status is `"draft"` and required when the target status is `"published"`. This means:
- Creating/updating a draft with the field empty: valid.
- Creating/updating a published entry with the field empty: invalid.
- Publishing a draft that's missing the field: invalid.

**Implementation**: After JSON Schema validation runs (which enforces the `required` array), check all properties with `x-substrukt-required-if-published: true`. If the target status is `"published"` and the field is null, empty string, or absent, emit an error.

**Critical: publish action must validate**

Currently, `set_entry_status` in `content/mod.rs:232-292` changes `_status` without validation. The publish routes in `routes/content.rs:869-958` and `routes/api.rs:741-793` call it directly. With required-if-published rules, publishing must validate first.

**Approach**: Add a `validate_for_publish` helper that loads the current entry data, constructs a `ValidationContext` with `target_status: "published"`, and runs advanced validation (skip JSON Schema since the data isn't changing -- only status is). If validation fails, the publish action is rejected with errors.

Updated publish flow:
1. Load current entry data from disk.
2. Run `validate_for_publish(schema, data, ctx)` -- checks only required-if-published rules.
3. If errors: return them (admin UI: flash + redirect with errors; API: 400 with error body).
4. If valid: call `set_entry_status` as before.

```rust
pub fn validate_for_publish(
    schema: &SchemaFile,
    data: &Value,
    ctx: &ValidationContext<'_>,
) -> Result<(), Vec<ValidationError>>
```

## Error Handling and UI

### Structured error display

**Admin UI**: Upgrade from flat list to per-field inline errors.

The template currently renders:
```html
{% if errors %}
<div class="bg-danger-soft text-danger p-4 rounded mb-4">
  <ul>{% for error in errors %}<li>{{ error }}</li>{% endfor %}</ul>
</div>
{% endif %}
```

Updated approach:
1. Render errors as a keyed map (JSON serialized to template): `{ "slug": ["Value must be unique"], "end_date": ["End date must be after start date"] }`.
2. Form field rendering in `form.rs` accepts an optional error map. Each field checks for errors at its path and renders inline error messages below the input.
3. A summary banner at the top still shows all errors for scrolled-past fields.

**API**: Return structured errors:

```json
{
  "errors": [
    { "path": "slug", "message": "Value must be unique within this collection", "rule": "unique" },
    { "path": "end_date", "message": "End date must be after start date", "rule": "after" }
  ]
}
```

For backwards compatibility, if callers expect the old `Vec<String>` format, the `ValidationError::to_string()` impl produces `"{path}: {message}"`.

### Error responses by context

| Context | Error behavior |
|---|---|
| Admin create/update | Re-render form with inline per-field errors + summary banner |
| Admin publish | Flash error with message listing missing required-if-published fields |
| API create/update | 400 with `{"errors": [...]}` structured response |
| API publish | 400 with `{"errors": [...]}` |
| Import validation | Non-fatal warnings (existing behavior); advanced rules not enforced on import |

## Data Models

### New types in `src/content/mod.rs`

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct ValidationError {
    pub path: String,
    pub message: String,
    pub rule: String,
}

pub struct ValidationContext<'a> {
    pub entry_id: Option<&'a str>,
    pub target_status: &'a str,
    pub cache: &'a crate::state::ContentCache,
    pub app_slug: &'a str,
    pub schema_slug: &'a str,
}
```

### New types in `src/schema/models.rs` or `src/content/mod.rs`

```rust
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum CrossFieldRule {
    After {
        field: String,
        reference: String,
        #[serde(default)]
        message: Option<String>,
    },
    Before {
        field: String,
        reference: String,
        #[serde(default)]
        message: Option<String>,
    },
    RequiredWith {
        field: String,
        when: String,
        #[serde(default)]
        message: Option<String>,
    },
    NotEqual {
        field: String,
        reference: String,
        #[serde(default)]
        message: Option<String>,
    },
}
```

### Schema extension on properties (per-field)

```json
{
  "slug": {
    "type": "string",
    "x-substrukt-unique": true,
    "x-substrukt-required-if-published": true
  }
}
```

### Schema extension on `x-substrukt` (cross-field)

```json
{
  "x-substrukt": {
    "title": "Events",
    "slug": "events",
    "validate": [
      { "rule": "after", "field": "end_date", "reference": "start_date" }
    ]
  }
}
```

### Updated `SubstruktMeta`

```rust
pub struct SubstruktMeta {
    pub title: String,
    pub slug: String,
    pub storage: StorageMode,
    pub kind: Kind,
    pub id_field: Option<String>,
    pub render: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validate: Vec<CrossFieldRule>,
}
```

## Edge Cases

### Single-kind schemas

Uniqueness is meaningless for `Kind::Single` (only one entry exists). The unique constraint is silently skipped. Cross-field and required-if-published rules apply normally.

### Case sensitivity for uniqueness

String comparisons are case-insensitive (`.to_lowercase()` before comparing). This prevents confusing situations where "hello-world" and "Hello-World" are both accepted as "unique." Numbers and booleans use exact comparison.

### Concurrent creates (race condition)

Two simultaneous requests creating entries with the same unique value can both pass the cache check and both succeed. There is no entry-level locking. This is the same race condition as concurrent edits (last-write-wins). After both writes, the cache rebuild detects the duplicate, but no automatic remediation occurs. This is an accepted limitation at the scale of this CMS. The race window is small (both requests must pass validation before either writes to disk).

### Null/missing fields

- **Uniqueness**: `null` and missing fields are not checked. Multiple entries can have a null/missing unique field.
- **Cross-field rules**: If either field in an `after`/`before`/`not_equal` rule is null or missing, the rule is silently skipped. This prevents false errors when optional fields are left empty.
- **Required-if-published**: Only checked when target status is `"published"`. A field is "missing" if it's absent from the data, null, or an empty string.

### Publish action validation

Publishing a draft that's missing required-if-published fields must fail. The publish routes (`routes/content.rs:869`, `routes/api.rs:741`) currently call `set_entry_status` without validation. After this feature:

1. Publish routes load current entry data.
2. Run `validate_for_publish()` with `target_status: "published"`.
3. If errors, reject the publish with appropriate error response.
4. If valid, proceed with `set_entry_status`.

Unpublish (published -> draft) never triggers required-if-published errors, since the target status is `"draft"`.

### Bulk publish

`bulk_publish` in `routes/content.rs:1154` publishes multiple entries. With required-if-published validation, each entry must be individually validated. Entries that fail validation are skipped (not published), and a flash message reports which entries failed and why.

### Import validation

Import validation in `sync/mod.rs:148-158` strips `_` fields before validating. With advanced rules:
- **Uniqueness**: Not enforced during import (no `ValidationContext` with cache). Duplicate entries in the bundle are accepted. The post-import cache rebuild will reflect the duplicates.
- **Required-if-published**: Enforced at the entry's own `_status`. If an imported published entry is missing a required-if-published field, it's reported as a non-fatal warning (matching existing import behavior).
- **Cross-field rules**: Enforced during import and reported as non-fatal warnings.

### Existing content when new rules are added

Adding a new validation rule to a schema does not retroactively flag existing entries. Rules only apply when content is created, updated, or published. Existing entries that violate new rules remain valid until next edit. This is intentional: schema authors should be able to add rules without triggering a data migration. A future "audit" feature could scan existing content against current rules, but that's out of scope.

### Upload and reference fields

Upload fields (stored as `{hash, filename, mime}` objects) and reference fields (`format: "reference"`) are excluded from uniqueness constraints. Their stored values are complex objects or opaque IDs, making uniqueness semantically unclear.

### Form hint display

For fields with `x-substrukt-required-if-published: true`, the form renderer adds a hint: "Required when published" below the field. This is rendered in `form.rs` alongside existing hints (character limits, patterns, etc.).

For fields with `x-substrukt-unique: true`, the form renderer adds: "Must be unique" as a hint.

## Non-Goals

- **Async / external validation**: No calling external APIs or running database queries during validation. All rules are evaluated synchronously from in-memory data.
- **Expression language / DSL**: No custom expression evaluator. Cross-field rules use a fixed set of named rule types (extensible via new enum variants, not via user-authored expressions).
- **Cross-app uniqueness**: Uniqueness is scoped to a single schema within a single app. Cross-app or cross-schema uniqueness is not supported.
- **Nested path uniqueness**: Uniqueness only works on top-level properties. Uniqueness on nested paths like `meta.slug` is not supported.
- **Client-side validation**: All advanced rules are validated server-side only. The admin UI renders hints but does not enforce rules in JavaScript. Standard JSON Schema constraints (required, minLength, etc.) continue to use HTML5 form validation attributes.
- **Retroactive enforcement**: Adding a new rule does not flag existing entries. Rules apply on create/update/publish only.
- **Custom error message localization**: Error messages are English-only. The optional `message` field on cross-field rules allows schema authors to customize the message.

## Implementation Sequence

### Phase 1: Foundation

1. Define `ValidationError` struct and `ValidationContext` struct in `src/content/mod.rs`.
2. Extract `resolve_target_status()` from `save_entry` logic.
3. Update `validate_content` signature to accept `ValidationContext` and return `Vec<ValidationError>`.
4. Convert existing `jsonschema` errors to `ValidationError` format.
5. Update all 6 caller sites to pass `ValidationContext`.
6. Update admin UI template to handle `ValidationError` (initially just `error.message` display, same as before).

### Phase 2: Uniqueness

1. Add `x-substrukt-unique` detection in `validate_content`.
2. Implement `validate_unique()` function using `ContentCache` lookup.
3. Add "Must be unique" hint in `form.rs`.
4. Unit tests: unique field on create (pass, fail), on update (exclude self), case-insensitive, null values, single-kind skip.

### Phase 3: Required-if-published

1. Add `x-substrukt-required-if-published` detection.
2. Implement check in `validate_content` (gated on `ctx.target_status == "published"`).
3. Implement `validate_for_publish()` helper.
4. Update publish routes (admin UI + API) to validate before `set_entry_status`.
5. Update bulk publish to validate per-entry.
6. Add "Required when published" hint in `form.rs`.
7. Unit tests: draft saves with missing field (pass), published saves with missing field (fail), publish action with missing field (fail), unpublish (always pass).

### Phase 4: Cross-field rules

1. Add `CrossFieldRule` enum and `validate` field to `SubstruktMeta`.
2. Implement rule evaluation: `after`, `before`, `required_with`, `not_equal`.
3. Unit tests per rule type, including null/missing field handling.

### Phase 5: Per-field inline errors

1. Update `validate_content` to group errors by field path.
2. Update `render_form_fields` signature in `form.rs` to accept an optional error map (`Option<&HashMap<String, Vec<String>>>`). Current signature: `render_form_fields(schema, data, prefix, ref_options, app_slug)` (`form.rs:210`). All existing callers pass `None` initially; error-rendering callers pass the grouped error map.
3. In `render_field` (`form.rs:290`), check the error map for the current field name and render inline error `<p>` elements below the input with danger styling.
4. Update admin templates to pass error map to form renderer via the render call in create/update handlers.
5. Keep the summary banner for errors on non-visible or scrolled-past fields.

### Phase 6: API and polish

1. Update API error responses to return structured `ValidationError` objects.
2. Update OpenAPI spec to document advanced validation extensions.
3. Update import validation to pass minimal `ValidationContext`.
4. Integration tests: end-to-end create/update/publish flows with advanced rules via admin UI and API.

## Testing Strategy

- **Unit tests** for `validate_unique()`: create duplicate (fail), update self (pass), case-insensitive strings, null values skipped, single-kind skip, different schema entries don't conflict.
- **Unit tests** for `validate_for_publish()`: missing required-if-published field (fail), present field (pass), draft target status (always pass).
- **Unit tests** for cross-field rules: `after` with dates (pass, fail, missing field), `before`, `not_equal`, `required_with` (present when condition met, absent when condition absent).
- **Unit tests** for `resolve_target_status()`: explicit status in data, preserve existing, default to draft.
- **Integration tests**: admin UI create with unique violation (re-renders form with error), API update with cross-field violation (400 response), publish with missing required-if-published field (rejected), bulk publish partial success.
- **Integration tests** for import: import with violations produces warnings, content is still imported.
