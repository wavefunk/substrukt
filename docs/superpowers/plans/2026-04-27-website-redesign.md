# Substrukt Website Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild the substrukt landing page and docs as a unified static site using the Wave Funk design system and eigen SSG.

**Architecture:** Eigen project lives in `website/`. Design system CSS is synced from `../design/css/` via justfile. Landing page uses marketing layout patterns; docs use 3-column layout with sidebar. All content is local YAML data with markdown bodies rendered by eigen's `| markdown` filter.

**Tech Stack:** Eigen (Rust SSG), Minijinja templates, Wave Funk CSS design system, pulldown-cmark markdown

**Key Reference Files:**
- Design spec: `docs/superpowers/specs/2026-04-27-website-redesign-design.md`
- Design system marketing template: `/home/nambiar/projects/wavefunk/design/templates/marketing.html` (lines 10-67 contain the `mk-*` layout CSS)
- Design system docs template: `/home/nambiar/projects/wavefunk/design/templates/docs.html` (lines 10-62 contain the `docs-*` layout CSS)
- Design system CSS entry point: `/home/nambiar/projects/wavefunk/design/css/wavefunk.css`
- Substrukt app accent override: `/home/nambiar/projects/wavefunk/substrukt/static/css/substrukt.css` (amber `#f59e0b`)
- Current landing page: `website/index.html`
- mdBook source: `docs/src/*.md` (21 files, ~2100 lines total)
- mdBook structure: `docs/src/SUMMARY.md`

**Important:** The `mk-*` marketing layout classes (`.mk-wrap`, `.mk-hero`, `.mk-hero-grid`, `.mk-hero-inner`, `.mk-hero-eyebrow`, `.mk-hero-stats`, `.mk-sect`, `.mk-sect-head`, `.mk-sect-kicker`, `.mk-sect-title`, `.mk-sect-sub`, `.mk-features`, `.mk-feat`, `.mk-foot`, `.mk-colophon`) and `.docs-*` layout classes (`.docs`, `.docs-side`, `.docs-toc`, `.docs-main`, prose styling) are NOT part of `wavefunk.css`. They live in the design system's template `<style>` blocks. These must be inlined in the `_marketing.html` and `_docs.html` layout templates respectively, matching the design system's own pattern.

**Important:** Eigen's `| markdown` filter (pulldown-cmark) does NOT generate heading IDs. The docs TOC requires a small client-side JS snippet that scans rendered `<h2>` elements, assigns IDs from the text, and builds the "On This Page" nav.

---

### Task 1: Build eigen and set up the website scaffold

**Files:**
- Modify: `justfile` (add website recipes)
- Create: `website/site.toml`
- Create: `website/static/css/substrukt.css`
- Create: `website/templates/_base.html`
- Create: `website/templates/index.html`

This task builds eigen, syncs the design system, and creates the minimal skeleton needed to verify the eigen build pipeline works end-to-end.

- [ ] **Step 0: Create feature branch**

```bash
git checkout -b website-redesign
```

- [ ] **Step 1: Build eigen**

```bash
cd /home/nambiar/projects/wavefunk/eigen && cargo build --release
```

This produces the binary at `/home/nambiar/projects/wavefunk/eigen/target/release/eigen`.

- [ ] **Step 2: Create website directory structure**

```bash
cd /home/nambiar/projects/wavefunk/substrukt
mkdir -p website/templates/_partials
mkdir -p website/templates/docs
mkdir -p website/_data/docs
mkdir -p website/static/css
mkdir -p website/static/images
```

- [ ] **Step 3: Sync the design system CSS**

```bash
cp -r /home/nambiar/projects/wavefunk/design/css /home/nambiar/projects/wavefunk/substrukt/website/static/css/wavefunk
```

- [ ] **Step 4: Create `website/static/css/substrukt.css`**

The accent override matching the actual substrukt app:

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

- [ ] **Step 5: Create `website/site.toml`**

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

- [ ] **Step 6: Create `website/templates/_base.html`**

This is the shared base layout. All pages extend it. It loads the design system CSS and the substrukt accent override.

```html
<!doctype html>
<html lang="en" data-mode="dark">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>{% block title %}{{ site.name }}{% endblock %}</title>
<link rel="stylesheet" href="/css/wavefunk/wavefunk.css">
<link rel="stylesheet" href="/css/substrukt.css">
{% block head %}{% endblock %}
</head>
<body>
{% block body %}{% endblock %}
</body>
</html>
```

- [ ] **Step 7: Create a minimal `website/templates/index.html`**

A smoke-test page to verify eigen builds:

```html
---
data:
  nav:
    file: "nav.yaml"
---
{% extends "_base.html" %}

{% block title %}Substrukt — Schema-driven CMS{% endblock %}

{% block body %}
<div style="padding: 100px 32px; text-align: center;">
  <h1 style="font-family: var(--font-mono); font-size: 48px; font-weight: 800; text-transform: uppercase; color: var(--fg-strong);">Substrukt</h1>
  <p style="color: var(--fg-muted); margin-top: 16px;">Build pipeline works.</p>
</div>
{% endblock %}
```

- [ ] **Step 8: Create `website/_data/nav.yaml`**

```yaml
- label: Docs
  url: /docs/introduction
- label: GitHub
  url: https://github.com/wavefunk/substrukt
  external: true
```

- [ ] **Step 9: Update the root `justfile`**

Add website recipes below the existing ones. The eigen binary path is hardcoded to the sibling repo's release build:

```just
# --- Website ---

# Sync design system CSS + fonts from sibling repo
sync-design:
    rm -rf website/static/css/wavefunk
    cp -r ../design/css website/static/css/wavefunk

# Build the website
site-build: sync-design
    cd website && /home/nambiar/projects/wavefunk/eigen/target/release/eigen build

# Dev server with live reload
site-dev: sync-design
    cd website && /home/nambiar/projects/wavefunk/eigen/target/release/eigen dev --port 4000

# Create a new doc page (usage: just new-doc getting-started "Getting Started" "User Guide")
new-doc slug title section:
    @echo 'slug: {{slug}}' > website/_data/docs/{{slug}}.yaml
    @echo 'title: "{{title}}"' >> website/_data/docs/{{slug}}.yaml
    @echo 'section: "{{section}}"' >> website/_data/docs/{{slug}}.yaml
    @echo 'lede: ""' >> website/_data/docs/{{slug}}.yaml
    @echo 'body: |' >> website/_data/docs/{{slug}}.yaml
    @echo '  # {{title}}' >> website/_data/docs/{{slug}}.yaml
    @printf -- '---\ndata:\n  doc:\n    file: "docs/{{slug}}.yaml"\n  docs_nav:\n    file: "docs-nav.yaml"\n---\n{%% extends "_docs.html" %%}\n' > website/templates/docs/{{slug}}.html
    @echo "Created _data/docs/{{slug}}.yaml and templates/docs/{{slug}}.html"
    @echo "Remember to add the page to _data/docs-nav.yaml"
```

