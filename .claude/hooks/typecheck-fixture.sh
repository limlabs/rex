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
  */fixtures/*) ;;
  *) exit 0 ;;
esac

# Extract the fixture directory (fixtures/<name>)
FIXTURE_DIR=$(echo "$FILE_PATH" | sed -n 's|\(.*fixtures/[^/]*\)/.*|\1|p')
if [[ -z "$FIXTURE_DIR" || ! -f "$FIXTURE_DIR/tsconfig.json" ]]; then
  exit 0
fi

ROOT=$(echo "$FILE_PATH" | sed -n 's|\(.*\)/fixtures/.*|\1|p')
exec "$ROOT/scripts/typecheck-fixtures.sh" "$FIXTURE_DIR"
