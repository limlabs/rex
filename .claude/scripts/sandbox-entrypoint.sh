#!/bin/bash
# Sandbox entrypoint — runs once on first boot, then execs the default command.
# Configures GitHub auth + git identity from GITHUB_TOKEN, fetches project deps.
set -e

MARKER="$HOME/.sandbox-initialized"

if [ ! -f "$MARKER" ]; then
  echo "==> Initializing sandbox..."

  # Source cargo env
  [ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"

  # Configure GitHub auth + git identity from PAT (written by sandbox.sh)
  GITHUB_TOKEN=""
  for d in /workspace /app .; do
    if [ -f "$d/.sandbox-github-token" ]; then
      GITHUB_TOKEN="$(cat "$d/.sandbox-github-token")"
      rm -f "$d/.sandbox-github-token"
      break
    fi
  done
  if [ -n "$GITHUB_TOKEN" ]; then
    echo "$GITHUB_TOKEN" | gh auth login --with-token 2>/dev/null || true
    GH_USER="$(gh api user --jq .login 2>/dev/null)" || true
    GH_ID="$(gh api user --jq .id 2>/dev/null)" || true
    if [ -n "$GH_USER" ]; then
      git config --global user.name "$GH_USER"
      GH_EMAIL="${GH_ID}+${GH_USER}@users.noreply.github.com"
      git config --global user.email "$GH_EMAIL"
      echo "==> Git identity: $GH_USER <$GH_EMAIL>"
    fi
  fi

  # Fetch project dependencies
  cargo fetch --quiet 2>/dev/null || true
  if [ -d fixtures/basic ]; then
    (cd fixtures/basic && npm install --silent 2>/dev/null) || true
  fi

  # Install git hooks
  if command -v lefthook &>/dev/null; then
    lefthook install 2>/dev/null || true
  fi

  touch "$MARKER"
  echo "==> Sandbox ready."
fi

# Exec the original entrypoint/command
exec "$@"