Note: `site-dev` uses port 4000 to avoid conflicting with substrukt's own dev server on 3000.

- [ ] **Step 10: Build and verify**

```bash
cd /home/nambiar/projects/wavefunk/substrukt && just site-build
```

Expected: eigen builds successfully, `website/dist/index.html` exists and contains the smoke-test content with design system CSS linked.

- [ ] **Step 11: Start dev server and check in browser**

```bash
just site-dev
```

Visit `http://localhost:4000`. Verify:
- Page loads with dark background (design system `--bg`)
- "SUBSTRUKT" heading renders in Martian Grotesk mono (if fonts loaded)
- Amber accent color is NOT visible yet (just checking base pipeline)

Stop the dev server after verifying.

- [ ] **Step 12: Commit**

```bash
git add website/site.toml website/static/css/substrukt.css website/templates/_base.html website/templates/index.html website/_data/nav.yaml website/static/css/wavefunk justfile
git commit -m "feat(website): scaffold eigen project with design system sync"
```

---

### Task 2: Landing page — marketing layout and nav

**Files:**
- Create: `website/templates/_marketing.html`
- Create: `website/templates/_partials/nav.html`
- Modify: `website/templates/index.html`

The marketing layout contains the `mk-*` CSS from the design system's `marketing.html` template, adapted with substrukt branding.

- [ ] **Step 1: Create `website/templates/_marketing.html`**

This layout extends `_base.html` and provides the marketing page structure. The `mk-*` CSS is inlined in a `<style>` block, copied from the design system's `marketing.html` template (lines 11-67) and adapted. The content block is where `index.html` puts its sections.

```html
{% extends "_base.html" %}

{% block head %}
<style>
  body { background: var(--bg); }
  .mk-wrap { max-width: 1240px; margin: 0 auto; border-left: 1px solid var(--hairline); border-right: 1px solid var(--hairline); }

  /* Nav */
  .mk-nav { padding: 0 32px; }

  /* Hero */
  .mk-hero { padding: 100px 32px 80px; border-top: 1px solid var(--hairline); position: relative; overflow: hidden; }
  .mk-hero-grid { position: absolute; inset: 0; opacity: 0.4; background-image: linear-gradient(var(--hairline-dim) 1px, transparent 1px), linear-gradient(90deg, var(--hairline-dim) 1px, transparent 1px); background-size: 64px 64px; pointer-events: none; }
  .mk-hero-inner { position: relative; z-index: 1; }
  .mk-hero-eyebrow { font-family: var(--font-mono); font-size: 11px; letter-spacing: 0.18em; text-transform: uppercase; color: var(--accent); margin-bottom: 24px; display: flex; align-items: center; gap: 10px; }
  .mk-hero-eyebrow::before { content: ""; width: 24px; height: 1px; background: var(--accent); }
  .mk-hero h1 { font-family: var(--font-mono); font-size: clamp(56px, 9vw, 112px); font-weight: 800; line-height: 0.95; letter-spacing: -0.03em; text-transform: uppercase; color: var(--fg-strong); margin: 0 0 24px; max-width: 16ch; }
  .mk-hero h1 em { font-style: normal; color: var(--accent); }
  .mk-hero p { font-size: 17px; line-height: 1.55; color: var(--fg-muted); max-width: 54ch; margin: 0 0 32px; }
  .mk-hero-cta { display: flex; gap: 12px; align-items: center; flex-wrap: wrap; }
  .mk-hero-cta .sep { width: 1px; height: 28px; background: var(--hairline); }
  .mk-hero-cta .shell-line { font-family: var(--font-mono); font-size: 12px; color: var(--fg-muted); letter-spacing: 0.04em; }
  .mk-hero-cta .shell-line .prompt { color: var(--fg-faint); margin-right: 6px; }

  /* Stat strip */
  .mk-hero-stats { margin-top: 72px; display: grid; grid-template-columns: repeat(4, 1fr); border-top: 1px solid var(--hairline); position: relative; z-index: 1; }
  .mk-hero-stats > div { padding: 22px 0; border-right: 1px solid var(--hairline); }
  .mk-hero-stats > div:last-child { border-right: 0; }
  .mk-hero-stats .l { font-family: var(--font-mono); font-size: 10px; letter-spacing: 0.14em; text-transform: uppercase; color: var(--fg-faint); margin-bottom: 6px; }
  .mk-hero-stats .v { font-family: var(--font-mono); font-size: 34px; font-weight: 800; color: var(--fg-strong); letter-spacing: -0.02em; }

  /* Sections */
  .mk-sect { padding: 100px 32px; border-top: 1px solid var(--hairline); }
  .mk-sect-head { display: grid; grid-template-columns: 1fr 2fr; gap: 40px; margin-bottom: 48px; align-items: end; }
  .mk-sect-kicker { font-family: var(--font-mono); font-size: 11px; letter-spacing: 0.18em; text-transform: uppercase; color: var(--accent); margin-bottom: 12px; }
  .mk-sect-title { font-family: var(--font-mono); font-size: clamp(32px, 4vw, 44px); font-weight: 800; letter-spacing: -0.02em; text-transform: uppercase; color: var(--fg-strong); margin: 0; line-height: 1; max-width: 14ch; }
  .mk-sect-sub { font-size: 16px; line-height: 1.6; color: var(--fg-muted); max-width: 52ch; margin: 0; }
  @media (max-width: 900px) { .mk-sect-head { grid-template-columns: 1fr; } }

  /* Feature grid */
  .mk-features { display: grid; grid-template-columns: repeat(3, 1fr); gap: 0; border-top: 1px solid var(--hairline); border-left: 1px solid var(--hairline); }
  .mk-feat { padding: 32px; border-right: 1px solid var(--hairline); border-bottom: 1px solid var(--hairline); min-height: 240px; display: flex; flex-direction: column; }
  .mk-feat-num { font-family: var(--font-mono); font-size: 10px; letter-spacing: 0.14em; color: var(--fg-faint); margin-bottom: 40px; }
  .mk-feat-t { font-family: var(--font-mono); font-size: 20px; font-weight: 800; letter-spacing: -0.01em; text-transform: uppercase; color: var(--fg-strong); margin: 0 0 10px; }
  .mk-feat-b { font-size: 13px; line-height: 1.65; color: var(--fg-muted); margin: 0; }
  @media (max-width: 900px) { .mk-features { grid-template-columns: 1fr; } }

  /* How it works grid */
  .mk-steps { display: grid; grid-template-columns: repeat(4, 1fr); gap: 0; border-top: 1px solid var(--hairline); border-left: 1px solid var(--hairline); }
  .mk-step { padding: 32px; border-right: 1px solid var(--hairline); border-bottom: 1px solid var(--hairline); }
  .mk-step-num { font-family: var(--font-mono); font-size: 10px; letter-spacing: 0.14em; color: var(--fg-faint); margin-bottom: 20px; }
  .mk-step-t { font-family: var(--font-mono); font-size: 16px; font-weight: 800; letter-spacing: -0.01em; text-transform: uppercase; color: var(--fg-strong); margin: 0 0 10px; }
  .mk-step-b { font-size: 13px; line-height: 1.65; color: var(--fg-muted); margin: 0; }
  @media (max-width: 900px) { .mk-steps { grid-template-columns: 1fr 1fr; } }
  @media (max-width: 600px) { .mk-steps { grid-template-columns: 1fr; } }

  /* Quick start code */
  .mk-code pre { background: var(--ink-100); border: 1px solid var(--hairline-dim); padding: 20px 24px; font-family: var(--font-mono); font-size: 12px; line-height: 1.7; color: var(--fg); overflow-x: auto; max-width: 720px; }
  .mk-code pre .comment { color: var(--fg-faint); }

  /* Footer */
  .mk-foot { padding: 48px 32px 32px; border-top: 1px solid var(--hairline); display: grid; grid-template-columns: 2fr 1fr 1fr 1fr; gap: 32px; }
  .mk-foot-h { font-family: var(--font-mono); font-size: 10px; letter-spacing: 0.16em; text-transform: uppercase; color: var(--fg-faint); margin-bottom: 14px; }
  .mk-foot a { display: block; font-family: var(--font-mono); font-size: 12px; letter-spacing: 0.04em; color: var(--fg-muted); text-decoration: none; padding: 4px 0; }
  .mk-foot a:hover { color: var(--fg-strong); }
  .mk-colophon { padding: 18px 32px; border-top: 1px solid var(--hairline); display: flex; justify-content: space-between; font-family: var(--font-mono); font-size: 10px; letter-spacing: 0.14em; text-transform: uppercase; color: var(--fg-faint); }
  @media (max-width: 900px) { .mk-foot { grid-template-columns: 1fr 1fr; } }
</style>
{% endblock %}

{% block body %}
<div class="mk-wrap">
  {% include "_partials/nav.html" %}
  {% block content %}{% endblock %}
  {% include "_partials/footer.html" %}
</div>
{% endblock %}
```

