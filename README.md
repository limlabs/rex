# Rex

A next-generation React framework built on the Next.js API. Write standard React — pages, layouts, server components, server actions — with the same file conventions you already know. Under the hood, Rex replaces the Node.js runtime with a purpose-built Rust engine.

### Why Rex

- **Fast** — Axum HTTP server, pooled V8 isolates for SSR, Rolldown bundler. No cold starts, no single-threaded bottlenecks.
- **Both routers** — Pages Router (`pages/` with `getServerSideProps`) and App Router (`app/` with RSC, layouts, streaming) in one framework.
- **Batteries included** — CSS Modules, Tailwind (auto-detected), MDX, image optimization, Google Fonts, middleware, server actions — no plugins to install.
- **Live mode** — point `rex live` at source directories and serve instantly with on-demand compilation, no build step. Mount multiple projects under different URL prefixes.
- **One CLI** — `rex dev`, `rex build`, `rex start`, `rex live`, plus built-in `lint` (oxlint), `fmt` (oxfmt), and `typecheck` (tsc).
- **Single binary** — ships as one native executable per platform via npm. No Node.js required at runtime.
- **Zero-config** — works without a `package.json`. Add one when you need a lockfile or extra dependencies.
- **Agent-friendly** — fewer files and dependencies means less baseline context for AI coding agents, leaving more of the context window for your actual application logic.

### Performance

Benchmarked against Next.js 16 on the same pages with Apache Bench (10k requests, 100 concurrent, 200 warmup). Clean builds with no cache. Apple M3 Max, 36 GB.

| Metric | Rex | Next.js 16 |
|--------|-----|-----------|
| **SSR throughput** | 20,411 req/s | 4,758 req/s |
| **SSR latency** | 4.9 ms | 21.0 ms |
| **Production build** | 205 ms | 1,789 ms |
| **Dev cold start** | 649 ms | 5,423 ms |
| **Install size** | 114 MB | 369 MB |
| **Install time** | 4,204 ms | 20,426 ms |
| **Client JS** | 196 KB | 547 KB |
| **Lint** | 119 ms (oxlint) | 412 ms (eslint) |

Reproduce: `cd benchmarks && uv run python bench.py --suite dx,server,client --framework rex,nextjs --iterations 1`. Raw data and methodology in [benchmarks/](benchmarks/METHODOLOGY.md).

## Quick Start

```sh
curl -fsSL https://raw.githubusercontent.com/limlabs/rex/main/install.sh | sh
rex init my-app
cd my-app
rex dev
```

Open http://localhost:3000.

## Install

### Shell (recommended)

```sh
curl -fsSL https://raw.githubusercontent.com/limlabs/rex/main/install.sh | sh
```

Downloads the latest binary for your platform (macOS arm64/x64, Linux x64/arm64) to `~/.rex/bin`.

### npm

```sh
npx @limlabs/rex init my-app    # one-shot, no global install
# or
npm install -g @limlabs/rex      # global install
```

### From source

```sh
git clone https://github.com/limlabs/rex.git
cd rex
cargo build --release
# Binary at target/release/rex
```

### Docker

```sh
docker build -t rex .
docker run -v $(pwd):/app -w /app -p 3000:3000 rex
```

## What's Supported

Rex aims for high compatibility with Next.js across both routers. Here's the high-level picture:

### Pages Router

File-system routing in `pages/`, server-side rendering with `getServerSideProps` (sync and async), dynamic routes (`[slug]`, `[...slug]`, `[[...slug]]`), API routes, custom `_app` and `_document`, client-side navigation, and data fetching via JSON endpoints.

### App Router

`app/` directory with nested layouts, React Server Components, `"use client"` boundary, streaming SSR with Suspense, `loading.tsx` / `error.tsx` / `not-found.tsx`, route groups `(group)`, `generateMetadata`, server actions (`"use server"`), and automatic static optimization.

### Shared Features

CSS Modules, Tailwind CSS (auto-detected), MDX pages, Google Fonts optimization, image optimization with blur placeholders, middleware, redirects/rewrites/headers config, TypeScript/TSX, and HMR in dev mode.

For the full feature-by-feature breakdown — including comparison with Next.js and Vinext — see the [Compatibility Matrix](COMPATIBILITY.md).

## CLI Reference

