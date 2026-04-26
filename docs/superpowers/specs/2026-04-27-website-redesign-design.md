# Substrukt Website Redesign

Rebuild the substrukt landing page and documentation as a single static site using the Wave Funk design system and the eigen static site generator. Replaces the current hand-rolled `website/index.html` and mdBook-based docs.

## Goals

- Unified visual identity using Wave Funk design system with substrukt's amber accent
- Landing page + docs in one eigen project, one build
- All content as local data (YAML + markdown bodies), no external sources
- Design system synced from `../design`, tree-shaken by eigen's CSS bundler
- Docs content migrated from mdBook markdown, preserving structure and URLs

## Project Structure

```
website/
├── site.toml
├── justfile
├── templates/
│   ├── _base.html                  # shared <head>, font loading, CSS imports
│   ├── _marketing.html             # landing page layout (extends _base)
│   ├── _docs.html                  # docs 3-column layout (extends _base)
│   ├── _partials/
│   │   ├── nav.html                # wf-mnav marketing top nav
│   │   ├── footer.html             # mk-foot shared footer
│   │   ├── docs-sidebar.html       # docs sidebar nav from docs-nav.yaml
│   │   └── docs-toc.html           # on-this-page TOC (h2 anchors)
│   ├── index.html                  # landing page (extends _marketing)
│   └── docs/
│       ├── introduction.html       # stub template → _data/docs/introduction.yaml
│       ├── getting-started.html    # stub template → _data/docs/getting-started.yaml
│       ├── configuration.html
│       ├── schemas.html
│       ├── field-types.html
│       ├── storage-modes.html
│       ├── single-vs-collection.html
│       ├── content-management.html
│       ├── uploads.html
│       ├── import-export.html
│       ├── webhooks.html
│       ├── api-authentication.html
│       ├── api-schemas.html
│       ├── api-content.html
│       ├── api-uploads.html
│       ├── api-sync.html
│       ├── api-publish.html
│       ├── deployment.html
│       ├── security.html
│       ├── observability.html
│       ├── data-directory.html
│       └── architecture.html
├── _data/
│   ├── nav.yaml                    # top nav links (Docs, GitHub)
│   ├── features.yaml               # 6 landing page feature cards
│   ├── how-it-works.yaml           # 4-step walkthrough
│   ├── docs-nav.yaml               # sidebar structure (sections + ordered pages)
│   └── docs/
│       ├── introduction.yaml
│       ├── getting-started.yaml
│       ├── configuration.yaml
│       ├── schemas.yaml
│       ├── field-types.yaml
│       ├── storage-modes.yaml
│       ├── single-vs-collection.yaml
│       ├── content-management.yaml
│       ├── uploads.yaml
│       ├── import-export.yaml
│       ├── webhooks.yaml
│       ├── api-authentication.yaml
│       ├── api-schemas.yaml
│       ├── api-content.yaml
│       ├── api-uploads.yaml
│       ├── api-sync.yaml
│       ├── api-publish.yaml
│       ├── deployment.yaml
│       ├── security.yaml
│       ├── observability.yaml
│       ├── data-directory.yaml
│       └── architecture.yaml
├── static/
│   ├── css/
│   │   ├── wavefunk/               # synced from ../design/css/ via justfile
│   │   │   ├── wavefunk.css
│   │   │   ├── 01-tokens.css
│   │   │   ├── 02-base.css
│   │   │   ├── 03-layout.css
│   │   │   ├── 04-components.css
│   │   │   ├── 05-utilities.css
│   │   │   └── fonts/
│   │   │       ├── MartianGrotesk-VF.woff2
│   │   │       └── MartianMono-VF.woff2
│   │   └── substrukt.css           # accent override + site-specific styles
│   └── images/                     # og image, favicon
└── dist/                           # build output (gitignored)
```

## Accent and Theming

Substrukt's app uses amber `#f59e0b` (dark) / `#d97706` (light). The website must match.

`static/css/substrukt.css`:
```css
:root {
  --accent: #f59e0b;
  --accent-ink: #000000;
}

[data-mode="light"] {
  --accent: #d97706;
  --accent-ink: #ffffff;
}
```

The design system's `color-mix` derivatives (`--accent-dim`, `--accent-wash`, `--accent-hover`, `--accent-press`) compute from `--accent` automatically. Only the two base tokens need overriding.

