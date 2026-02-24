# Rex — Development Guide for Claude

## Quick Reference

- **Build**: `cargo build` (first build is slow due to V8 compilation)
- **Test**: `cargo test` — 33 tests across rex_router, rex_build, rex_v8, and rex_server
- **Check**: `cargo check` — must be zero warnings
- **Run dev server**: `cargo run -- dev --root fixtures/basic`
- **Verbose logging**: `RUST_LOG=rex=debug cargo run -- dev --root fixtures/basic`
- **Fixture setup**: `cd fixtures/basic && npm install` (only needed once)

## Crate Map

| Crate | Purpose | Key files |
|-------|---------|-----------|
| `rex_core` | Config, shared types | `config.rs`, `route.rs` |
| `rex_router` | File scanner + trie matcher | `scanner.rs`, `matcher.rs` |
| `rex_build` | SWC transforms + bundler | `transform.rs`, `bundler.rs` |
| `rex_v8` | V8 isolate pool + SSR | `ssr_isolate.rs`, `isolate_pool.rs` |
| `rex_server` | Axum handlers + HTML doc | `handlers.rs`, `document.rs`, `server.rs` |
| `rex_dev` | File watcher + HMR WS | `watcher.rs`, `hmr.rs` |
| `rex_cli` | CLI entry, React loader | `main.rs` |

## V8 Crate (v146) Patterns

V8 isolates are `!Send`. Each lives on a dedicated OS thread. Work dispatched via crossbeam channels.

Scope macros:
```rust
v8::scope!(scope, &mut isolate);
v8::scope_with_context!(scope, &mut isolate, &context);
v8::tc_scope!(tc, scope);  // TryCatch
```

Async GSSP functions return promises. Resolve them with:
```rust
self.isolate.perform_microtask_checkpoint();
```

## SWC (latest) Patterns

```rust
let mut program = Program::Module(module);
strip(unresolved_mark, top_level_mark).process(&mut program);
```

Server bundles use `Runtime::Classic` (React.createElement), not automatic JSX runtime.

## React 19 Compatibility

- No UMD builds — use CJS from `react/cjs/react.production.js`
- `renderToString` is in the **legacy** module: `react-dom/cjs/react-dom-server-legacy.browser.production.js`
- CJS modules need `require()` polyfill and `__modules` registry in V8
- Needs Web API polyfills: `MessageChannel`, `setTimeout`, `queueMicrotask`, `TextEncoder`, `TextDecoder`, `performance`

## Server Bundle Format

Pages registered at `globalThis.__rex_pages[routeKey]`. ESM converted to CJS via line-matching (fragile — tracked as REX-4). Two global runtime functions:
- `__rex_render_page(routeKey, propsJson)` → HTML
- `__rex_get_server_side_props(routeKey, contextJson)` → JSON (returns `"__REX_ASYNC__"` for promises)

## Plane Project Tracker

Project **Rex** (`REX`), ID: `bb7d9e34-d888-4548-bdec-016b8e01a12f`

### IDs to avoid re-fetching

| Entity | ID |
|--------|----|
| **Project** | `bb7d9e34-d888-4548-bdec-016b8e01a12f` |
| **State: Backlog** | `cb79eadb-59c3-42c8-b00c-9c6dd2afb92d` |
| **State: Todo** | `9a2c6253-2c12-4f08-93ca-85c9b8188402` |
| **State: In Progress** | `11b53ad5-e64c-46f0-b311-e24ecc102c08` |
| **State: Done** | `0c34df63-44e3-4c15-b385-8ad59a69dd71` |
| **State: Cancelled** | `f30c0c05-121d-4220-9e9c-402f64807212` |
| **Label: bugfix** | `4213c893-0f4f-4f6d-bf74-b16028078c10` |
| **Label: architecture** | `2df292a3-cc91-4ad0-a331-fa33200a9636` |
| **Label: testing** | `e702cf96-599a-4470-947a-5cc4eb215b5b` |
| **Label: feature** | `b4cdf3c7-3e11-4322-8828-8d33d3fb5d1d` |

### Usage tips

- Use IDs directly from the table above — no need to call `list_states` or `list_labels`
- To mark a work item done: `update_work_item(state: "0c34df63-...")`
- Search work items by name with `search_work_items` instead of listing all
- When creating work items, set `priority` (urgent/high/medium/low/none) and `point` (effort)