- [ ] **Step 2: Create `website/templates/_partials/nav.html`**

The marketing top nav with substrukt branding and amber accent:

```html
<div class="mk-nav">
  <div class="wf-mnav" style="padding: 0;">
    <div class="wf-wordmark">
      <div style="width: 22px; height: 22px; background: var(--accent); color: var(--accent-ink); display: inline-flex; align-items: center; justify-content: center; font-family: var(--font-mono); font-weight: 800; font-size: 13px;">S</div>
      <span class="wf-wordmark-name">SUBSTRUKT</span>
    </div>
    {% for item in nav %}
    {% if item.external %}
    <a href="{{ item.url }}" target="_blank" rel="noopener">{{ item.label }}</a>
    {% else %}
    <a href="{{ item.url }}">{{ item.label }}</a>
    {% endif %}
    {% endfor %}
    <div class="wf-mnav-spacer"></div>
  </div>
</div>
```

- [ ] **Step 3: Update `website/templates/index.html` to extend marketing layout**

Replace the smoke-test content with just the hero for now (full content in Task 3):

```html
---
data:
  nav:
    file: "nav.yaml"
---
{% extends "_marketing.html" %}

{% block title %}Substrukt — Schema-driven CMS built in Rust{% endblock %}

{% block content %}
<section class="mk-hero">
  <div class="mk-hero-grid" aria-hidden="true"></div>
  <div class="mk-hero-inner">
    <div class="mk-hero-eyebrow">SUBSTRUKT · OPEN SOURCE CMS</div>
    <h1>Schema-driven CMS, built in <em>Rust</em>.</h1>
    <p>Define content types with JSON Schema. Edit through a web UI. Store as files. Serve via API.</p>
    <div class="mk-hero-cta">
      <a href="/docs/introduction" class="wf-btn lg primary">Read the docs</a>
      <a href="https://github.com/wavefunk/substrukt" class="wf-btn lg" target="_blank" rel="noopener">GitHub</a>
      <span class="sep"></span>
      <code class="shell-line"><span class="prompt">$</span>docker pull ghcr.io/wavefunk/substrukt</code>
    </div>
  </div>
  <div class="mk-hero-stats">
    <div><div class="l">RUNTIME</div><div class="v">SINGLE BINARY</div></div>
    <div><div class="l">SCHEMA</div><div class="v">JSON SCHEMA</div></div>
    <div><div class="l">DATA</div><div class="v">REST API</div></div>
    <div><div class="l">STORAGE</div><div class="v">FILES ON DISK</div></div>
  </div>
</section>
{% endblock %}
```

- [ ] **Step 4: Create a placeholder `website/templates/_partials/footer.html`**

```html
<footer>
  <div class="mk-colophon">
    <span>&copy; {{ current_year() }} SUBSTRUKT</span>
    <span>OPEN SOURCE · MIT</span>
  </div>
</footer>
```

- [ ] **Step 5: Build and verify in browser**

```bash
just site-build && just site-dev
```

Visit `http://localhost:4000`. Verify:
- Marketing nav with amber "S" wordmark renders
- Hero section with grid background, eyebrow, headline ("Rust" in amber), subtext, CTA buttons
- Stats strip with 4 columns
- Amber accent color (`#f59e0b`) throughout
- Footer colophon at bottom

Stop the dev server.

- [ ] **Step 6: Commit**

```bash
git add website/templates/_marketing.html website/templates/_partials/nav.html website/templates/_partials/footer.html website/templates/index.html
git commit -m "feat(website): marketing layout with nav and hero section"
```

---

### Task 3: Landing page — features, how it works, quick start, footer

**Files:**
- Create: `website/_data/features.yaml`
- Create: `website/_data/how-it-works.yaml`
- Modify: `website/templates/index.html`
- Modify: `website/templates/_partials/footer.html`

