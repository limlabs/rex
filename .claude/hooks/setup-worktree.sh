#!/bin/bash
# Bootstrap a new worktree with all dependencies ready to go.
# Runs automatically via the WorktreeCreate hook in settings.json.
set -e

WORKTREE_DIR="${CLAUDE_WORKTREE_DIR:-$(pwd)}"

echo "==> Bootstrapping worktree: $WORKTREE_DIR"

# Install lefthook git hooks
if command -v lefthook &>/dev/null; then
  (cd "$WORKTREE_DIR" && lefthook install 2>/dev/null) || true
fi

# Pre-fetch cargo dependencies
if [ -f "$WORKTREE_DIR/Cargo.lock" ]; then
  echo "==> Fetching cargo dependencies..."
  (cd "$WORKTREE_DIR" && cargo fetch --quiet 2>/dev/null) || true
fi

# Install fixture dependencies
if [ -d "$WORKTREE_DIR/fixtures/basic" ]; then
  echo "==> Installing fixture npm dependencies..."
  (cd "$WORKTREE_DIR/fixtures/basic" && npm install --silent 2>/dev/null) || true
fi

echo "==> Worktree ready: $WORKTREE_DIR"
exit 0
