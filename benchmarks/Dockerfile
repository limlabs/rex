# Reproducible benchmark environment for Rex vs Next.js vs TanStack Start vs Vinext
#
# Build:  docker build -t rex-bench -f benchmarks/Dockerfile .
# Run:    docker run --rm rex-bench
# Custom: docker run --rm rex-bench uv run python bench.py --suite server --framework rex,nextjs
#
# The image builds Rex from source (V8 uses prebuilt binaries, not compiled),
# installs all fixture node_modules, and provides ab, Node 24, uv pre-installed.

FROM rust:1.93-bookworm

# System deps: Apache Bench + Chromium (prebuilt, for Lighthouse)
RUN apt-get update && apt-get install -y --no-install-recommends \
    apache2-utils chromium curl ca-certificates git python3 \
    && rm -rf /var/lib/apt/lists/*

# Tell Lighthouse/chrome-launcher to use the system Chromium
ENV CHROME_PATH=/usr/bin/chromium

# Node.js 24
RUN curl -fsSL https://deb.nodesource.com/setup_24.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

# uv (Python package manager)
RUN curl -LsSf https://astral.sh/uv/install.sh | sh
ENV PATH="/root/.local/bin:$PATH"

# Lighthouse (uses system Chromium via CHROME_PATH)
RUN npm install -g lighthouse

WORKDIR /rex

# Cache Rust deps — copy manifests first
COPY Cargo.toml Cargo.lock ./
COPY crates/rex_core/Cargo.toml crates/rex_core/
COPY crates/rex_router/Cargo.toml crates/rex_router/
COPY crates/rex_build/Cargo.toml crates/rex_build/
COPY crates/rex_v8/Cargo.toml crates/rex_v8/
COPY crates/rex_server/Cargo.toml crates/rex_server/
COPY crates/rex_dev/Cargo.toml crates/rex_dev/
COPY crates/rex_cli/Cargo.toml crates/rex_cli/
COPY crates/rex_e2e/Cargo.toml crates/rex_e2e/

# Stub out lib.rs / main.rs so cargo fetch works
RUN for d in crates/rex_core crates/rex_router crates/rex_build crates/rex_v8 crates/rex_server crates/rex_dev crates/rex_e2e; do \
      mkdir -p "$d/src" && echo "" > "$d/src/lib.rs"; \
    done && \
    mkdir -p crates/rex_cli/src && echo "fn main() {}" > crates/rex_cli/src/main.rs

RUN cargo fetch

# Copy full source and build release
COPY . .
# Touch source files so cargo sees them as newer than the stubs
RUN find crates -name "*.rs" -exec touch {} +
RUN cargo build --release

# Install benchmark fixture node_modules
RUN cd fixtures/basic && npm install
RUN cd fixtures/tailwind && npm install
RUN cd benchmarks/next-basic && npm install
RUN cd benchmarks/next-app-basic && npm install
RUN cd benchmarks/tanstack-basic && npm install
RUN cd benchmarks/next-tailwind && npm install
RUN cd benchmarks/vinext-basic && npm install
RUN cd benchmarks/tanstack-tailwind && npm install

# Install Python benchmark deps
RUN cd benchmarks && uv sync

WORKDIR /rex/benchmarks

# Default: run all suites, write results to results.json
CMD ["uv", "run", "python", "bench.py", "--json", "results.json"]