- [ ] **Step 1: Create `website/_data/features.yaml`**

Content migrated from the current landing page's feature cards:

```yaml
- title: Schema-driven forms
  body: Define content types with JSON Schema. The UI generates forms automatically from your schema, including nested objects, arrays, and file uploads.

- title: File-based content
  body: Content is stored as JSON files on disk, not in a database. Easy to inspect, version control, and migrate. In-memory caching keeps reads fast.

- title: REST API
  body: Every content type gets full CRUD endpoints. Bearer token auth. Export and import bundles for syncing between environments.

- title: Content-addressed uploads
  body: Files are stored by SHA-256 hash with automatic deduplication. Upload the same file twice, it is stored once.

- title: Single binary
  body: One Rust binary, no runtime dependencies. SQLite for auth. Templates and static assets are served directly. Docker image included.

- title: Observability
  body: Prometheus metrics endpoint, structured logging with tracing, and a separate audit log database tracking all mutations.
```

- [ ] **Step 2: Create `website/_data/how-it-works.yaml`**

```yaml
- title: Define a schema
  body: Create a JSON Schema with an x-substrukt extension that sets the title, slug, and storage mode.

- title: Edit content
  body: The web UI generates a form from your schema. Create, edit, and delete entries through the browser.

- title: Consume via API
  body: Content is served as JSON through the REST API. Use bearer tokens for authentication.

- title: Sync and deploy
  body: Export and import bundles to move content between local and production. Trigger deployment webhooks on changes.
```

- [ ] **Step 3: Update `website/templates/index.html` with all sections**

Replace the hero-only content with the full landing page. This extends `_marketing.html` and loads all data files:

```html
---
data:
  nav:
    file: "nav.yaml"
  features:
    file: "features.yaml"
  steps:
    file: "how-it-works.yaml"
---
{% extends "_marketing.html" %}

{% block title %}Substrukt — Schema-driven CMS built in Rust{% endblock %}

{% block content %}
<!-- HERO -->
<section class="mk-hero">
  <div class="mk-hero-grid" aria-hidden="true"></div>
  <div class="mk-hero-inner">
    <div class="mk-hero-eyebrow">SUBSTRUKT · OPEN SOURCE CMS</div>
    <h1>Schema-driven CMS, built in <em>Rust</em>.</h1>
    <p>Define content types with JSON Schema. Edit through a web UI. Store as files. Serve via API.</p>
    <div class="mk-hero-cta">
      <a href="/docs/introduction" class="wf-btn lg primary">Read the docs</a>
      <a href="https://github.com/wavefunk/substrukt" class="wf-btn lg" target="_blank" rel="noopener">GitHub</a>
      <span class="sep"></span>
      <code class="shell-line"><span class="prompt">$</span>docker pull ghcr.io/wavefunk/substrukt</code>
    </div>
  </div>
  <div class="mk-hero-stats">
    <div><div class="l">RUNTIME</div><div class="v">SINGLE BINARY</div></div>
    <div><div class="l">SCHEMA</div><div class="v">JSON SCHEMA</div></div>
    <div><div class="l">DATA</div><div class="v">REST API</div></div>
    <div><div class="l">STORAGE</div><div class="v">FILES ON DISK</div></div>
  </div>
</section>

<!-- FEATURES -->
<section class="mk-sect">
  <div class="mk-sect-head">
    <div>
      <div class="mk-sect-kicker">— 01 / FEATURES</div>
      <h2 class="mk-sect-title">Everything you need, nothing you don't.</h2>
    </div>
    <p class="mk-sect-sub">A schema-first CMS that stores content as files, serves it over a REST API, and fits in a single binary. No database server. No SaaS lock-in.</p>
  </div>
  <div class="mk-features">
    {% for feat in features %}
    <div class="mk-feat">
      <div class="mk-feat-num">— 0{{ loop.index }}</div>
      <h3 class="mk-feat-t">{{ feat.title }}</h3>
      <p class="mk-feat-b">{{ feat.body }}</p>
    </div>
    {% endfor %}
  </div>
</section>

<!-- HOW IT WORKS -->
<section class="mk-sect">
  <div class="mk-sect-head">
    <div>
      <div class="mk-sect-kicker">— 02 / HOW IT WORKS</div>
      <h2 class="mk-sect-title">Four steps to content.</h2>
    </div>
    <p class="mk-sect-sub">Define your content types, edit them through a generated UI, consume via API, and sync between environments.</p>
  </div>
  <div class="mk-steps">
    {% for step in steps %}
    <div class="mk-step">
      <div class="mk-step-num">— 0{{ loop.index }}</div>
      <h3 class="mk-step-t">{{ step.title }}</h3>
      <p class="mk-step-b">{{ step.body }}</p>
    </div>
    {% endfor %}
  </div>
</section>

<!-- QUICK START -->
<section class="mk-sect mk-code">
  <div class="mk-sect-head">
    <div>
      <div class="mk-sect-kicker">— 03 / QUICK START</div>
      <h2 class="mk-sect-title">Up and running in minutes.</h2>
    </div>
    <p class="mk-sect-sub">Build from source or pull the Docker image. One command to start.</p>
  </div>
  <pre><code><span class="comment"># Build from source</span>
git clone https://github.com/wavefunk/substrukt.git
cd substrukt
cargo build --release
./target/release/substrukt serve

<span class="comment"># Or run with Docker</span>
docker build -t substrukt .
docker run -p 3000:3000 -v substrukt-data:/data substrukt</code></pre>
</section>
{% endblock %}
```

- [ ] **Step 4: Update `website/templates/_partials/footer.html`**

Full footer with column layout:

```html
<footer>
  <div class="mk-foot">
    <div>
      <div class="wf-wordmark" style="margin-bottom: 14px;">
        <div style="width: 20px; height: 20px; background: var(--accent); color: var(--accent-ink); display: inline-flex; align-items: center; justify-content: center; font-family: var(--font-mono); font-weight: 800; font-size: 12px;">S</div>
        <span class="wf-wordmark-name">SUBSTRUKT</span>
      </div>
      <p style="font-size: 12px; color: var(--fg-muted); max-width: 32ch; line-height: 1.55; font-family: var(--font-mono);">A schema-driven CMS built in Rust. Define content with JSON Schema, edit through a web UI, serve via API.</p>
    </div>
    <div>
      <div class="mk-foot-h">RESOURCES</div>
      <a href="/docs/introduction">Documentation</a>
      <a href="/docs/getting-started">Getting Started</a>
      <a href="/docs/api-authentication">API Reference</a>
      <a href="/docs/architecture">Architecture</a>
    </div>
    <div>
      <div class="mk-foot-h">PROJECT</div>
      <a href="https://github.com/wavefunk/substrukt" target="_blank" rel="noopener">GitHub</a>
      <a href="https://github.com/wavefunk/substrukt/blob/main/LICENSE" target="_blank" rel="noopener">License</a>
      <a href="https://github.com/wavefunk/substrukt/issues" target="_blank" rel="noopener">Issues</a>
    </div>
    <div>
      <div class="mk-foot-h">OPERATIONS</div>
      <a href="/docs/deployment">Deployment</a>
      <a href="/docs/security">Security</a>
      <a href="/docs/observability">Observability</a>
    </div>
  </div>
  <div class="mk-colophon">
    <span>&copy; {{ current_year() }} SUBSTRUKT</span>
    <span>OPEN SOURCE · MIT</span>
  </div>
</footer>
```

