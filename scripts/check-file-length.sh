#!/usr/bin/env bash
set -euo pipefail

MAX_LINES=700
EXTENSIONS='rs|ts|tsx|js|jsx|sh|yml|yaml|toml|css|html'

# Usage: check-file-length.sh [--staged | --diff BASE]
#   --staged    Check staged files (pre-commit)
#   --diff BASE Check files changed vs BASE branch (CI)
#
# Only flags a file if it is newly added OR its line count increased
# compared to the base version. This avoids failing on grandfathered
# files that were already over the limit before this check existed.

if [ "${1:-}" = "--staged" ]; then
    files=$(git diff --cached --name-only --diff-filter=ACM)
    base_ref="HEAD"
elif [ "${1:-}" = "--diff" ]; then
    base="${2:-origin/main}"
    files=$(git diff --name-only "$base"...HEAD -- || git diff --name-only "$base" HEAD --)
    base_ref="$base"
else
    echo "Usage: check-file-length.sh [--staged | --diff BASE]"
    exit 1
fi

filtered=$(echo "$files" | grep -E "\.($EXTENSIONS)$" || true)

violations=()
while IFS= read -r file; do
    [ -z "$file" ] && continue
    [ ! -f "$file" ] && continue
    lines=$(wc -l < "$file" | tr -d ' ')
    if [ "$lines" -gt "$MAX_LINES" ]; then
        # Check if the file existed in the base — if so, only fail when it grew
        old_lines=$(git show "$base_ref":"$file" 2>/dev/null | wc -l | tr -d ' ' || echo 0)
        if [ "$old_lines" -gt "$MAX_LINES" ] && [ "$lines" -le "$old_lines" ]; then
            continue
        fi
        violations+=("  $file ($lines lines)")
    fi
done <<< "$filtered"

if [ "${#violations[@]}" -gt 0 ]; then
    echo "Files exceeding $MAX_LINES lines:"
    for v in "${violations[@]}"; do
        echo "$v"
    done
    exit 1
fi
