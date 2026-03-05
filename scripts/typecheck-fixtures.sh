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
SKIP=("tanstack-basic" "tanstack-tailwind")

# Ensure packages/rex has its dependencies installed (fixtures resolve
# rex source files via path aliases and need @types/react available).
REX_PKG="$ROOT/packages/rex"
if [[ ! -d "$REX_PKG/node_modules" ]]; then
  echo "--- Installing packages/rex dependencies ---"
  (cd "$REX_PKG" && npm install --ignore-scripts --no-audit --no-fund)
fi

check_fixture() {
  local dir="$1"
  local name
  name="$(basename "$dir")"

  if [[ ! -f "$dir/tsconfig.json" ]]; then
    return 0
  fi

  echo "--- Checking $name ---"

  if [[ ! -d "$dir/node_modules" ]]; then
    echo "  Installing dependencies..."
    (cd "$dir" && npm install --ignore-scripts --no-audit --no-fund) || {
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
}

# Single fixture mode
if [[ $# -ge 1 ]]; then
  dir="$(cd "$1" && pwd)"
  check_fixture "$dir"
  exit $?
fi

# All fixtures mode
FAILED=()

for dir in "$FIXTURES_DIR"/*/; do
  name="$(basename "$dir")"

  for skip in "${SKIP[@]}"; do
    [[ "$name" == "$skip" ]] && continue 2
  done

  if ! check_fixture "$dir"; then
    FAILED+=("$name")
  fi
done

for dir in "$BENCHMARKS_DIR"/*/; do
  name="$(basename "$dir")"

  for skip in "${SKIP[@]}"; do
    [[ "$name" == "$skip" ]] && continue 2
  done

  if ! check_fixture "$dir"; then
    FAILED+=("benchmarks/$name")
  fi
done

echo ""
if [[ ${#FAILED[@]} -gt 0 ]]; then
  echo "TypeScript errors in: ${FAILED[*]}"
  exit 1
fi

echo "All fixture and benchmark type checks passed."
