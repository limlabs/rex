# Rex — Development Guide for Claude

## Worktree Workflow

**ALWAYS use a git worktree when making any changes.** Never create branches or make commits directly on the current worktree.

1. At the start of any task that involves code changes, use the `EnterWorktree` tool to create an isolated worktree.
2. All edits, commits, and pushes must happen inside the worktree — never on the main checkout.
3. After finishing, create a PR from the worktree branch.

## Git Identity

At the start of every session, configure the bot identity for commits and pushes. The PAT is stored in 1Password and read via the `op` CLI.

**IMPORTANT: Never embed the PAT value directly in commands or git config.** The token must only live in a credentials file to prevent it from leaking into chat output.

```bash
# Step 1: Read PAT and write to a credentials file (single command to avoid leaking the value)
op read 'op://claude/ClaudeLiminal-GitHub/pat' | xargs -I{} sh -c 'echo "https://x-access-token:{}@github.com" > "$HOME/.rex-git-credentials" && chmod 600 "$HOME/.rex-git-credentials"'

# Step 2: Configure git to use the file-based credential store
git config --local credential.helper "store --file=$HOME/.rex-git-credentials"

# Step 3: Export GH_TOKEN for gh CLI (read from the credentials file)
export GH_TOKEN="$(sed -n 's|https://x-access-token:\(.*\)@github.com|\1|p' "$HOME/.rex-git-credentials")"
```

If `op` is not available or fails, warn the user and continue without bot identity. Do not commit or push until the identity is configured.

When committing, always use the bot author:

```bash
git commit --author="claude-liminal <264858718+claude-liminal@users.noreply.github.com>"
```

When using `gh` CLI (PRs, issues), pass `GH_TOKEN`:

```bash
GH_TOKEN="$GH_TOKEN" gh pr create ...
```

## GitHub Operations

**Prefer MCP tools (`mcp__github__*`) over `gh` CLI** for PRs, issues, and reviews. Use `gh` CLI only for operations not covered by MCP (e.g., pushing code, release creation).

## Conventional Commits

