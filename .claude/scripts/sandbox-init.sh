#!/bin/bash
# Initialize dev environment inside a Docker sandbox.
# Run this once when a sandbox is first created — installed tools persist
# across sandbox sessions for the same workspace.
#
# Installs: Rust toolchain, Node.js 22, gh CLI, project dependencies
set -e

echo "==> Initializing sandbox dev environment..."

# ── Rust ─────────────────────────────────────────────────────────────
if ! command -v cargo &>/dev/null; then
  echo "==> Installing Rust toolchain..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  # shellcheck source=/dev/null
  source "$HOME/.cargo/env"
else
  echo "    Rust already installed: $(rustc --version)"
fi

# ── Node.js ──────────────────────────────────────────────────────────
if ! command -v node &>/dev/null; then
  echo "==> Installing Node.js 22..."
  curl -fsSL https://deb.nodesource.com/setup_22.x | sudo bash -
  sudo apt-get install -y --no-install-recommends nodejs
else
  echo "    Node already installed: $(node --version)"
fi

# ── gh CLI ───────────────────────────────────────────────────────────
if ! command -v gh &>/dev/null; then
  echo "==> Installing GitHub CLI..."
  (type -p wget >/dev/null || (sudo apt-get update && sudo apt-get install -y wget)) \
    && sudo mkdir -p -m 755 /etc/apt/keyrings \
    && out=$(mktemp) && wget -nv -O"$out" https://cli.github.com/packages/githubcli-archive-keyring.gpg \
    && cat "$out" | sudo tee /etc/apt/keyrings/githubcli-archive-keyring.gpg > /dev/null \
    && sudo chmod go+r /etc/apt/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" | sudo tee /etc/apt/sources.list.d/github-cli.list > /dev/null \
    && sudo apt-get update \
    && sudo apt-get install -y gh
else
  echo "    gh already installed: $(gh --version | head -1)"
fi

# ── Project dependencies ─────────────────────────────────────────────
echo "==> Fetching project dependencies..."
cargo fetch --quiet 2>/dev/null || true

if [ -d fixtures/basic ]; then
  echo "==> Installing fixture npm dependencies..."
  (cd fixtures/basic && npm install --silent 2>/dev/null) || true
fi

echo ""
echo "==> Sandbox dev environment ready!"
echo "    cargo: $(cargo --version)"
echo "    node:  $(node --version)"
echo "    gh:    $(gh --version | head -1)"
