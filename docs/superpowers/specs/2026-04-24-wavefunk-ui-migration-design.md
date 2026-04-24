# Wavefunk UI Migration — Design Spec

Replace substrukt's twind-based UI with the wavefunk company design system (`../design`). Ships as a single release.

## Decisions

- **Layout:** Sidebar + main content + modeline statusbar (no topbar)
- **Accent:** Amber `#f59e0b` (override wavefunk's default purple)
- **Styling:** wavefunk.css only — twind removed entirely
- **Fonts:** Self-host both Martian Grotesk and Martian Mono (no CDN)
- **Theme:** Dark by default, user toggle via `data-mode` attribute with localStorage persistence
- **EasyMDE:** Stays as-is, restyling is a separate task
- **Sidebar brand:** "Substrukt" text, wavefunk attribution is a separate task
- **CSS consumption:** Copy files into `static/css/`, not symlinked or submoduled
- **Migration strategy:** Incremental template-by-template (Approach A), single release

## 1. Static Assets & CSS Setup

Copy wavefunk CSS into substrukt:

```
static/
├── css/
│   ├── substrukt.css         # imports wavefunk.css, overrides accent
│   ├── wavefunk.css          # single import point (orchestrates order)
│   ├── 01-tokens.css         # CSS custom properties
│   ├── 02-base.css           # resets + element defaults
│   ├── 03-layout.css         # page scaffolds
│   ├── 04-components.css     # all .wf-* component classes
│   ├── 05-utilities.css      # atomic helpers
│   └── fonts/
│       ├── MartianGrotesk-VF.woff2
│       └── MartianMono-VF.woff2
├── favicon.svg
└── wavefunk.svg
```

`substrukt.css` imports wavefunk.css then overrides the accent tokens:

```css
@import "wavefunk.css";

:root {
  --accent: #f59e0b;
  --accent-ink: #000000;
  --accent-dim: color-mix(in srgb, #f59e0b 55%, black);
  --accent-wash: color-mix(in srgb, #f59e0b 14%, black);
  --accent-hover: color-mix(in srgb, #f59e0b 82%, white);
  --accent-press: color-mix(in srgb, #f59e0b 70%, black);
}

[data-mode="light"] {
  --accent: #d97706;
  --accent-ink: #ffffff;
  --accent-dim: color-mix(in srgb, #d97706 55%, white);
  --accent-wash: color-mix(in srgb, #d97706 10%, white);
  --accent-hover: color-mix(in srgb, #d97706 82%, black);
  --accent-press: color-mix(in srgb, #d97706 70%, white);
}
```

Update `01-tokens.css` to reference self-hosted Martian Mono instead of Google Fonts CDN.

## 2. Base Layout Shell

`base.html` restructured to wavefunk shell with modeline:

```html
<html data-mode="dark">
<head>
  <link rel="stylesheet" href="/static/css/substrukt.css">
  <!-- EasyMDE CSS stays for now -->
  <script src="htmx CDN"></script>
  <script src="EasyMDE CDN"></script>
</head>
<body hx-boost="true" hx-target="#main-content" hx-swap="innerHTML">
  <div class="wf-shell">
    <aside class="wf-sidebar">
      {% include "_nav.html" %}
    </aside>
    <main class="wf-main" id="main-content">
      {% block content %}{% endblock %}
    </main>
    <footer class="wf-modeline">
      <!-- modeline segments -->
    </footer>
  </div>
</body>
</html>
```

Changes from current:
- Remove twind CDN script and twind config/install block
- Remove inline CSS variable definitions (tokens come from wavefunk.css)
- Replace `.dark` class toggle with `data-mode="dark"` / `data-mode="light"` attribute toggle
- Keep localStorage persistence + system preference detection, adapted to `data-mode`
- Keep htmx and EasyMDE scripts
- Keep JS functions: markdown editors, upload zones, form validation, array fields, clipboard

Modeline shows CMS context:
```
[substrukt]  app: my-blog  ●  admin  ──────────────────  sandeep@wavefunk.io
```

Using `.wf-ml-buffer` for app name, `.wf-ml-mode` for role, `.wf-ml-fill` for spacer.

## 3. Sidebar Navigation

`_nav.html` mapped to wavefunk sidebar components:

| Current element | Wavefunk component |
|---|---|
| Brand/logo | `.wf-brand` with "Substrukt" text |
| App selector | `.wf-nav-item` at top level |
| Content types (schemas) | `.wf-nav-section` with collapsible list, `.wf-nav-count` for entry counts |
| Admin section (Users, Audit, Backups) | Separate `.wf-nav-section` |
| Deployments | `.wf-nav-item` under app section |
| Uploads | `.wf-nav-item` under app section |
| User profile + logout | `.wf-user` with `.wf-popover` for menu |
| Theme toggle | `.wf-icon-btn` in sidebar footer |

Active page: `.is-active` on `.wf-nav-item`.
Collapse: localStorage-based, toggling `.is-collapsed` class.

## 4. Auth Pages

Standalone pages (no sidebar shell): login, signup, reset password, forgot password, verify pending, verify result, setup.

- Layout: `.wf-auth-top` for centered form area with Substrukt branding
- Form fields: `.wf-field` wrappers with `.wf-label` + `.wf-input`
- Submit: `.wf-btn.primary`
- Errors: `.wf-alert.err`
- Links: plain anchors styled by base layer

## 5. Content & Data Pages

**Content list:**
- Search/filter → `.wf-filterbar` with `.wf-input` + `.wf-select`
- Bulk actions → `.wf-bulkbar`
- Table → `.wf-table.is-interactive` with `.wf-check` for selection
- Status → `.wf-tag.ok` (published), `.wf-tag.warn` (draft), with `<span class="dot">`
- Pagination → `.wf-pagination`
- New entry → `.wf-btn.primary`

**Content edit:**
- Schema-driven fields → `.wf-field` + `.wf-label` + appropriate input type
- Array fields → repeated `.wf-field` blocks, `.wf-btn.ghost` for add/remove
- File uploads → `.wf-dropzone` with `.is-dragover`
- Markdown → EasyMDE as-is
- Status control → `.wf-tag` + `.wf-btn.ghost` with htmx
- Save/delete → `.wf-btn.primary` / `.wf-btn.danger`
- Validation → `.wf-field.is-error`

**Content history:**
- Version list → `.wf-timeline` with `.wf-timeline-item` per version

**Content diff:**
- Diff display → `.wf-framed` with monospace text, `.wf-ok`/`.wf-err` hints for added/removed

## 6. Remaining Pages

**Schema list:** `.wf-table` with name, field count, entry count. New schema → `.wf-btn.primary`.

**Schema edit:** `.wf-field` wrappers, `.wf-panel` per field definition group.

**App list:** `.wf-card` per app with `.wf-card-title`, `.wf-card-body`. New app → `.wf-btn.primary`.

**App settings:** `.wf-field` + `.wf-input` for config. Delete → `.wf-panel.is-danger` + `.wf-btn.danger`.

**Uploads list:** `.wf-table` with filename, size, date. Filter → `.wf-filterbar`.

**Deployments list:** `.wf-table` per target with status, last deployed. New target → `.wf-btn.primary`.

**Deployment form:** `.wf-field` + `.wf-input` for webhook URL, `.wf-switch` for auto-deploy.

**Profile:** `.wf-field` form for username, email, password change.

**Users:** `.wf-table` with role badges — admin `.wf-tag.err`, editor `.wf-tag.warn`, viewer plain `.wf-tag`.

**Audit log:** `.wf-table` with timestamp, user, action columns.

**Backups:** `.wf-dl` for config, `.wf-table` for history, `.wf-btn` for manual trigger.

**Error page:** `.wf-empty` with `.wf-empty-glyph`, `.wf-empty-title`, `.wf-empty-body`, `.wf-empty-actions`.

**Flash messages (all pages):** Success → `.wf-alert.ok`, error → `.wf-alert.err`, info → `.wf-alert.info`.

## 7. Migration Order

Sequential foundation (each depends on prior):
1. Copy CSS + fonts into `static/css/`, create `substrukt.css` with amber overrides
2. `base.html` — shell structure, wavefunk.css, theme toggle adapted to `data-mode`
3. `_nav.html` — sidebar with wavefunk components
4. `_partial.html` — match new shell, flash messages as `.wf-alert`

Independent page conversions (parallelizable):
5. Auth pages (login, signup, reset, forgot, verify, setup)
6. App pages (list, new, settings)
7. Schema pages (list, edit)
8. Content pages (list, edit, history, diff, status control)
9. Upload page (list)
10. Deployment pages (list, form)
11. Settings pages (profile, users, audit log, backups)
12. Error page

Cleanup:
13. Remove twind — CDN script, config, leftover inline CSS variables
14. Final pass — verify all pages against PORTING.md 14-step checklist

## Out of Scope

- EasyMDE restyling (separate task)
- Wavefunk attribution placement (separate task)
- New features or functional changes — this is a pure UI reskin
