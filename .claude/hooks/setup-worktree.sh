#!/bin/sh
# Post-worktree setup: install dependencies needed by pre-commit hooks and builds.
# Runs after EnterWorktree to ensure cargo deps, tsc, fixture deps, etc. are available.
set -e

# Find the repo root (works inside worktrees too)
ROOT="$(git rev-parse --show-toplevel 2>/dev/null || echo "")"
[ -z "$ROOT" ] && exit 0

# Cargo — download crate dependencies so builds/checks don't start from scratch
if [ -f "$ROOT/Cargo.lock" ]; then
  echo "Fetching cargo dependencies..." >&2
  (cd "$ROOT" && cargo fetch --quiet) >&2
fi

# runtime/ — needed for `npm run typecheck` pre-commit hook
if [ -d "$ROOT/runtime" ] && [ ! -d "$ROOT/runtime/node_modules" ]; then
  echo "Installing runtime/ dependencies..." >&2
  (cd "$ROOT/runtime" && npm install --no-audit --no-fund) >&2
fi

# fixtures/basic/ — needed for typecheck-fixtures hook and manual testing
if [ -d "$ROOT/fixtures/basic" ] && [ ! -d "$ROOT/fixtures/basic/node_modules" ]; then
  echo "Installing fixtures/basic/ dependencies..." >&2
  (cd "$ROOT/fixtures/basic" && npm install --no-audit --no-fund) >&2
fi
