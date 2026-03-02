#!/bin/bash
# Runs tsc --noEmit (strict mode) on each Rex fixture that has a tsconfig.json.
# Skips nextjs-app-router (third-party reference implementation).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
FIXTURES_DIR="$ROOT/fixtures"

SKIP=("nextjs-app-router")
FAILED=()

# Ensure packages/rex has its dependencies installed (fixtures resolve
# rex source files via path aliases and need @types/react available).
REX_PKG="$ROOT/packages/rex"
if [[ ! -d "$REX_PKG/node_modules" ]]; then
  echo "--- Installing packages/rex dependencies ---"
  (cd "$REX_PKG" && npm install --ignore-scripts --no-audit --no-fund)
fi

for dir in "$FIXTURES_DIR"/*/; do
  name="$(basename "$dir")"

  # Skip excluded fixtures
  for skip in "${SKIP[@]}"; do
    if [[ "$name" == "$skip" ]]; then
      continue 2
    fi
  done

  # Only check fixtures that have a tsconfig.json
  if [[ ! -f "$dir/tsconfig.json" ]]; then
    continue
  fi

  echo "--- Checking $name ---"

  # Install deps if node_modules is missing
  if [[ ! -d "$dir/node_modules" ]]; then
    echo "  Installing dependencies..."
    (cd "$dir" && npm install --ignore-scripts --no-audit --no-fund) || {
      echo "  WARN: npm install failed for $name, skipping"
      continue
    }
  fi

  # Run tsc --noEmit
  if (cd "$dir" && npx tsc --noEmit); then
    echo "  OK"
  else
    FAILED+=("$name")
  fi
done

echo ""
if [[ ${#FAILED[@]} -gt 0 ]]; then
  echo "TypeScript errors in: ${FAILED[*]}"
  exit 1
fi

echo "All fixture type checks passed."
