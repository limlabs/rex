#!/bin/bash
# Launch an agent in an isolated Docker sandbox with a dedicated worktree.
#
# Usage:
#   .claude/scripts/launch-sandbox.sh <branch-name> "<task description>"
#
# What this does:
#   1. Creates a git worktree on a new branch
#   2. Installs all project dependencies
#   3. Writes a TASK.md so the sandbox agent knows what to do
#   4. Launches a Docker sandbox (isolated network — no port conflicts)
#
# The sandbox agent gets its own network namespace, so it can freely bind
# to any port (3000, 8080, etc.) without colliding with other agents or the host.

set -e

BRANCH_NAME="${1:?Usage: launch-sandbox.sh <branch-name> \"<task description>\"}"
TASK_DESC="${2:-}"

PROJECT_ROOT="$(git rev-parse --show-toplevel)"
WORKTREE_DIR="$PROJECT_ROOT/.claude/worktrees/$BRANCH_NAME"

# ── 1. Create worktree ──────────────────────────────────────────────
if [ -d "$WORKTREE_DIR" ]; then
  echo "Worktree already exists: $WORKTREE_DIR"
  echo "Launching sandbox for existing worktree..."
else
  echo "==> Creating worktree: $WORKTREE_DIR (branch: $BRANCH_NAME)"
  git worktree add -b "$BRANCH_NAME" "$WORKTREE_DIR"
fi

# ── 2. Install dependencies ─────────────────────────────────────────
echo "==> Installing dependencies..."

if command -v lefthook &>/dev/null; then
  (cd "$WORKTREE_DIR" && lefthook install 2>/dev/null) || true
fi

if [ -f "$WORKTREE_DIR/Cargo.lock" ]; then
  echo "    cargo fetch..."
  (cd "$WORKTREE_DIR" && cargo fetch --quiet 2>/dev/null) || true
fi

if [ -d "$WORKTREE_DIR/fixtures/basic" ]; then
  echo "    npm install (fixtures)..."
  (cd "$WORKTREE_DIR/fixtures/basic" && npm install --silent 2>/dev/null) || true
fi

# ── 3. Write task file ──────────────────────────────────────────────
if [ -n "$TASK_DESC" ]; then
  mkdir -p "$WORKTREE_DIR/.claude"
  cat > "$WORKTREE_DIR/.claude/TASK.md" <<TASKEOF
# Task

$TASK_DESC

## Workflow

You are working in an isolated Docker sandbox on branch \`$BRANCH_NAME\`.

1. **Bootstrap**: If dev tools are missing, run \`.claude/scripts/sandbox-init.sh\`
2. **Implement**: Make changes on this branch
3. **Verify**: \`cargo check\` (zero warnings) and \`cargo test\` (all pass)
4. **Commit**: Use Conventional Commits (enforced by commit-msg hook, required by release-please):
   - \`feat: ...\` for new features, \`fix: ...\` for bug fixes, \`chore: ...\` for maintenance
   - Types: feat, fix, chore, docs, style, refactor, perf, test, build, ci, revert
5. **PR**: Push and open a pull request:
   \`\`\`bash
   git push -u origin HEAD
   gh pr create --title "..." --body "## Summary\n..."
   \`\`\`

Port isolation is handled by the sandbox — use any ports freely.
TASKEOF
  echo "==> Task written to .claude/TASK.md"
fi

# ── 4. Launch Docker sandbox ────────────────────────────────────────
echo "==> Launching Docker sandbox for: $WORKTREE_DIR"
echo "    (Each sandbox has its own network — no port conflicts)"
echo ""
docker sandbox run claude "$WORKTREE_DIR"
