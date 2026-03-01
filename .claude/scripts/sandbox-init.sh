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

echo "==> Fetching project dependencies..."

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
