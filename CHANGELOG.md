# Changelog

## v0.13.0

### New Features

- **Auth rebuild on allowthem**: User accounts, sessions, invitations, and API tokens now run on the `allowthem` library. Existing data migrates on first boot. Bearer-token API auth is app-scoped.
- **Email sending**: Built-in SMTP sender supporting StartTLS, implicit TLS, and unencrypted relays.
- **Password reset**: Self-service password reset flow with email-delivered tokens.
- **Email verification**: Unverified users are hard-blocked at login with a resend flow. Invitation-based signups are implicitly verified.
- **Advanced validation rules**: Schema-level rules (e.g. regex, bounds, cross-field constraints) enforced at publish time, with inline form hints.
- **Content references**: Recursive reference resolution, `label_field` for display, delete warnings when removing referenced entries.
- **Version history**: Version diff route, authorship metadata, API history endpoints, revamped history UI.
- **Bulk operations API**: Five new bulk endpoints for publish / unpublish / delete / update / create.
- **Media management**: Focal-point selection, upload deletion, export bundle fixes.
- **Shared query pipeline**: API pagination, sorting, and field filtering across content endpoints.
- **Admin UX polish**: Status filter, server-side sorting, date filtering, column sorting, schema entry counts, collapsible sidebar, array previews, unsaved-changes warnings, import confirmation.

### Bug Fixes

- Migrator now tolerates migrations applied by allowthem into the shared DB.
- `sort_entries` descending order fixed.
- Single-kind API returns an entry regardless of status.
- Flash messages correctly consumed on edit pages.
- Re-invite after expiry behavior pinned by tests.

### Internal

- 140+ new integration tests covering validation, references, history, bulk ops, media, pagination, and auth.
- Dependency upgrade to sqlx 0.9.

## v0.12.0

### New Features

- **Markdown rendering in the API**: Content fields with `format: "markdown"` can now be served as rendered HTML via the API. Add `?render=html` to any content GET endpoint to receive pre-rendered HTML instead of raw markdown. Rendered output is wrapped in `<div class="sk-markdown">` for easy CSS scoping -- style with `.sk-markdown h1`, `.sk-markdown p`, etc.

- **Schema-level render default**: Set `"render": "html"` in a schema's `x-substrukt` block to serve rendered HTML by default, without requiring `?render=html` on every request. Use `?render=raw` to override back to raw markdown when needed. Ideal for SSG consumers that always want HTML.

- **Enhanced markdown editor**: The EasyMDE editor now includes a full toolbar (bold, italic, strikethrough, headings, code, lists, links, images), side-by-side live preview that stays inline in the form, and full dark mode support matching the CMS theme.

- **Copy-to-clipboard buttons** for API tokens and invite URLs.

- **Formatted timestamps and audit details**: Dates in the admin UI are now human-readable instead of raw ISO strings. Audit log details are displayed as readable key-value pairs.

### Improvements

- Inline validation messages on required form fields when submitting.
- Flash messages added for entry deletion, invitation revocation, and other actions that previously gave no feedback.
- Active sidebar navigation link now has a clear visual indicator.
- Login form supports password manager autofill (`autocomplete` attributes).
- CSRF token expiry now shows a styled error page with guidance instead of plain text.
- Disabled pagination controls and buttons show proper visual states (opacity, cursor).
- Import Bundle button disabled until a file is selected.
- Delete App button uses danger styling.
- Invite URLs are now full absolute URLs.
- Schema validation errors use user-friendly language instead of internal jargon.
- Error pages (404, 403, 429, 500) now show the full sidebar navigation.
- Uploads empty state shows guidance text.
- Long text values in content lists are truncated with ellipsis.
- `title` field is sorted to appear first in content forms.
- Published status shown as Yes/No badges in content lists.

### Accessibility

- Accessible labels added to form controls: schema filter dropdown, app name input, role dropdown, and dark mode toggle.
- Login error messages announced to screen readers via `role="alert"`.

### Bug Fixes

- Fixed `_status` field appearing as an editable form field.
- Fixed sidebar app name not updating after rename (full page reload).
- Fixed navigation context missing on error pages.
