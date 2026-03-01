#!/bin/bash
# Launch a Claude Code session inside a Docker sandbox.
# Fetches a GitHub PAT from 1Password so sandbox commits use the bot identity.
set -e

IMAGE="rex-sandbox"
NAME="rex"

# Fetch GitHub PAT from 1Password (requires op CLI + auth on the host)
if ! command -v op &>/dev/null; then
  echo "error: 1Password CLI (op) not found. Install it: https://developer.1password.com/docs/cli" >&2
  exit 1
fi

GITHUB_TOKEN="$(op read "op://claude/ClaudeLiminal-GitHub/pat" 2>/dev/null)" || {
  echo "error: failed to read GitHub PAT from 1Password. Run 'op signin' first." >&2
  exit 1
}

exec docker sandbox run --name "$NAME" -t "$IMAGE" \
  -e "GITHUB_TOKEN=$GITHUB_TOKEN" \
  claude .