- [ ] **Step 5: Build and verify in browser**

```bash
just site-build && just site-dev
```

Visit `http://localhost:4000`. Verify all sections:
- Hero with eyebrow, headline, CTA buttons, shell line, stats strip
- Features: 6 cards in 3-column grid
- How it works: 4 steps in 4-column grid
- Quick start: code block with build + docker commands
- Footer: 4 columns (project blurb, resources, project links, operations)
- Colophon at very bottom

Check responsive at 900px and 600px widths — grids should collapse.

- [ ] **Step 6: Commit**

```bash
git add website/_data/features.yaml website/_data/how-it-works.yaml website/templates/index.html website/templates/_partials/footer.html
git commit -m "feat(website): complete landing page with features, how-it-works, quick start"
```

---

### Task 4: Docs layout and sidebar

**Files:**
- Create: `website/templates/_docs.html`
- Create: `website/templates/_partials/docs-sidebar.html`
- Create: `website/templates/_partials/docs-toc.html`
- Create: `website/_data/docs-nav.yaml`
- Create: `website/_data/docs/introduction.yaml`
- Create: `website/templates/docs/introduction.html`

Build the docs layout with one real page (introduction) to validate the full pipeline before migrating all 21 pages.

- [ ] **Step 1: Create `website/_data/docs-nav.yaml`**

The full sidebar navigation structure:

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

- [ ] **Step 2: Create `website/templates/_docs.html`**

The 3-column docs layout. The `docs-*` CSS is inlined, adapted from the design system's `docs.html` template. Includes a small JS snippet at the bottom for TOC generation (scanning h2 elements).

```html
{% extends "_base.html" %}

{% block title %}{{ doc.title }} — Substrukt Docs{% endblock %}

{% block head %}
<style>
  body { background: var(--bg); }
  .docs {
    display: grid;
    grid-template-columns: 240px minmax(0, 1fr) 200px;
    min-height: 100vh;
  }
  .docs-side, .docs-toc {
    position: sticky; top: 0; align-self: start;
    height: 100vh; overflow: auto;
    padding: 28px 0;
  }
  .docs-side { border-right: 1px solid var(--hairline); }
  .docs-toc  { border-left: 1px solid var(--hairline); padding: 28px 20px; }
  .docs-main { padding: 40px 48px 120px; max-width: 80ch; }

  .docs-side-head { padding: 0 20px 16px; border-bottom: 1px solid var(--hairline); margin-bottom: 16px; display: flex; align-items: center; gap: 10px; }
  .docs-side-section { font-size: 10px; letter-spacing: 0.16em; text-transform: uppercase; color: var(--fg-faint); padding: 14px 20px 6px; font-family: var(--font-mono); font-weight: 700; }
  .docs-side a {
    display: block; padding: 4px 20px;
    font-family: var(--font-mono); font-size: 11px; letter-spacing: 0.04em; text-transform: uppercase;
    color: var(--fg-muted); text-decoration: none;
    border-left: 2px solid transparent;
  }
  .docs-side a:hover { color: var(--fg-strong); background: var(--ink-100); }
  .docs-side a.is-active { color: var(--fg-strong); background: var(--ink-100); border-left-color: var(--accent); }

  .docs-toc-label { font-family: var(--font-mono); font-size: 10px; letter-spacing: 0.16em; text-transform: uppercase; color: var(--fg-faint); margin-bottom: 10px; }
  .docs-toc a { display: block; padding: 4px 10px; font-family: var(--font-mono); font-size: 10px; letter-spacing: 0.04em; color: var(--fg-muted); text-decoration: none; border-left: 2px solid transparent; }
  .docs-toc a:hover { color: var(--fg-strong); }
  .docs-toc a.is-active { color: var(--fg-strong); border-left-color: var(--accent); background: var(--ink-100); }

  /* Prose */
  .docs-main > h1 { font-family: var(--font-mono); font-size: 40px; font-weight: 800; text-transform: uppercase; letter-spacing: -0.02em; color: var(--fg-strong); margin: 0 0 12px; }
  .docs-main > .lede { font-size: 16px; color: var(--fg); line-height: 1.65; max-width: 62ch; margin: 0 0 32px; }
  .docs-prose h2 { font-family: var(--font-mono); font-size: 22px; font-weight: 800; text-transform: uppercase; letter-spacing: -0.01em; color: var(--fg-strong); margin: 48px 0 12px; padding-top: 20px; border-top: 1px solid var(--hairline); }
  .docs-prose h3 { font-family: var(--font-mono); font-size: 14px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.08em; color: var(--fg-strong); margin: 28px 0 10px; }
  .docs-prose p, .docs-prose ul > li, .docs-prose ol > li { font-size: 14px; line-height: 1.7; color: var(--fg); }
  .docs-prose p { margin: 0 0 14px; max-width: 68ch; }
  .docs-prose ul, .docs-prose ol { padding-left: 22px; margin: 0 0 14px; }
  .docs-prose pre { background: var(--ink-100); border: 1px solid var(--hairline-dim); padding: 14px 16px; font-family: var(--font-mono); font-size: 12px; line-height: 1.7; color: var(--fg); overflow-x: auto; margin: 0 0 16px; }
  .docs-prose code { font-family: var(--font-mono); font-size: 0.9em; background: var(--ink-100); padding: 1px 5px; border: 1px solid var(--hairline-dim); color: var(--fg-strong); }
  .docs-prose pre code { background: transparent; border: 0; padding: 0; color: inherit; font-size: inherit; }
  .docs-prose table { width: 100%; border-collapse: collapse; margin: 0 0 16px; font-size: 13px; }
  .docs-prose th { font-family: var(--font-mono); font-size: 10px; letter-spacing: 0.12em; text-transform: uppercase; color: var(--fg-faint); text-align: left; padding: 8px 12px; border-bottom: 1px solid var(--hairline); }
  .docs-prose td { padding: 8px 12px; border-bottom: 1px solid var(--hairline-dim); color: var(--fg); vertical-align: top; }

  .docs-foot { display: flex; justify-content: space-between; gap: 16px; border-top: 1px solid var(--hairline); padding-top: 20px; margin-top: 48px; }
  .docs-foot a { text-decoration: none; display: flex; flex-direction: column; padding: 12px 16px; border: 1px solid var(--hairline-dim); flex: 1; }
  .docs-foot a:hover { border-color: var(--accent); }
  .docs-foot .k { font-family: var(--font-mono); font-size: 10px; letter-spacing: 0.12em; text-transform: uppercase; color: var(--fg-faint); }
  .docs-foot .t { font-family: var(--font-mono); font-size: 13px; color: var(--fg-strong); font-weight: 700; margin-top: 4px; }

  @media (max-width: 1100px) { .docs { grid-template-columns: 240px 1fr; } .docs-toc { display: none; } }
  @media (max-width: 800px) { .docs { grid-template-columns: 1fr; } .docs-side { display: none; } }
</style>
{% endblock %}

{% block body %}
<div class="docs">
  {% include "_partials/docs-sidebar.html" %}

  <main class="docs-main">
    <div class="wf-crumbs" style="margin-bottom: 14px;">
      <a href="/">HOME</a><span class="sep">/</span>
      <a href="/docs/introduction">DOCS</a><span class="sep">/</span>
      <span aria-current="page">{{ doc.title | upper }}</span>
    </div>

    <h1>{{ doc.title }}</h1>
    {% if doc.lede %}
    <p class="lede">{{ doc.lede }}</p>
    {% endif %}

    <div class="docs-prose" id="docs-content">
      {{ doc.body | markdown }}
    </div>

    <div class="docs-foot">
      {% if prev_page %}
      <a href="/docs/{{ prev_page.slug }}">
        <span class="k">&larr; PREV</span>
        <span class="t">{{ prev_page.title }}</span>
      </a>
      {% else %}
      <span></span>
      {% endif %}
      {% if next_page %}
      <a href="/docs/{{ next_page.slug }}" style="text-align: right;">
        <span class="k">NEXT &rarr;</span>
        <span class="t">{{ next_page.title }}</span>
      </a>
      {% else %}
      <span></span>
      {% endif %}
    </div>
  </main>

  {% include "_partials/docs-toc.html" %}
</div>

<script>
(function() {
  var content = document.getElementById('docs-content');
  var toc = document.getElementById('docs-toc-links');
  if (!content || !toc) return;
  var headings = content.querySelectorAll('h2');
  headings.forEach(function(h) {
    var id = h.textContent.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/(^-|-$)/g, '');
    h.id = id;
    var a = document.createElement('a');
    a.href = '#' + id;
    a.textContent = h.textContent;
    toc.appendChild(a);
  });
})();
</script>
{% endblock %}
```