`_base.html` loads both stylesheets:
```html
<link rel="stylesheet" href="/css/wavefunk/wavefunk.css">
<link rel="stylesheet" href="/css/substrukt.css">
```

Eigen's CSS bundler merges and tree-shakes the output.

## Landing Page (index.html)

Extends `_marketing.html`. Uses marketing template patterns from the design system. Same content as the current site, rebuilt with Wave Funk components.

### Nav

`wf-mnav` with:
- Amber square wordmark with "S" + "SUBSTRUKT"
- Links: Docs, GitHub
- No auth/pricing (open source project)

### Hero

`mk-hero` pattern:
- Eyebrow: `SUBSTRUKT · OPEN SOURCE CMS`
- Headline: `Schema-driven CMS, built in Rust.` with "Rust" in `--accent`
- Subtext: "Define content types with JSON Schema. Edit through a web UI. Store as files. Serve via API."
- CTAs: primary "Read the docs" + ghost "GitHub"
- Shell line: `$ docker pull ghcr.io/wavefunk/substrukt`
- Stats strip: 4 qualitative stats (SINGLE BINARY, JSON SCHEMA, REST API, FILE STORAGE)

### Features

`mk-features` 3-column grid with 6 cards, content from `_data/features.yaml`:

1. Schema-driven forms — JSON Schema drives the entire editing UI
2. File-based content — JSON files on disk, in-memory caching, git-friendly
3. REST API — CRUD endpoints with bearer token auth, scoped per app
4. Content-addressed uploads — SHA-256 deduplication, S3-compatible storage
5. Single binary — One Rust binary, SQLite for auth, no external services
6. Observability — Prometheus metrics, structured logging, audit trail

### How It Works

`mk-showcase` style section with 4 steps from `_data/how-it-works.yaml`:

1. Define schema — write a JSON Schema for your content type
2. Edit content — use the generated web UI to create and manage entries
3. Consume via API — fetch content through the REST API
4. Sync and deploy — export, import, and trigger deployment webhooks

### Quick Start

`mk-sect` with code blocks for Docker pull/run and build-from-source.

### Footer

`mk-foot` with columns:
- Project: blurb about substrukt
- Resources: Docs, Getting Started, API Reference
- Project: GitHub, License, Architecture
- Colophon: copyright + version

## Docs Layout (_docs.html)

Adapts the design system's `docs.html` template. 3-column grid: sidebar (240px), main content (fluid), table of contents (200px).

### Responsive Breakpoints

- Below 1100px: TOC column hides
- Below 800px: sidebar hides

### Sidebar (docs-sidebar.html)

- Header: amber square "S" + "SUBSTRUKT" + "DOCS · v0.1"
- 4 sections rendered from `_data/docs-nav.yaml`:
  - USER GUIDE: Introduction, Getting Started, Configuration, Schemas, Field Types, Storage Modes, Single vs Collection, Content Management, Uploads, Import/Export, Deployments
  - API REFERENCE: Authentication, Schemas API, Content API, Uploads API, Sync API, Deployments API
  - OPERATIONS: Deployment, Security, Observability
  - REFERENCE: Data Directory Layout, Architecture
- Active page highlighted with `is-active` class (accent left border)

### Main Content

- Breadcrumbs via `wf-crumbs`: DOCS / Section / Page Title
- Page title as `h1`
- Optional lede paragraph
- Body rendered via `{{ doc.body | markdown }}`
- Prev/next navigation at bottom via `docs-foot` pattern, order from `docs-nav.yaml`

### Table of Contents (docs-toc.html)

- "ON THIS PAGE" label
- Links to h2 headings
- "Edit on GitHub" link

## Doc Page Data Model

Each doc page has a YAML file in `_data/docs/` and a stub template in `templates/docs/`.

### YAML file (`_data/docs/getting-started.yaml`)

```yaml
slug: getting-started
title: Getting Started
section: User Guide
lede: Build and run substrukt in under five minutes.
body: |
  ## Prerequisites

  You need Rust nightly (2026-01-05 or later) and SQLite...

  ## Build from source

  ```bash
  git clone https://github.com/wavefunk/substrukt
  cd substrukt
  cargo build --release
  ```
```

Fields:
- `slug`: URL path segment, matches the template filename
- `title`: page heading and sidebar label
- `section`: which sidebar section this page belongs to
- `lede`: optional intro paragraph displayed below the title
- `body`: markdown content rendered via `| markdown` filter

