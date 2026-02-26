# Development Guide

## Prerequisites

- **Rust** (1.75+): https://rustup.rs
- **Node.js** (18+): only needed for test fixtures that use `react`/`react-dom`
- **clang/llvm**: required by the `v8` crate to build V8 from source
- **jq**: used by Claude Code hooks for auto-formatting (`brew install jq` on macOS)

On macOS, Xcode Command Line Tools provides clang. On Linux, install `clang` and `libstdc++-dev`.

## Building

```sh
cargo build
```

First build takes a while (~5-10 min) because the `v8` crate compiles V8 from source. Subsequent builds are fast.

For release:

```sh
cargo build --release
```

## Running

Use the test fixture to try things locally:

```sh
cd fixtures/basic
npm install react react-dom
cd ../..
cargo run -- dev --root fixtures/basic
```

Then visit http://localhost:3000.

## Testing

```sh
cargo test
```

Tests live alongside their source files in `#[cfg(test)]` modules. Current test coverage:

- **rex_core** (7) — config pattern matching, `rex.config.json` loading/parsing
- **rex_router** (9) — scanner parses filenames into route patterns, matcher resolves URLs with correct priority
- **rex_build** (18) — bundler output structure, CSS modules, integration tests (build → V8 SSR)
- **rex_server** (14) — page/data/API handlers, GSSP props/redirect/notFound, config redirects/rewrites/headers
- **rex_v8** (23) — SSR rendering, GSSP/GSP sync+async, data strategy detection, isolate reload

To run a single crate's tests:

```sh
cargo test -p rex_core
cargo test -p rex_router
cargo test -p rex_build
cargo test -p rex_server
cargo test -p rex_v8
```

### E2E tests

End-to-end tests live in `crates/rex_e2e`. They start a real `rex dev` server and make HTTP requests against it.

Prerequisites:
- `cargo build` (or `cargo build --release`)
- `cd fixtures/basic && npm install`

```sh
cargo test -p rex_e2e -- --ignored --test-threads=1
```

The tests are marked `#[ignore]` so they don't run during `cargo test`. Use `--test-threads=1` because all tests share a single server process.

To use a specific rex binary: `REX_BIN=/path/to/rex cargo test -p rex_e2e -- --ignored`

## Project structure

```
crates/
  rex_core/       Shared types, config, errors
  rex_router/     File-system route scanner + trie matcher
  rex_build/      Rolldown bundler (server + client)
  rex_v8/         V8 isolate pool + SSR engine
  rex_server/     Axum HTTP handlers + HTML document assembly
  rex_dev/        File watcher + HMR WebSocket
  rex_cli/        Binary entry point (dev/build/start/lint/init commands)
  rex_e2e/        E2E tests (spawns real server, HTTP assertions)
runtime/          JS evaluated at runtime (HMR client, entry templates)
packages/rex/     npm package shipped to users (rex/document, rex/link, rex/router)
fixtures/basic/   Minimal test project with pages
```

## Crate dependency graph

```
rex_cli
  ├── rex_dev
  │     ├── rex_server
  │     │     ├── rex_build
  │     │     │     ├── rex_router
  │     │     │     │     └── rex_core
  │     │     │     └── rex_core
  │     │     ├── rex_v8
  │     │     │     └── rex_core
  │     │     ├── rex_router
  │     │     └── rex_core
  │     ├── rex_build
  │     ├── rex_v8
  │     ├── rex_router
  │     └── rex_core
  ├── rex_server
  ├── rex_build
  ├── rex_v8
  ├── rex_router
  └── rex_core
```

## How the request lifecycle works

```
GET /blog/hello-world
  │
  ├─ Axum fallback handler (rex_server/src/handlers.rs)
  │
  ├─ Check rex.config.json redirects → 301/307 if matched
  ├─ Check rex.config.json rewrites → transparently rewrite path
  │
  ├─ Route matching: trie.match_path("/blog/hello-world")
  │   → RouteMatch { pattern: "/blog/:slug", params: { slug: "hello-world" } }
  │
  ├─ Build GSSP context (params, query, headers)
  │
  ├─ isolate_pool.execute(|iso| iso.get_server_side_props(route_key, context_json))
  │   → V8 calls globalThis.__rex_get_server_side_props() in server bundle
  │   → Returns JSON: { props: { slug: "hello-world", title: "..." } }
  │
  ├─ Check result: props → continue, redirect → 301/307, notFound → 404
  │
  ├─ isolate_pool.execute(|iso| iso.render_page(route_key, props_json))
  │   → V8 calls globalThis.__rex_render_page() using React.createElement + renderToString
  │   → Returns HTML string
  │
  ├─ assemble_document(ssr_html, props_json, client_scripts, build_id, is_dev)
  │   → Full HTML with <div id="__rex">{ssr_html}</div>, props JSON, script tags
  │
  └─ 200 OK text/html
```

