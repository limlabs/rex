#!/bin/bash
# Claude PostToolUse hook: runs tsc --noEmit on the relevant fixture
# when a .ts/.tsx file inside fixtures/ is edited.
INPUT=$(cat)
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // empty')

# Only act on TypeScript files inside fixtures/
if [[ "$FILE_PATH" != *.ts && "$FILE_PATH" != *.tsx ]]; then
  exit 0
fi

case "$FILE_PATH" in
  */fixtures/*)
    ;;
  *)
    exit 0
    ;;
esac

# Extract the fixture directory (fixtures/<name>)
FIXTURE_DIR=$(echo "$FILE_PATH" | sed -n 's|\(.*fixtures/[^/]*\)/.*|\1|p')

if [[ -z "$FIXTURE_DIR" || ! -f "$FIXTURE_DIR/tsconfig.json" ]]; then
  exit 0
fi

# Ensure packages/rex has deps (fixtures resolve rex source via path aliases)
ROOT=$(echo "$FILE_PATH" | sed -n 's|\(.*\)/fixtures/.*|\1|p')
if [[ -n "$ROOT" && -d "$ROOT/packages/rex" && ! -d "$ROOT/packages/rex/node_modules" ]]; then
  (cd "$ROOT/packages/rex" && npm install --ignore-scripts --no-audit --no-fund) 2>/dev/null
fi

# Install fixture deps if needed
if [[ ! -d "$FIXTURE_DIR/node_modules" ]]; then
  (cd "$FIXTURE_DIR" && npm install --ignore-scripts --no-audit --no-fund) 2>/dev/null
fi

# Run tsc and report
OUTPUT=$(cd "$FIXTURE_DIR" && npx tsc --noEmit 2>&1)
STATUS=$?

if [[ $STATUS -ne 0 ]]; then
  FIXTURE_NAME=$(basename "$FIXTURE_DIR")
  echo "TypeScript errors in fixtures/$FIXTURE_NAME:"
  echo "$OUTPUT"
  exit 1
fi

exit 0
