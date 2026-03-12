# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Substrukt is a schema-driven CMS built in Rust. Users define JSON Schemas, then edit data conforming to those schemas through a web UI. Data is stored as JSON files on disk (with in-memory caching) and served via an API. The CMS also supports file uploads via a custom `upload` type in schemas. User accounts and other project-independent data live in SQLite via sqlx.

## Tech Stack

- **Web framework**: Axum
- **Database**: SQLite via sqlx (for users/auth, not content)
- **Templating**: minijinja (server-side rendering)
- **Frontend**: htmx + twind (tailwind-in-browser alternative)
- **Schema**: JSON Schema (runtime-driven UI generation)
- **Content storage**: JSON files on disk, cached in memory
- **Toolchain**: Rust nightly (2026-01-05), edition 2024

## Build & Dev Commands

```bash
# Enter dev environment (nix + direnv)
direnv allow

# Build
cargo build

# Run
cargo run

# Run tests
cargo test

# Run a single test
cargo test test_name

# Check without building
cargo check

# Format
cargo fmt

# Lint
cargo clippy
```

## Architecture

**Data flow**: JSON Schema → UI form generation → JSON file on disk → served via API

- **Schemas** define the structure of content types. The `upload` type is a custom extension for file uploads.
- **Content** is persisted as JSON files, read into memory for caching, and served from cache.
- **SQLite** (via sqlx) handles only base infrastructure: users, sessions, auth, API tokens. Not content.
- **API access**: All content is also available via a REST API, authenticated with bearer tokens. Users create/manage tokens through the UI; tokens are stored in SQLite.
- **UI** is server-rendered with minijinja templates, enhanced with htmx for interactivity. twind provides styling without a build step.
- **Sync/transfer**: The CMS supports exporting and importing a full project bundle (schemas, data, uploads) so that local changes can be pushed to a cloud-deployed instance. The target workflow is a GitHub Action that syncs content as part of the push/release cycle.

## Working Notes

`NOTES.md` is a scratchpad for things learned while building this project. Read it at the start of each session and update it as you go. Record architectural decisions, code style conventions, bug-fix insights, and anything that would save a future session from repeating mistakes.

Create a branch for each new feature, and make small atomic commits as each small self contained part of the feature is done. Once the feature is complete, merge the branch back into main, keeping commit history.

Add often used commands to the justfile.

## Environment

- Nix flake provides the dev shell (`flake.nix`), direnv activates it automatically
- Private env vars go in `.envrc.private` (gitignored)
- `bacon` and `just` are available in the dev shell
