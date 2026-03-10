#!/usr/bin/env bash
# Test the docs static export end-to-end:
#   1. Build the rex binary
#   2. Export the docs site
#   3. Serve it and verify pages render with correct styling/links
#
# Usage: ./scripts/test-docs-export.sh

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Building rex binary..."
cargo build --quiet

REX="$ROOT/target/debug/rex"

echo "==> Cleaning previous export..."
rm -rf docs/.rex

echo "==> Exporting docs site..."
"$REX" export --root docs 2>&1 | grep -E '✓|Error|error'

EXPORT_DIR="$ROOT/docs/.rex/export"

echo "==> Checking exported files..."
HTML_COUNT=$(find "$EXPORT_DIR" -name '*.html' | wc -l | tr -d ' ')
echo "   Found $HTML_COUNT HTML files"

if [ "$HTML_COUNT" -lt 10 ]; then
  echo "   FAIL: expected at least 10 HTML files"
  exit 1
fi

echo "==> Checking CSS: sidebar classes present..."
if grep -q '\.bg-slate-900' "$EXPORT_DIR/index.html"; then
  echo "   OK: .bg-slate-900 found in CSS"
else
  echo "   FAIL: .bg-slate-900 missing from CSS (Tailwind not scanning components/)"
  exit 1
fi

echo "==> Checking nav links have .html extensions..."
if grep -q 'href="/getting-started.html"' "$EXPORT_DIR/index.html"; then
  echo "   OK: HTML links have .html extension"
else
  echo "   FAIL: href=\"/getting-started\" missing .html extension"
  exit 1
fi

echo "==> Checking RSC flight data links have .html..."
if grep -q '"href":"/getting-started.html"' "$EXPORT_DIR/index.html"; then
  echo "   OK: Flight data hrefs have .html extension"
else
  echo "   FAIL: flight data href missing .html extension"
  exit 1
fi

echo "==> Checking static export flag injected..."
if grep -q '__REX_STATIC_EXPORT' "$EXPORT_DIR/index.html"; then
  echo "   OK: __REX_STATIC_EXPORT flag present"
else
  echo "   FAIL: __REX_STATIC_EXPORT flag missing"
  exit 1
fi

echo "==> Checking subpage renders correctly..."
if grep -q 'Quickstart' "$EXPORT_DIR/getting-started.html"; then
  echo "   OK: getting-started.html contains Quickstart content"
else
  echo "   FAIL: getting-started.html missing content"
  exit 1
fi

echo ""
echo "==> All checks passed!"
