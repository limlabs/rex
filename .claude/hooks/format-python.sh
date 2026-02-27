#!/bin/bash
INPUT=$(cat)
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // empty')

# Only format Python files
if [[ "$FILE_PATH" != *.py ]]; then
  exit 0
fi

cd "$CLAUDE_PROJECT_DIR/benchmarks" || exit 0

# Format with black, then lint-fix with ruff
uv run black --quiet "$FILE_PATH" 2>/dev/null
uv run ruff check --fix --quiet "$FILE_PATH" 2>/dev/null
uv run ty check "$FILE_PATH" 2>&1
exit 0
