
Substrukt generates UI form elements from JSON Schema property definitions. Each combination of `type` and `format` maps to a specific HTML input.

## Supported types

| JSON Schema | Format | UI Element | Stored as |
|-------------|--------|------------|-----------|
| `"type": "string"` | (none) | Text input | `"value"` |
| `"type": "string"` | `"textarea"` | Multi-line textarea | `"value"` |
| `"type": "string"` | `"upload"` | File input | `{"hash": "...", "filename": "...", "mime": "..."}` |
| `"type": "string"` | `"markdown"` | Markdown textarea | `"# heading\n..."` |
| `"type": "string"` | `"markdown-richtext"` | WYSIWYG rich text editor | `{"markdown": "...", "html": "..."}` |
| `"type": "string"` + `"enum"` | (none) | Select dropdown | `"value"` |
| `"type": "number"` | (none) | Number input (decimal) | `1.5` |
| `"type": "integer"` | (none) | Number input (whole) | `42` |
| `"type": "boolean"` | (none) | Checkbox | `true` / `false` |
| `"type": "object"` | (none) | Nested fieldset | `{ ... }` |
| `"type": "array"` | (none) | Repeatable field group | `[ ... ]` |

## String fields

A basic text input:

```json
{
  "title": { "type": "string", "title": "Title" }
}
```

### Textarea

For longer text content, use `format: "textarea"`:

```json
{
  "body": { "type": "string", "format": "textarea", "title": "Body" }
}
```

This renders as a multi-line textarea with 6 rows.

### Enum (select dropdown)

Add an `enum` array to create a dropdown:

```json
{
  "category": {
    "type": "string",
    "title": "Category",
    "enum": ["tech", "design", "business"]
  }
}
```

### Upload

Use `format: "upload"` for file fields:

```json
{
  "cover": { "type": "string", "format": "upload", "title": "Cover Image" }
}
```

Upload fields are stored as objects, not strings. See [File Uploads](./uploads.md) for details.

### Markdown

Use `format: "markdown"` for plain markdown editing:

```json
{
  "body": { "type": "string", "format": "markdown", "title": "Body" }
}
```

Renders as a textarea with markdown preview. Stored as a plain markdown string. When fetched via the API with `?render=html`, the markdown is converted to HTML server-side.

### Rich text (markdown-richtext)

Use `format: "markdown-richtext"` for a WYSIWYG rich text editor:

```json
{
  "body": { "type": "string", "format": "markdown-richtext", "title": "Body" }
}
```

Opens a full-screen Milkdown editor with support for headings, lists, images, links, code blocks, and more. Images can be dragged into the editor and are uploaded automatically.

Stored as an object with both representations:

```json
{
  "body": {
    "markdown": "# Hello\n\n![photo](upload:abc123def/photo.jpg)",
    "html": "<h1>Hello</h1>\n<img src=\"upload:abc123def/photo.jpg\">"
  }
}
```

Images in the stored data use the `upload:` URI scheme (e.g. `upload:hash/filename`). When fetched via the API, richtext fields are projected to a plain string and upload URIs are resolved to API paths. See [Content API](./api-content.md#richtext-fields) for details.

## Number fields

```json
{
  "price": { "type": "number", "title": "Price" },
  "quantity": { "type": "integer", "title": "Quantity" }
}
```

`number` allows decimals (`step="any"`), `integer` is whole numbers only (`step="1"`).

## Boolean fields

```json
{
  "published": { "type": "boolean", "title": "Published" }
}
```

Rendered as a checkbox. A hidden input ensures `false` is submitted when unchecked.

## Object fields (nested)

Objects render as a bordered fieldset containing the nested properties:

```json
{
  "author": {
    "type": "object",
    "title": "Author",
    "properties": {
      "name": { "type": "string", "title": "Name" },
      "email": { "type": "string", "title": "Email" }
    }
  }
}
```

Form field names use dot notation: `author.name`, `author.email`.

## Array fields (repeatable)

Arrays render as a list of items with "Add Item" and "Remove" buttons:

```json
{
  "tags": {
    "type": "array",
    "title": "Tags",
    "items": {
      "type": "object",
      "properties": {
        "name": { "type": "string", "title": "Tag Name" }
      }
    }
  }
}
```

Form field names use bracket notation: `tags[0].name`, `tags[1].name`.

Items can be added and removed dynamically in the browser. The form template supports any nesting depth.

## Required fields

Add field names to the `required` array to mark them as mandatory:

```json
{
  "type": "object",
  "properties": {
    "title": { "type": "string", "title": "Title" },
    "body": { "type": "string", "format": "textarea", "title": "Body" }
  },
  "required": ["title"]
}
```

Required fields show an asterisk (*) in the UI and have the HTML `required` attribute.

## Title property

The `title` property on any field controls its label in the UI. If omitted, the property key is used as the label.