Note on prev/next: the `prev_page` and `next_page` variables need to be computed. Since each stub template only loads its own doc data and the full nav, the `_docs.html` template will compute prev/next by iterating `docs_nav` to find the current doc's position. However, minijinja's logic capabilities are limited for this. A simpler approach: add `prev_slug`, `prev_title`, `next_slug`, `next_title` fields directly to each doc YAML file during migration. This avoids complex template logic.

Revised approach for prev/next: each `_data/docs/*.yaml` file includes:

```yaml
prev_slug: ""
prev_title: ""
next_slug: getting-started
next_title: Getting Started
```

And the template reads `doc.prev_slug` / `doc.next_slug` directly.

- [ ] **Step 3: Create `website/templates/_partials/docs-sidebar.html`**

```html
<aside class="docs-side">
  <div class="docs-side-head">
    <div style="width: 20px; height: 20px; background: var(--accent); color: var(--accent-ink); display: inline-flex; align-items: center; justify-content: center; font-family: var(--font-mono); font-weight: 800; font-size: 12px;">S</div>
    <div>
      <div style="font-family: var(--font-mono); font-size: 11px; font-weight: 700; letter-spacing: 0.12em; text-transform: uppercase; color: var(--fg-strong);">Substrukt</div>
      <div style="font-size: 9px; letter-spacing: 0.14em; text-transform: uppercase; color: var(--fg-faint); margin-top: 2px;">Docs</div>
    </div>
  </div>

  {% for section in docs_nav %}
  <div class="docs-side-section">{{ section.section }}</div>
  {% for page in section.pages %}
  <a href="/docs/{{ page.slug }}"{% if page.slug == doc.slug %} class="is-active"{% endif %}>{{ page.title }}</a>
  {% endfor %}
  {% endfor %}
</aside>
```

- [ ] **Step 4: Create `website/templates/_partials/docs-toc.html`**

The TOC sidebar. The `#docs-toc-links` container is populated by the JS snippet in `_docs.html`:

```html
<aside class="docs-toc">
  <div class="docs-toc-label">ON THIS PAGE</div>
  <div id="docs-toc-links"></div>

  <div class="docs-toc-label" style="margin-top: 24px;">ACTIONS</div>
  <a href="https://github.com/wavefunk/substrukt/edit/main/docs/src/{{ doc.slug }}.md" target="_blank" rel="noopener">Edit on GitHub &nearr;</a>
</aside>
```

- [ ] **Step 5: Create `website/_data/docs/introduction.yaml`**

Migrate the introduction.md content. The markdown body is placed in the `body` field using YAML block scalar. Includes prev/next navigation fields:

```yaml
slug: introduction
title: Introduction
section: User Guide
lede: Substrukt is a schema-driven CMS built in Rust.
prev_slug: ""
prev_title: ""
next_slug: getting-started
next_title: Getting Started
body: |
  ## Why Substrukt

  Most CMS options fall into two camps: heavy database-backed systems that require significant infrastructure, or headless CMSes locked behind SaaS platforms. Substrukt takes a different approach:

  - **Schema-first**: Content types are defined as JSON Schema. The UI, validation, and API are all generated from the schema at runtime. No code changes needed to add a new content type.
  - **Files on disk**: Content lives as JSON files in a directory. You can read them, version them in git, or sync them between environments with a tar.gz bundle. SQLite is only used for infrastructure (users, sessions, API tokens).
  - **Single binary**: One Rust binary handles everything -- the web UI, REST API, file storage, and background jobs. No external services required beyond the filesystem.
  - **Minimal frontend**: The UI is server-rendered with htmx for interactivity and twind for styling. No build step, no node_modules, no bundler.

  ## What it does

  1. You create **schemas** that describe your content types (blog posts, settings, pages, etc.)
  2. The CMS generates **forms** from those schemas for editing content through the web UI
  3. Content is saved as **JSON files** on disk and cached in memory for fast reads
  4. A **REST API** serves the content to your frontend, static site generator, or mobile app
  5. **Import/export** bundles let you sync content between local and production environments
  6. **Deployments** trigger webhooks to rebuild your frontend when content changes

  ## Core concepts

  | Concept | Description |
  |---------|-------------|
  | **Schema** | A JSON Schema document with an `x-substrukt` extension that defines a content type |
  | **Content entry** | A JSON object conforming to a schema, stored as a file on disk |
  | **Upload** | A file (image, document, etc.) stored with content-addressed deduplication |
  | **App** | An isolated content space with its own schemas, content, uploads, and deployments |
  | **Bundle** | A tar.gz archive containing all schemas, content, and uploads for syncing |
  | **API token** | A bearer token for authenticating API requests, scoped to a specific app |
  | **Deployment** | A webhook target that fires when content changes, configured per app |
```

- [ ] **Step 6: Create `website/templates/docs/introduction.html`**

The stub template:

```html
---
data:
  doc:
    file: "docs/introduction.yaml"
  docs_nav:
    file: "docs-nav.yaml"
---
{% extends "_docs.html" %}
```

- [ ] **Step 7: Build and verify in browser**

```bash
just site-build && just site-dev
```

