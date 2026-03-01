#!/bin/bash
# Fetch project dependencies inside the Docker sandbox.
# Safe to run on every session — skips if already done.
#
# Dev tools (Rust, Node, gh, lefthook) are pre-installed via Dockerfile.sandbox.
# This script only handles project-level dependencies.
set -e

MARKER="$HOME/.sandbox-deps-fetched"

if [ -f "$MARKER" ]; then
  exit 0
fi

echo "==> Initializing sandbox..."

# Configure GitHub auth + git identity from PAT (injected by sandbox.sh)
if [ -n "$GITHUB_TOKEN" ]; then
  echo "$GITHUB_TOKEN" | gh auth login --with-token 2>/dev/null || true
  GH_USER="$(gh api user --jq .login 2>/dev/null)" || true
  GH_ID="$(gh api user --jq .id 2>/dev/null)" || true
  GH_EMAIL="${GH_ID}+${GH_USER}@users.noreply.github.com"
  if [ -n "$GH_USER" ]; then
    git config --global user.name "$GH_USER"
    echo "==> Git user.name: $GH_USER"
  fi
  if [ -n "$GH_EMAIL" ]; then
    git config --global user.email "$GH_EMAIL"
    echo "==> Git user.email: $GH_EMAIL"
  fi
fi

# Source cargo env in case shell hasn't picked it up yet
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"

cargo fetch --quiet 2>/dev/null || true

if [ -d fixtures/basic ]; then
  (cd fixtures/basic && npm install --silent 2>/dev/null) || true
fi

# Install git hooks
if command -v lefthook &>/dev/null; then
  lefthook install 2>/dev/null || true
fi

touch "$MARKER"
echo "==> Dependencies ready."