### Stub template (`templates/docs/getting-started.html`)

```html
---
data:
  doc:
    file: "docs/getting-started.yaml"
  docs_nav:
    file: "docs-nav.yaml"
---
{% extends "_docs.html" %}
```

Every stub is identical except for the `file` path. The `_docs.html` layout handles all rendering.

## Sidebar Navigation Data (`_data/docs-nav.yaml`)

```yaml
- section: User Guide
  pages:
    - slug: introduction
      title: Introduction
    - slug: getting-started
      title: Getting Started
    - slug: configuration
      title: Configuration
    - slug: schemas
      title: Schemas
    - slug: field-types
      title: Field Types
    - slug: storage-modes
      title: Storage Modes
    - slug: single-vs-collection
      title: Single vs Collection
    - slug: content-management
      title: Content Management
    - slug: uploads
      title: File Uploads
    - slug: import-export
      title: Import and Export
    - slug: webhooks
      title: Deployments

- section: API Reference
  pages:
    - slug: api-authentication
      title: Authentication
    - slug: api-schemas
      title: Schemas API
    - slug: api-content
      title: Content API
    - slug: api-uploads
      title: Uploads API
    - slug: api-sync
      title: Sync API
    - slug: api-publish
      title: Deployments API

- section: Operations
  pages:
    - slug: deployment
      title: Deployment
    - slug: security
      title: Security
    - slug: observability
      title: Observability

- section: Reference
  pages:
    - slug: data-directory
      title: Data Directory Layout
    - slug: architecture
      title: Architecture
```

## Eigen Configuration (`site.toml`)

```toml
[site]
name = "Substrukt"
base_url = "https://substrukt.dev"

[build]
fragments = false
minify = true
clean_urls = true

[build.bundling]
enabled = true
css = true
tree_shake_css = true

[sitemap]
enabled = true
```

No remote sources. No fragments (static site, no htmx). CSS bundling with tree-shaking enabled to strip unused design system selectors.

## Build Tooling (`justfile`)

```just
# Sync design system CSS + fonts from sibling repo
sync-design:
    rm -rf website/static/css/wavefunk
    cp -r ../design/css website/static/css/wavefunk

# Build the site
build: sync-design
    cd website && eigen build

# Dev server with live reload
dev: sync-design
    cd website && eigen dev --port 3000

# Create a new doc page
new-doc slug title section:
    @echo "slug: {{slug}}" > website/_data/docs/{{slug}}.yaml
    @echo "title: {{title}}" >> website/_data/docs/{{slug}}.yaml
    @echo "section: {{section}}" >> website/_data/docs/{{slug}}.yaml
    @echo "lede: " >> website/_data/docs/{{slug}}.yaml
    @echo "body: |" >> website/_data/docs/{{slug}}.yaml
    @echo "  # {{title}}" >> website/_data/docs/{{slug}}.yaml
    @printf -- "---\ndata:\n  doc:\n    file: \"docs/{{slug}}.yaml\"\n  docs_nav:\n    file: \"docs-nav.yaml\"\n---\n{%% extends \"_docs.html\" %%}\n" > website/templates/docs/{{slug}}.html
    @echo "Created _data/docs/{{slug}}.yaml and templates/docs/{{slug}}.html"
    @echo "Remember to add the page to _data/docs-nav.yaml"
```

`sync-design` runs before every build/dev. `new-doc` scaffolds both files.

## Design System Drift

The design system is copied from `../design/css/` into `website/static/css/wavefunk/` via the `sync-design` justfile recipe. When the design system updates:

1. Run `just sync-design`
2. Review the diff in git
3. Commit

The copy is checked into git so the site is self-contained and deployable without the sibling repo.

## Migration Plan

Content is migrated from the existing mdBook source at `docs/src/*.md`:

1. Each markdown file becomes a YAML file in `_data/docs/` with the markdown body in the `body` field
2. The SUMMARY.md structure maps directly to `docs-nav.yaml`
3. Landing page content is extracted from the current `website/index.html` into YAML data files
4. No content rewriting — migrate as-is, improve later

## What This Does NOT Include

- Light/dark mode toggle UI (ships dark-only, matching the app)
- Search functionality
- Blog or changelog section
- Analytics integration
- Custom domain setup
