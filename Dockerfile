# ── Chef base ────────────────────────────────────────────────────────
FROM rust:1.93-bookworm AS chef
RUN cargo install cargo-chef --locked && \
    curl -fsSL https://github.com/mozilla/sccache/releases/download/v0.14.0/sccache-v0.14.0-x86_64-unknown-linux-musl.tar.gz \
    | tar xz -C /usr/local/bin --strip-components=1 --wildcards '*/sccache'
ENV RUSTC_WRAPPER=/usr/local/bin/sccache
WORKDIR /usr/src/rex

# ── Planner ──────────────────────────────────────────────────────────
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ── Builder ──────────────────────────────────────────────────────────
FROM chef AS builder

# Build dependencies (cargo-chef layer cache when Cargo.lock unchanged;
# sccache provides compilation-unit caching as fallback when layer misses)
COPY --from=planner /usr/src/rex/recipe.json recipe.json
RUN --mount=type=secret,id=ACTIONS_CACHE_URL \
    --mount=type=secret,id=ACTIONS_RUNTIME_TOKEN \
    if [ -f /run/secrets/ACTIONS_CACHE_URL ]; then \
      export SCCACHE_GHA_ENABLED=true \
             ACTIONS_CACHE_URL=$(cat /run/secrets/ACTIONS_CACHE_URL) \
             ACTIONS_RUNTIME_TOKEN=$(cat /run/secrets/ACTIONS_RUNTIME_TOKEN); \
      if ! sccache --show-stats >/dev/null 2>&1; then \
        echo "sccache: GHA cache unavailable, building without cache"; \
        unset SCCACHE_GHA_ENABLED RUSTC_WRAPPER; \
      fi; \
    fi && \
    cargo chef cook --release -p rex_cli --features build --recipe-path recipe.json

# Copy runtime/ (needed by include_str! in rex_server)
COPY runtime/ runtime/

# Copy full source and build two binaries:
#   1. rex-builder: includes `build` feature for `rex build` (used in app-build stage)
#   2. rex:         runtime-only, no bundler/linter/dev (ships in final image)
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/
RUN --mount=type=secret,id=ACTIONS_CACHE_URL \
    --mount=type=secret,id=ACTIONS_RUNTIME_TOKEN \
    if [ -f /run/secrets/ACTIONS_CACHE_URL ]; then \
      export SCCACHE_GHA_ENABLED=true \
             ACTIONS_CACHE_URL=$(cat /run/secrets/ACTIONS_CACHE_URL) \
             ACTIONS_RUNTIME_TOKEN=$(cat /run/secrets/ACTIONS_RUNTIME_TOKEN); \
      if ! sccache --show-stats >/dev/null 2>&1; then \
        echo "sccache: GHA cache unavailable, building without cache"; \
        unset SCCACHE_GHA_ENABLED RUSTC_WRAPPER; \
      fi; \
    fi && \
    cargo build --release --bin rex -p rex_cli --features build && \
    cp target/release/rex /usr/local/bin/rex-builder && \
    cargo build --release --bin rex -p rex_cli --no-default-features

# ── App build (available for downstream `FROM ... AS app-build`) ────
# Users extend this stage to run `rex build` with the full binary,
# then copy only the built assets into a runtime-only final image.
FROM debian:bookworm-slim AS app-build

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/bin/rex-builder /usr/local/bin/rex

# ── Runtime ─────────────────────────────────────────────────────────
# Distroless cc image: glibc + libgcc + ca-certs, no shell, no package manager.
FROM gcr.io/distroless/cc-debian12

COPY --from=builder /usr/src/rex/target/release/rex /usr/local/bin/rex

EXPOSE 3000

ENTRYPOINT ["rex"]
CMD ["start"]