```
rex init <name>                    Create a new Rex project
rex dev  [--port 3000] [--root .]  Dev server with HMR
         [-H host] [--no-tui]
rex build [--root .]               Production build
rex start [--port 3000] [--root .] Serve production build
          [-H host]
rex lint  [--root .] [--fix]       Lint with oxlint
          [--deny-warnings]
rex typecheck [--root .]           Type-check with tsc
rex fmt [--root .] [--check]       Format with oxfmt
rex live -m /=./app                Live mode — serve from source
         [-m /admin=./admin-app]   Mount multiple projects
         [--port 4000] [-H host]
         [--workers 4]
```

All port/host flags also read `$PORT` and `$HOST` environment variables.

Set `RUST_LOG=rex=debug` for verbose logging.

## Configuration

Create `rex.config.json` (or `rex.config.toml`) in your project root:

```json
{
  "redirects": [
    { "source": "/old/:slug", "destination": "/new/:slug", "permanent": true }
  ],
  "rewrites": [
    { "source": "/api/:path*", "destination": "/api/v2/:path*" }
  ],
  "headers": [
    { "source": "/(.*)", "headers": [{ "key": "X-Frame-Options", "value": "DENY" }] }
  ],
  "build": {
    "alias": { "@components": "./src/components" },
    "sourcemap": true
  }
}
```

## How It Works

1. **Route scanning** — walks `pages/` and `app/` directories, builds a trie for URL matching (static > dynamic > catch-all priority)
2. **Rolldown bundling** — OXC parses TSX/JSX and strips TypeScript; Rolldown produces an IIFE server bundle and ESM client bundles with code splitting
3. **V8 SSR** — a pool of V8 isolates (one per thread) evaluates the server bundle, runs data fetching, then renders to HTML
4. **Axum serving** — assembles the HTML document with SSR markup, props/flight data, and `<script type="module">` tags
5. **Client hydration** — React hydrates the server-rendered HTML; the client-side router handles subsequent navigations
6. **HMR** — file watcher triggers incremental rebuilds, V8 isolates reload, WebSocket pushes updates to the browser

## Live Mode

Live mode serves React projects directly from source with zero build step. Rex compiles on the first request, caches the result, and automatically recompiles when source files change.

```sh
# Serve a single project
rex live -m /=./my-app

# Mount multiple projects under different prefixes
rex live -m /=./marketing -m /dashboard=./admin-app -m /docs=./docs-site

# Custom port and worker count
rex live -m /=./my-app --port 8080 --workers 8
```

Each mounted project is fully isolated with its own build cache, V8 isolate pool, and file watcher. Cache invalidation is automatic — Rex checks source file timestamps on every request and the file watcher proactively invalidates the cache when files change.

## Architecture

```
rex/
├── crates/
│   ├── rex_cli/      CLI (dev, build, start, lint, fmt, typecheck, init)
│   ├── rex_core/     Shared types and config
│   ├── rex_router/   File-system scanner + trie matcher
│   ├── rex_build/    Rolldown bundler (server IIFE + client ESM)
│   ├── rex_v8/       V8 isolate pool + SSR engine
│   ├── rex_server/   Axum HTTP server + document assembly
│   ├── rex_dev/      File watcher + HMR WebSocket
│   ├── rex_live/     Live mode server (on-demand compilation)
│   ├── rex_image/    Image optimization + blur placeholders
│   ├── rex_mdx/      MDX compiler
│   ├── rex_napi/     Node.js N-API bindings
│   ├── rex_python/   Python bindings (PyO3)
│   └── rex_e2e/      End-to-end tests
├── runtime/          JS runtime (SSR, hydration, router, HMR client)
└── packages/rex/     npm package (@limlabs/rex)
```

## Deployment

### Docker

```dockerfile
FROM node:20-slim AS deps
WORKDIR /app
COPY package*.json ./
RUN npm ci

FROM ghcr.io/limlabs/rex:latest
WORKDIR /app
COPY --from=deps /app/node_modules ./node_modules
COPY . .
RUN rex build
CMD ["start"]
```

### Railway

See `fixtures/railway/` for a ready-to-deploy example with `railway.toml`.

### Any server

```sh
rex build
rex start --port 8080
```

The production server binds to `0.0.0.0` by default for container compatibility.

## Contributing

```sh
git clone https://github.com/limlabs/rex.git
cd rex
cargo build
cargo test

# Run dev server against the test fixture
cd fixtures/basic && npm install && cd ../..
cargo run -- dev --root fixtures/basic
```

## License

[MIT](LICENSE) — Lim Labs
