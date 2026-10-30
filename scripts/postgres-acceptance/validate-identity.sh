#!/usr/bin/env bash
set -euo pipefail

for name in BASE_SHA CANDIDATE_SHA; do
  value="${!name:-}"
  if [[ ! "$value" =~ ^[0-9a-f]{40}$ ]] || [[ "$(git cat-file -t "$value" 2>/dev/null)" != commit ]]; then
    echo "$name must identify an existing full lowercase commit SHA" >&2
    exit 1
  fi
done

test "$(git rev-parse HEAD)" = "$CANDIDATE_SHA"
git merge-base --is-ancestor "$BASE_SHA" "$CANDIDATE_SHA"
