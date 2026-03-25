#!/bin/bash
INPUT=$(cat)
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // empty')

# Only format JS/TS files
case "$FILE_PATH" in
  *.ts|*.tsx|*.js|*.jsx) ;;
  *) exit 0 ;;
esac

# Lint-fix with oxlint first (removes debugger, etc.)
case "$FILE_PATH" in
  */runtime/*|*/packages/rex/src/*|*/benchmarks/*)
    npx oxlint --fix "$FILE_PATH" 2>/dev/null
    ;;
esac

# Then format with rex fmt (oxfmt) to clean up whitespace
REX_BIN="$CLAUDE_PROJECT_DIR/target/debug/rex"
if [ -x "$REX_BIN" ]; then
  "$REX_BIN" fmt --file "$FILE_PATH" --root "$CLAUDE_PROJECT_DIR" 2>/dev/null
fi

exit 0
