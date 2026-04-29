# Stage 1: Build
FROM rustlang/rust:nightly-slim AS builder

WORKDIR /build

# Install build dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Copy manifests and toolchain for dependency caching
COPY Cargo.toml Cargo.lock rust-toolchain.toml build.rs ./

# Copy templates and static assets early (embedded in release binary)
COPY templates/ templates/
COPY static/ static/

# Create dummy source to cache dependency build
RUN mkdir src && echo "fn main() {}" > src/main.rs && echo "" > src/lib.rs
RUN cargo build --release 2>/dev/null || true
RUN rm -rf src

# Copy actual source and build
COPY src/ src/
COPY migrations/ migrations/
COPY audit_migrations/ audit_migrations/
COPY templates/ templates/
COPY static/ static/
RUN touch src/main.rs src/lib.rs && cargo build --release

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates curl gosu && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --shell /bin/bash --uid 1000 substrukt

COPY --from=builder /build/target/release/substrukt /usr/local/bin/substrukt
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh

WORKDIR /opt/substrukt

# Create data directories writable by substrukt user and group
# (chmod 775 so volume mounts with matching GID also work)
RUN mkdir -p /data/schemas /data/content /data/uploads /data/_history \
    && chown -R substrukt:substrukt /data \
    && chmod -R 775 /data

EXPOSE 3000

HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:3000/healthz || exit 1

VOLUME ["/data"]

ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["substrukt", "serve", "--data-dir", "/data", "--db-path", "/data/substrukt.db", "--port", "3000"]
