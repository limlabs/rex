#!/bin/bash
# Validate that commit messages follow Conventional Commits format.
# Required by release-please for changelog generation and version bumps.
#
# Format: type(scope): description
# Types:  feat, fix, chore, docs, style, refactor, perf, test, build, ci, revert

MSG_FILE="$1"
MSG=$(head -1 "$MSG_FILE")

# Allow release-please auto-commits
[[ "$MSG" =~ ^chore\(main\):\ release ]] && exit 0

# Allow merge commits
[[ "$MSG" =~ ^Merge ]] && exit 0

# Conventional commit pattern
PATTERN="^(feat|fix|chore|docs|style|refactor|perf|test|build|ci|revert)(\(.+\))?(!)?: .+"

if ! [[ "$MSG" =~ $PATTERN ]]; then
  echo ""
  echo "ERROR: Commit message does not follow Conventional Commits format."
  echo ""
  echo "  Expected: type(scope): description"
  echo "  Types:    feat, fix, chore, docs, style, refactor, perf, test, build, ci, revert"
  echo ""
  echo "  Examples:"
  echo "    feat: add user authentication"
  echo "    fix(router): handle trailing slashes"
  echo "    feat!: redesign config format (breaking change)"
  echo ""
  echo "  Got: $MSG"
  echo ""
  exit 1
fi
