#!/bin/bash
INPUT=$(cat)
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // empty')

# Only format Rust files
if [[ "$FILE_PATH" != *.rs ]]; then
  exit 0
fi

rustfmt "$FILE_PATH" 2>/dev/null
exit 0