All commits must use [Conventional Commits](https://www.conventionalcommits.org/) — enforced by a `commit-msg` hook and required by release-please for changelog/version bumps:
- `feat: add widget support` — new feature (minor version bump)
- `fix(router): handle trailing slashes` — bug fix (patch bump)
- `feat!: redesign config format` — breaking change (major bump)
- Types: `feat`, `fix`, `chore`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `revert`

## Testing

Rex has four layers of testing. **Every code change should consider which layers need new or updated tests.**

### Layer 1: Unit & Integration Tests (`cargo test`)

Follow the standard Rust convention for test placement:

- **Integration tests** go in `crates/<crate>/tests/*.rs` — these test the crate's public API from the outside. Files under `tests/` are automatically excluded from code coverage reports via `--ignore-filename-regex`.
- **Unit tests** use `#[cfg(test)] mod tests` inline in the source file — only for testing private internals that aren't reachable through the public API.

Prefer integration tests in `tests/` over inline `#[cfg(test)]` modules. This keeps production source files focused, improves coverage accuracy, and follows Rust idioms.

Key test suites by crate:

| Crate | Integration tests | What they cover |
|-------|-------------------|-----------------|
| `rex_build` | `build_tests.rs`, `integration_tests.rs`, `font_tests.rs`, `mdx_tests.rs` | Build pipeline, SSR bundles loaded into V8, Google Fonts, MDX |
| `rex_v8` | `ssr_isolate.rs`, `ssr_url.rs`, `fetch.rs`, `ssr_actions.rs`, `ssr_fs.rs`, `ssr_middleware.rs`, `ssr_crypto.rs`, `fs_sandbox.rs` | V8 isolate pool, page rendering, polyfills, server actions, middleware |
| `rex_server` | inline `#[cfg(test)]` in `handlers/*.rs` | Route handling, GSSP, document assembly, API routes, RSC |
| `rex_router` | inline `#[cfg(test)]` in `scanner.rs`, `matcher.rs` | File scanning, route matching, dynamic segments |

Test helpers live in `crates/<crate>/tests/common/mod.rs` — they provide mock `node_modules`, temp project scaffolding, and minimal React runtimes for V8.

**Coverage ratchet**: CI fails if line coverage drops below the threshold in `.coverage-threshold` (currently **68%**). This value only goes up — when your changes raise coverage, bump the threshold to lock in the gain. Measured with `cargo llvm-cov --workspace --ignore-filename-regex 'tests/'`.

### Layer 2: E2E Tests (`cargo test -p rex_e2e`)

End-to-end tests in `crates/rex_e2e/tests/` build and run the actual `rex` binary against fixture projects:

- **Pages Router** (`lib.rs`): Spawns `rex dev` against `fixtures/basic`, verifies page renders, dynamic routes, GSSP, HMR connectivity. Tests are `#[ignore]`-gated — run with `cargo test -p rex_e2e -- --ignored`.
- **App Router ASO** (`aso_e2e.rs`): Runs `rex build` + `rex start` against `fixtures/app-router`, tests static pre-rendering and caching.
- **RSC** (`rsc_e2e.rs`): Full React Server Components workflow with server actions against `fixtures/app-router`.

The test harness (`rex_e2e/src/lib.rs`) handles binary detection (`REX_BIN` env var or `target/`), free port allocation, TCP health checks, and 30-second startup timeout.

**Before running E2E tests locally**, install fixture dependencies: `cd fixtures/basic && npm install` (and similarly for `fixtures/app-router`).

### Layer 3: Smoke Tests (CI only — post-publish)

After a release publishes npm packages, `.github/workflows/smoke-test.yml` verifies the published artifacts work:

1. Rewrites each fixture's `package.json` to use the published `@limlabs/rex` version (not a local file reference)
2. Waits for the package to appear on the npm registry (up to 5 minutes)
3. Runs `npm install` → `rex build` → `rex start` → `curl http://localhost:3000/` and asserts HTTP 200

**Fixture matrix**: `basic`, `tailwind`, `mcp`, `app-router`, `custom-server`

An additional Railway deployment smoke test verifies the Docker image works in a hosted environment.

### Layer 4: Static Analysis (CI)

Run automatically on every PR:

- `cargo fmt --check` — formatting
- `cargo clippy -- -D warnings` — zero-warning lint
- `npx oxlint --deny-warnings` — JS/TS lint
- `npm run typecheck` + `scripts/typecheck-fixtures.sh` — TypeScript strict mode on runtime/ and fixtures
- `scripts/check-file-length.sh` — 700-line file limit

### Test Fixtures

All live in `fixtures/`. Each is a complete project requiring `npm install`.

| Fixture | Router | Purpose |
|---------|--------|---------|
| `basic` | Pages | Main fixture — index, about, blog/[slug], \_app, \_document |
| `app-router` | App | Server components, client components, layouts |
| `tailwind` | Pages | Tailwind CSS + CSS modules |
| `mdx` | App | MDX compilation |
| `mcp` | Pages | MCP server integration |
| `custom-server` | Pages | Custom Express server wrapper |
| `font` | App | Google Fonts + custom fonts |
| `live` | Pages | Live mode multi-project mounting (app-a, app-b sub-dirs) |

### When to Add Tests

- **New feature or handler** → integration test in `crates/<crate>/tests/` + E2E test if it affects page rendering
- **Bug fix** → regression test at the most specific layer that reproduces the bug
- **New fixture** → add to smoke test matrix in `.github/workflows/smoke-test.yml`
- **Runtime JS change** → TypeScript typecheck coverage is automatic; add E2E assertions if behavior changes
- **Config change** → unit test for parsing + integration test for behavior

## 700-Line Rule

Source files must not exceed **700 lines**. This is enforced by CI. When a file crosses this threshold, it needs to be broken down into smaller, focused modules for better maintainability and testability.

## Quick Reference

- **Build**: `cargo build`
- **Test (unit + integration)**: `cargo test`
- **Test (single crate)**: `cargo test -p rex_build`
- **Test (E2E)**: `cargo test -p rex_e2e -- --ignored`
- **Coverage**: `cargo llvm-cov --workspace --ignore-filename-regex 'tests/'`
- **Check**: `cargo check` — must be zero warnings
- **Run dev server**: `cargo run -- dev --root fixtures/basic`
- **Verbose logging**: `RUST_LOG=rex=debug cargo run -- dev --root fixtures/basic`
- **Fixture setup**: `cd fixtures/basic && npm install`

## Crate Map

| Crate | Purpose | Key files |
|-------|---------|-----------|
| `rex_core` | Config, shared types | `config.rs`, `route.rs` |
| `rex_router` | File scanner + trie matcher | `scanner.rs`, `matcher.rs` |
| `rex_build` | Rolldown bundler (server + client) | `bundler.rs` |
| `rex_v8` | V8 isolate pool + SSR | `ssr_isolate.rs`, `isolate_pool.rs` |
| `rex_server` | Axum handlers + HTML doc | `handlers.rs`, `document.rs`, `server.rs` |
| `rex_dev` | File watcher + HMR WS | `watcher.rs`, `hmr.rs` |
| `rex_cli` | CLI entry | `main.rs` |

## V8 Patterns

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

## Rolldown (Bundler)

- Server bundle: **IIFE** — self-contained, runs in bare V8 with no module loader
- Client bundles: **ESM** with code splitting
- `build_bundles()` is async (rolldown requires it)
- CSS handled separately (scanned from source, copied to output)

## Client-Side Router

File: `runtime/client/router.js`. Global API at `window.__REX_ROUTER` (`push`, `replace`, `prefetch`).

Endpoints:
- `/_rex/router.js` — served from `server.rs` via `include_str!`
- `/_rex/data/{buildId}/{path}.json` — GSSP data for client transitions

## CI/CD

**Repo:** `github.com/limlabs/rex`

### Workflows

| Workflow | Trigger | What it does |
|----------|---------|-------------|
| **CI** (`.github/workflows/ci.yml`) | PR to main, push to main | fmt, clippy, check, oxlint, tests |
| **Release** (`.github/workflows/release.yml`) | GitHub Release created | Builds linux/macOS binaries, pushes Docker to `ghcr.io/limlabs/rex`, publishes npm `@limlabs/rex` |

### Creating a release

1. `git tag v0.1.0 && git push origin v0.1.0`
2. Create a GitHub Release from the tag (or `gh release create v0.1.0`)
3. The release workflow builds binaries, pushes Docker, and publishes to npm

### Docker

```bash
docker build -t rex .
docker run -v $(pwd):/app -w /app -p 3000:3000 rex
```

### Required secrets

| Secret | Where | Purpose |
|--------|-------|---------|
| `GITHUB_TOKEN` | Automatic | GHCR push, release asset upload |
| `NPM_TOKEN` | Manual (`gh secret set NPM_TOKEN`) | npm publish |
| `CARGO_REGISTRY_TOKEN` | Manual (`gh secret set CARGO_REGISTRY_TOKEN`) | crates.io publish |

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
