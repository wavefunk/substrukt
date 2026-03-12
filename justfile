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
