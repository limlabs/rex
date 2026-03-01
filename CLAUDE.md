# Rex — Development Guide for Claude

## Agent Workflow — Sandboxed Development

All new agent work (features, bug fixes, refactors) MUST use this workflow. Do not skip steps.

### Launching a Sandboxed Agent

From the main session, use the launch script to create an isolated environment:

```bash
.claude/scripts/launch-sandbox.sh <branch-name> "<task description>"
```

This single command:
1. Creates a git worktree at `.claude/worktrees/<branch-name>` on a new branch
2. Installs all dependencies (cargo fetch, npm install, lefthook)
3. Writes a `.claude/TASK.md` with the task description and workflow instructions
4. Launches a **Docker sandbox** (`docker sandbox run claude`) pointed at the worktree

**Port isolation**: Each Docker sandbox runs in its own microVM with a dedicated network namespace. Agents can bind to any port (3000, 8080, etc.) without conflicting with other agents or the host. This is the primary reason all agent work runs in sandboxes.

### Inside the Sandbox

When an agent starts inside a sandbox, it should:

1. **Bootstrap** (first run only): Run `.claude/scripts/sandbox-init.sh` to install dev tools (Rust, Node.js, gh CLI). Tools persist across sandbox sessions for the same workspace.
2. **Read the task**: Check `.claude/TASK.md` for the work item description
3. **Implement**: Make changes on the worktree branch
4. **Verify**: `cargo check` (zero warnings) + `cargo test` (all pass)
5. **Commit**: Use [Conventional Commits](https://www.conventionalcommits.org/) — enforced by a `commit-msg` hook and required by release-please for changelog/version bumps:
   - `feat: add widget support` — new feature (minor version bump)
   - `fix(router): handle trailing slashes` — bug fix (patch bump)
   - `feat!: redesign config format` — breaking change (major bump)
   - Types: `feat`, `fix`, `chore`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `revert`
6. **Open a PR**:
   ```bash
   git push -u origin HEAD
   gh pr create --title "feat: ..." --body "## Summary\n- ..."
   ```

### After the PR is Opened

From the main session, spawn a lightweight review agent with minimal context:

```bash
claude -p "$(sed 's/{PR_NUMBER}/123/g' .claude/prompts/review.md)"
```

The review agent:
- Fetches the PR diff via `gh pr diff`
- Checks for correctness, best practices, performance, and safety
- Posts a review via `gh pr review` (approve, request changes, or comment)

### Why This Workflow

| Concern | How it's solved |
|---------|----------------|
| Port conflicts | Each sandbox has its own network namespace |
| Dependency pollution | Sandboxes have isolated filesystems |
| Blast radius | Agents can't affect the host or other agents |
| Code quality | Automatic PR review by a separate agent |
| Traceability | Every change is a branch + PR |

---

## Quick Reference

- **Build**: `cargo build`
- **Test**: `cargo test`
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
