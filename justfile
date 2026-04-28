default:
    @just --list

build:
    cargo build

check:
    cargo check

dev:
    cargo run -- serve

test:
    cargo test

test-integration:
    cargo test --test integration -- --test-threads=4

clippy:
    cargo clippy

fmt:
    cargo fmt

watch:
    bacon

# --- Website ---

# Sync design system CSS + fonts from sibling repo
sync-design:
    rm -rf website/static/css/wavefunk
    cp -r ../design/css website/static/css/wavefunk

# Build the website
site-build: sync-design
    cd website && eigen build

# Dev server with live reload
site-dev: sync-design
    cd website && eigen dev --port 4000

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

# Bundle the Milkdown editor (JS + CSS)
bundle-editor:
    cd editor && npm install && cd ..
    esbuild editor/milkdown-editor.js --bundle --format=iife --minify --outfile=static/js/milkdown-editor.bundle.js
    esbuild editor/milkdown-theme.css --bundle --minify --loader:.woff=empty --loader:.woff2=empty --loader:.ttf=empty --outfile=static/css/milkdown-theme.css
