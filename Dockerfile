# ── Builder ──────────────────────────────────────────────────────────
FROM rust:1.93-bookworm AS builder

WORKDIR /usr/src/rex

# Cache dependency builds: copy manifests first, then fetch
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/rex_cli/Cargo.toml crates/rex_cli/Cargo.toml
COPY crates/rex_core/Cargo.toml crates/rex_core/Cargo.toml
COPY crates/rex_router/Cargo.toml crates/rex_router/Cargo.toml
COPY crates/rex_build/Cargo.toml crates/rex_build/Cargo.toml
COPY crates/rex_v8/Cargo.toml crates/rex_v8/Cargo.toml
COPY crates/rex_server/Cargo.toml crates/rex_server/Cargo.toml
COPY crates/rex_dev/Cargo.toml crates/rex_dev/Cargo.toml
COPY crates/rex_image/Cargo.toml crates/rex_image/Cargo.toml
COPY crates/rex_e2e/Cargo.toml crates/rex_e2e/Cargo.toml
COPY crates/rex_napi/Cargo.toml crates/rex_napi/Cargo.toml
COPY crates/rex_python/Cargo.toml crates/rex_python/Cargo.toml
COPY crates/rex_mdx/Cargo.toml crates/rex_mdx/Cargo.toml

# Create dummy src files so cargo fetch/build can resolve the workspace
RUN for dir in crates/rex_cli crates/rex_core crates/rex_router crates/rex_build \
    crates/rex_v8 crates/rex_server crates/rex_dev crates/rex_image crates/rex_e2e \
    crates/rex_napi crates/rex_python crates/rex_mdx; do \
    mkdir -p "$dir/src" && echo "" > "$dir/src/lib.rs"; \
    done && \
    mkdir -p crates/rex_cli/src && echo "fn main() {}" > crates/rex_cli/src/main.rs

RUN cargo fetch

# Copy runtime/ (needed by include_str! in rex_server)
COPY runtime/ runtime/

# Copy full source and build two binaries:
#   1. rex-builder: includes `build` feature for `rex build` (used in app-build stage)
#   2. rex:         runtime-only, no bundler/linter/dev (ships in final image)
COPY crates/ crates/
RUN cargo build --release --bin rex -p rex_cli --features build && \
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
