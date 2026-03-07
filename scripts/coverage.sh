#!/usr/bin/env bash
set -euo pipefail

# Rex code coverage script
# Requires: cargo install cargo-llvm-cov

REPO_ROOT="$(git rev-parse --show-toplevel)"
THRESHOLD="${COVERAGE_THRESHOLD:-$(cat "$REPO_ROOT/.coverage-threshold" 2>/dev/null || echo 50)}"

if ! command -v cargo-llvm-cov &>/dev/null; then
    echo "Error: cargo-llvm-cov not found"
    echo "Install: cargo install cargo-llvm-cov"
    exit 1
fi

echo "Running tests with coverage..."
cargo llvm-cov --workspace --ignore-filename-regex 'tests/' --fail-under-lines "$THRESHOLD" "$@"
