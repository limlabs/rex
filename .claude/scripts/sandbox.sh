#!/bin/bash
# Launch a Claude Code session in its own worktree + Docker sandbox.
# Each invocation gets an isolated copy of the repo with a bot GitHub identity.
set -e

IMAGE="rex-sandbox"
ID="$(openssl rand -hex 4)"
WORKTREE_DIR=".claude/worktrees/$ID"
BRANCH="sandbox-$ID"

# Fetch GitHub PAT from 1Password (requires op CLI + auth on the host)
if ! command -v op &>/dev/null; then
  echo "error: 1Password CLI (op) not found. Install it: https://developer.1password.com/docs/cli" >&2
  exit 1
fi

GITHUB_TOKEN="$(op read "op://claude/ClaudeLiminal-GitHub/pat" 2>/dev/null)" || {
  echo "error: failed to read GitHub PAT from 1Password. Run 'op signin' first." >&2
  exit 1
}

# Create worktree for filesystem isolation
mkdir -p .claude/worktrees
git worktree add "$WORKTREE_DIR" -b "$BRANCH" HEAD

# Write token into the worktree for the entrypoint to read
echo "$GITHUB_TOKEN" > "$WORKTREE_DIR/.sandbox-github-token"

cleanup() {
  rm -f "$WORKTREE_DIR/.sandbox-github-token"
  # Remove worktree + branch if no new commits were made
  COMMITS="$(git -C "$WORKTREE_DIR" rev-list HEAD --not main --count 2>/dev/null)" || COMMITS=0
  if [ "$COMMITS" -eq 0 ]; then
    git worktree remove "$WORKTREE_DIR" --force 2>/dev/null || true
    git branch -D "$BRANCH" 2>/dev/null || true
  else
    echo "Worktree kept at $WORKTREE_DIR (branch $BRANCH, $COMMITS commits ahead)"
  fi
}
trap cleanup EXIT

docker sandbox run -t "$IMAGE" claude "$WORKTREE_DIR"
