#!/usr/bin/env bash
set -euo pipefail

# Coverage ratchet — ensures the coverage floor only goes up.
#
# 1. Reads the minimum from .coverage-threshold
# 2. Runs cargo llvm-cov and extracts line coverage %
# 3. If coverage < threshold → fail
# 4. If coverage > threshold → update .coverage-threshold and stage it
#
# Requires: cargo-llvm-cov, jq

REPO_ROOT="$(git rev-parse --show-toplevel)"
THRESHOLD_FILE="$REPO_ROOT/.coverage-threshold"

if [ ! -f "$THRESHOLD_FILE" ]; then
    echo "Error: $THRESHOLD_FILE not found"
    exit 1
fi

THRESHOLD="$(cat "$THRESHOLD_FILE" | tr -d '[:space:]')"

if ! [[ "$THRESHOLD" =~ ^[0-9]+$ ]]; then
    echo "Error: .coverage-threshold must contain an integer, got '$THRESHOLD'"
    exit 1
fi

if ! command -v cargo-llvm-cov &>/dev/null; then
    echo "Error: cargo-llvm-cov not found"
    echo "Install: cargo install cargo-llvm-cov"
    exit 1
fi

if ! command -v jq &>/dev/null; then
    echo "Error: jq not found"
    exit 1
fi

echo "Running tests with coverage..."
cargo llvm-cov --workspace --exclude rex_python --ignore-filename-regex 'tests/' --json --output-path "$REPO_ROOT/coverage.json"

COVERAGE=$(jq '.data[0].totals.lines.percent' "$REPO_ROOT/coverage.json" | xargs printf '%.0f')
rm -f "$REPO_ROOT/coverage.json"

echo "Line coverage: ${COVERAGE}% (threshold: ${THRESHOLD}%)"

if [ "$COVERAGE" -lt "$THRESHOLD" ]; then
    echo "FAIL: Coverage ${COVERAGE}% is below threshold ${THRESHOLD}%"
    exit 1
fi

if [ "$COVERAGE" -gt "$THRESHOLD" ]; then
    echo "Coverage exceeds threshold — ratcheting ${THRESHOLD}% → ${COVERAGE}%"
    echo "$COVERAGE" > "$THRESHOLD_FILE"
    git add "$THRESHOLD_FILE"
fi

echo "OK"
