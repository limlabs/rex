#!/usr/bin/env bash
set -euo pipefail

# Rex code coverage script
# Requires: cargo install cargo-llvm-cov

THRESHOLD="${COVERAGE_THRESHOLD:-50}"

if ! command -v cargo-llvm-cov &>/dev/null; then
    echo "Error: cargo-llvm-cov not found"
    echo "Install: cargo install cargo-llvm-cov"
    exit 1
fi

echo "Running tests with coverage..."
cargo llvm-cov --workspace --fail-under-lines "$THRESHOLD" "$@"