## Key technical notes

### V8 crate (v146)

V8 isolates are `!Send` — each lives on a dedicated OS thread. The `IsolatePool` dispatches work via crossbeam channels and returns results via tokio oneshot channels.

The v8 crate v146 uses a pinned scope API. Scopes must be stack-pinned:

```rust
v8::scope!(scope, &mut isolate);                              // HandleScope
let scope = &mut v8::ContextScope::new(scope, context);       // ContextScope
v8::tc_scope!(tc, scope);                                     // TryCatch
```

Function callbacks use `&mut v8::PinScope` (not `&mut v8::HandleScope`).

### Server bundle format

The server bundle is a single self-contained IIFE built by rolldown that:
1. Includes V8 polyfills as a banner (MessageChannel, setTimeout, TextEncoder, etc.)
2. Bundles React and ReactDOMServer directly (no separate loading)
3. Registers each page component into `globalThis.__rex_pages[routeKey]`
4. Exposes `globalThis.__rex_render_page(routeKey, propsJson)` → JSON `{ body, head }`
5. Exposes `globalThis.__rex_get_server_side_props(routeKey, contextJson)` → JSON string
6. Exposes `globalThis.__rex_get_static_props(routeKey, contextJson)` → JSON string

### HMR flow

1. `notify` watches `pages/` for changes (debounced 200ms)
2. On change: rescan routes, rebuild bundles, reload V8 isolates
3. `tokio::sync::broadcast` fans out `HmrMessage` to all WebSocket clients
4. Browser-side `hmr_client.js` receives the message and triggers `window.location.reload()`

Full React Fast Refresh (component-level hot reload without page refresh) is not yet implemented.

## Adding a new feature

### New route pattern

Edit `crates/rex_router/src/scanner.rs` (`parse_dynamic_segment`) and `crates/rex_router/src/matcher.rs` (trie insertion/matching). Both have comprehensive test suites.

### New server handler

Edit `crates/rex_server/src/handlers.rs`. Add the route in `crates/rex_server/src/server.rs` in `build_router()`.

### New CLI command

Edit `crates/rex_cli/src/main.rs`. Add a variant to the `Commands` enum and a handler function.

## Git hooks (lefthook)

We use [lefthook](https://github.com/evilmartians/lefthook) for git hooks. Install it once:

```sh
# macOS
brew install lefthook

# or with Go
go install github.com/evilmartians/lefthook@latest
```

Then activate the hooks:

```sh
lefthook install
```

This sets up:
- **pre-commit**: `cargo fmt --check`, `cargo clippy`, `cargo test`
- **pre-push**: `cargo test`

### Code coverage

Install `cargo-llvm-cov` for coverage reports:

```sh
cargo install cargo-llvm-cov
```

Run coverage:

```sh
./scripts/coverage.sh           # default 50% threshold
COVERAGE_THRESHOLD=60 ./scripts/coverage.sh   # custom threshold
cargo llvm-cov --workspace --html   # HTML report in target/llvm-cov/html/
```

## Code style

- No external formatter config — use `cargo fmt`
- Run `cargo clippy` before submitting
- Keep `cargo check` warning-free
- Tests go in `#[cfg(test)] mod tests` next to the code they test
- Avoid unnecessary abstractions — the v8 scope types in particular resist generic helper functions, so inline code with macros where needed

## Debugging

### Verbose logging

```sh
RUST_LOG=rex=debug cargo run -- dev --root fixtures/basic
```

### V8 console output

`console.log`/`warn`/`error` inside V8 route to `tracing` under the `v8::console` target:

```sh
RUST_LOG=v8::console=info cargo run -- dev --root fixtures/basic
```

### Inspecting generated bundles

After `rex build` or `rex dev`, look at:
- `.rex/build/server/server-bundle.js` — the server bundle evaluated in V8
- `.rex/build/client/*.js` — per-page client chunks
- `.rex/build/manifest.json` — route-to-chunk mapping