Visit `http://localhost:4000/docs/introduction`. Verify:
- 3-column layout renders (sidebar, content, TOC)
- Sidebar shows all 4 sections with page links
- "Introduction" is highlighted with amber left border in sidebar
- Breadcrumbs show HOME / DOCS / INTRODUCTION
- Title "Introduction" renders as h1
- Lede text below title
- Markdown body renders correctly (headings, lists, bold, table)
- TOC "ON THIS PAGE" populates with h2 links (Why Substrukt, What it does, Core concepts)
- "Edit on GitHub" link in TOC
- Next link shows "Getting Started" (prev is empty since it's the first page)
- Responsive: TOC hides at 1100px, sidebar hides at 800px

- [ ] **Step 8: Commit**

```bash
git add website/templates/_docs.html website/templates/_partials/docs-sidebar.html website/templates/_partials/docs-toc.html website/_data/docs-nav.yaml website/_data/docs/introduction.yaml website/templates/docs/introduction.html
git commit -m "feat(website): docs layout with sidebar, TOC, and introduction page"
```

---

### Task 5: Migrate all remaining doc pages

**Files:**
- Create: `website/_data/docs/*.yaml` (20 files)
- Create: `website/templates/docs/*.html` (20 stub files)

This is mechanical migration: for each of the 20 remaining mdBook source files, create a YAML data file and a stub template. The markdown content goes into the `body` field. Each file includes prev/next navigation fields based on the ordering in `docs-nav.yaml`.

The page order (for prev/next links) is:
1. introduction, 2. getting-started, 3. configuration, 4. schemas, 5. field-types, 6. storage-modes, 7. single-vs-collection, 8. content-management, 9. uploads, 10. import-export, 11. webhooks, 12. api-authentication, 13. api-schemas, 14. api-content, 15. api-uploads, 16. api-sync, 17. api-publish, 18. deployment, 19. security, 20. observability, 21. data-directory, 22. architecture

- [ ] **Step 1: Create a migration script**

Create a temporary script that reads each `.md` file from `docs/src/`, extracts the first `# Heading` as the title, strips the `# Heading` line from the body (since the template renders `doc.title` as h1), and writes both the YAML data file and HTML stub. Save as `website/migrate-docs.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

DOCS_SRC="docs/src"
DATA_DIR="website/_data/docs"
TMPL_DIR="website/templates/docs"

# Ordered list of slugs (matches docs-nav.yaml order)
SLUGS=(
  introduction
  getting-started
  configuration
  schemas
  field-types
  storage-modes
  single-vs-collection
  content-management
  uploads
  import-export
  webhooks
  api-authentication
  api-schemas
  api-content
  api-uploads
  api-sync
  api-publish
  deployment
  security
  observability
  data-directory
  architecture
)

# Section mapping
declare -A SECTIONS
SECTIONS[introduction]="User Guide"
SECTIONS[getting-started]="User Guide"
SECTIONS[configuration]="User Guide"
SECTIONS[schemas]="User Guide"
SECTIONS[field-types]="User Guide"
SECTIONS[storage-modes]="User Guide"
SECTIONS[single-vs-collection]="User Guide"
SECTIONS[content-management]="User Guide"
SECTIONS[uploads]="User Guide"
SECTIONS[import-export]="User Guide"
SECTIONS[webhooks]="User Guide"
SECTIONS[api-authentication]="API Reference"
SECTIONS[api-schemas]="API Reference"
SECTIONS[api-content]="API Reference"
SECTIONS[api-uploads]="API Reference"
SECTIONS[api-sync]="API Reference"
SECTIONS[api-publish]="API Reference"
SECTIONS[deployment]="Operations"
SECTIONS[security]="Operations"
SECTIONS[observability]="Operations"
SECTIONS[data-directory]="Reference"
SECTIONS[architecture]="Reference"

# Title mapping (extracted from SUMMARY.md)
declare -A TITLES
TITLES[introduction]="Introduction"
TITLES[getting-started]="Getting Started"
TITLES[configuration]="Configuration"
TITLES[schemas]="Schemas"
TITLES[field-types]="Field Types"
TITLES[storage-modes]="Storage Modes"
TITLES[single-vs-collection]="Single vs Collection"
TITLES[content-management]="Content Management"
TITLES[uploads]="File Uploads"
TITLES[import-export]="Import and Export"
TITLES[webhooks]="Deployments"
TITLES[api-authentication]="Authentication"
TITLES[api-schemas]="Schemas API"
TITLES[api-content]="Content API"
TITLES[api-uploads]="Uploads API"
TITLES[api-sync]="Sync API"
TITLES[api-publish]="Deployments API"
TITLES[deployment]="Deployment"
TITLES[security]="Security"
TITLES[observability]="Observability"
TITLES[data-directory]="Data Directory Layout"
TITLES[architecture]="Architecture"

for i in "${!SLUGS[@]}"; do
  slug="${SLUGS[$i]}"
  title="${TITLES[$slug]}"
  section="${SECTIONS[$slug]}"
  src_file="$DOCS_SRC/${slug}.md"

  # Skip introduction (already migrated in Task 4)
  if [ "$slug" = "introduction" ]; then
    continue
  fi

  if [ ! -f "$src_file" ]; then
    echo "WARNING: $src_file not found, skipping"
    continue
  fi

  # Compute prev/next
  prev_slug=""
  prev_title=""
  next_slug=""
  next_title=""
  if [ "$i" -gt 0 ]; then
    prev_slug="${SLUGS[$((i-1))]}"
    prev_title="${TITLES[$prev_slug]}"
  fi
  if [ "$((i+1))" -lt "${#SLUGS[@]}" ]; then
    next_slug="${SLUGS[$((i+1))]}"
    next_title="${TITLES[$next_slug]}"
  fi

  # Read markdown, strip the first "# Title" line
  body=$(sed '1{/^# /d}' "$src_file")

  # Write YAML data file
  cat > "$DATA_DIR/${slug}.yaml" <<YAMLEOF
slug: ${slug}
title: "${title}"
section: "${section}"
lede: ""
prev_slug: "${prev_slug}"
prev_title: "${prev_title}"
next_slug: "${next_slug}"
next_title: "${next_title}"
body: |
$(echo "$body" | sed 's/^/  /')
YAMLEOF

  # Write stub template
  cat > "$TMPL_DIR/${slug}.html" <<HTMLEOF
---
data:
  doc:
    file: "docs/${slug}.yaml"
  docs_nav:
    file: "docs-nav.yaml"
---
{% extends "_docs.html" %}
HTMLEOF

  echo "Migrated: $slug"
done

echo "Done. ${#SLUGS[@]} pages processed."
```

- [ ] **Step 2: Run the migration script**

```bash
cd /home/nambiar/projects/wavefunk/substrukt
chmod +x website/migrate-docs.sh
bash website/migrate-docs.sh
```

Expected output: 21 lines of "Migrated: ..." (skipping introduction which was done in Task 4), ending with "Done. 22 pages processed."

- [ ] **Step 3: Update introduction.yaml prev/next fields**

The introduction page was created manually in Task 4. Verify its `prev_slug`/`next_slug` fields are correct (prev empty, next is getting-started). Should already be correct.

- [ ] **Step 4: Build and spot-check several pages**

```bash
just site-build && just site-dev
```

Visit and verify each of these pages renders correctly:
- `http://localhost:4000/docs/getting-started` — has code blocks, ordered lists
- `http://localhost:4000/docs/schemas` — has JSON code blocks, tables
- `http://localhost:4000/docs/api-content` — longest page (188 lines), check it renders fully
- `http://localhost:4000/docs/field-types` — has tables with many rows
- `http://localhost:4000/docs/architecture` — check the last page's next link is empty

For each page verify:
- Sidebar highlights the correct active page
- Breadcrumb shows correct section
- Prev/next navigation links work
- TOC generates h2 links
- Markdown renders (headings, code blocks, tables, lists, bold/italic)

- [ ] **Step 5: Remove the migration script**

```bash
rm website/migrate-docs.sh
```

- [ ] **Step 6: Commit**

```bash
git add website/_data/docs/ website/templates/docs/
git commit -m "feat(website): migrate all 21 doc pages from mdBook"
```

---

### Task 6: Final polish and cleanup

**Files:**
- Modify: `website/templates/_docs.html` (breadcrumb section link)
- Modify: `.gitignore`
- Possibly modify: various templates for issues found during review

- [ ] **Step 1: Add website dist to .gitignore**

Check the current `.gitignore` and add `website/dist/` if not already present:

```bash
echo "website/dist/" >> .gitignore
```

- [ ] **Step 2: Copy favicon and images**

```bash
cp website/roundedicon.svg website/static/images/
```

Update `_base.html` to include the favicon:

In `website/templates/_base.html`, add inside `<head>` after the CSS links:

```html
<link rel="icon" type="image/svg+xml" href="/images/roundedicon.svg">
```

- [ ] **Step 3: Full site build and comprehensive review**

```bash
just site-build && just site-dev
```

Walk through the entire site:
1. Landing page (`/`) — all sections, responsive at 900px and 600px
2. Click "Read the docs" — should navigate to `/docs/introduction`
3. Click through every sidebar link — each page should load with correct active state
4. Verify prev/next on first page (no prev), last page (no next), and a middle page (both)
5. Check footer links from landing page point to correct doc pages
6. Check "Edit on GitHub" links in TOC point to correct files
7. Resize to 800px — sidebar should hide on docs pages
8. Resize to 1100px — TOC should hide on docs pages

- [ ] **Step 4: Fix any issues found in review**

Address any rendering issues, broken links, or styling problems found in Step 3.

- [ ] **Step 5: Remove old mdBook config**

The old `docs/book.toml` and `docs/book/` build output can be removed since docs are now served by eigen. Keep `docs/src/` as the markdown source of truth for now (the YAML files reference this content).

Actually, do NOT remove `docs/src/` — it's the source that was migrated. The YAML files contain copies of this content. Keep both for now; cleanup can happen in a follow-up.

Remove only the mdBook build artifacts and config:

```bash
rm -f docs/book.toml
rm -rf docs/book/
```

Update the justfile — remove the old `docs-build` and `docs-serve` recipes and replace them:

In the justfile, replace:
```
# Documentation (requires mdBook: cargo install mdbook)
docs-build:
    mdbook build docs

docs-serve:
    mdbook serve docs --open
```

With nothing (the `site-build` and `site-dev` recipes now handle docs).

- [ ] **Step 6: Final build verification**

```bash
just site-build
```

Verify `website/dist/` contains:
- `index.html` (landing page)
- `docs/introduction/index.html` (or `docs/introduction.html` depending on clean_urls)
- All other doc pages
- `sitemap.xml`
- `css/` directory with bundled, tree-shaken CSS
- `images/` with favicon

- [ ] **Step 7: Commit**

```bash
git add .gitignore website/templates/_base.html website/static/images/ justfile
git add -u docs/  # stages deletion of book.toml
git commit -m "feat(website): final polish, favicon, gitignore, remove mdBook config"
```

---

### Task 7: Feature branch merge

- [ ] **Step 1: Run the full build one final time**

```bash
just site-build
```

Verify no errors.

- [ ] **Step 2: Review all changes**

```bash
git log --oneline main..HEAD
git diff --stat main
```

Review the commit history is clean and atomic.

- [ ] **Step 3: Merge to main**

Per project conventions (branch per feature, merge keeping history):

```bash
git checkout main
git merge --no-ff website-redesign -m "feat: redesign website with Wave Funk design system and eigen SSG"
```
