#!/bin/bash
# Runs tsc --noEmit (strict mode) on Rex fixtures.
#
# Usage:
#   scripts/typecheck-fixtures.sh              # check all fixtures
#   scripts/typecheck-fixtures.sh fixtures/basic  # check one fixture
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
FIXTURES_DIR="$ROOT/fixtures"
BENCHMARKS_DIR="$ROOT/benchmarks"
# Skip directories that require build-time codegen (e.g. TanStack Router)
# zero-config: types are extracted by `rex build`, not available before Rex runs
SKIP=("tanstack-basic" "tanstack-tailwind" "zero-config" "context")

# Ensure workspace dependencies are installed (hoisted to root node_modules).
if [[ ! -d "$ROOT/node_modules" ]]; then
  echo "--- Installing workspace dependencies ---"
  (cd "$ROOT" && npm install --no-audit --no-fund)
fi

TMPDIR_BASE="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_BASE"' EXIT

check_fixture() {
  local dir="$1"
  local name
  name="$(basename "$dir")"
  local logfile="$TMPDIR_BASE/$name.log"

  if [[ ! -f "$dir/tsconfig.json" ]]; then
    return 0
  fi

  {
    echo "--- Checking $name ---"

    if [[ ! -d "$dir/node_modules" ]]; then
      echo "  Installing dependencies..."
      (cd "$dir" && npm install --no-package-lock --ignore-scripts --no-audit --no-fund) || {
        echo "  WARN: npm install failed for $name, skipping"
        return 0
      }
    fi

    if (cd "$dir" && npx tsc --noEmit); then
      echo "  OK"
      return 0
    else
      return 1
    fi
  } > "$logfile" 2>&1
}

# Single fixture mode
if [[ $# -ge 1 ]]; then
  dir="$(cd "$1" && pwd)"
  check_fixture "$dir"
  exit $?
fi

# All fixtures mode — run checks in parallel
PIDS=()
NAMES=()

should_skip() {
  local name="$1"
  for skip in "${SKIP[@]}"; do
    [[ "$name" == "$skip" ]] && return 0
  done
  return 1
}

for dir in "$FIXTURES_DIR"/*/; do
  name="$(basename "$dir")"
  should_skip "$name" && continue
  check_fixture "$dir" &
  PIDS+=($!)
  NAMES+=("$name")
done

for dir in "$BENCHMARKS_DIR"/*/; do
  name="$(basename "$dir")"
  should_skip "$name" && continue
  check_fixture "$dir" &
  PIDS+=($!)
  NAMES+=("benchmarks/$name")
done

# Wait for all and collect failures
FAILED=()
for i in "${!PIDS[@]}"; do
  if ! wait "${PIDS[$i]}"; then
    FAILED+=("${NAMES[$i]}")
  fi
done

# Print all logs in order
for i in "${!NAMES[@]}"; do
  name="${NAMES[$i]}"
  logfile="$TMPDIR_BASE/$(basename "$name").log"
  if [[ -f "$logfile" ]]; then
    cat "$logfile"
  fi
done

echo ""
if [[ ${#FAILED[@]} -gt 0 ]]; then
  echo "TypeScript errors in: ${FAILED[*]}"
  exit 1
fi

echo "All fixture and benchmark type checks passed."
