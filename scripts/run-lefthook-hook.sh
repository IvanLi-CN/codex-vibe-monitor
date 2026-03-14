#!/usr/bin/env bash
set -euo pipefail

hook_name="${1:?hook name is required}"
shift || true

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [ -z "$repo_root" ]; then
  exit 0
fi
cd "$repo_root"

sync_script="$repo_root/scripts/sync-worktree-resources.sh"

if [ "$hook_name" = "post-checkout" ]; then
  if [ -x "$sync_script" ]; then
    "$sync_script" "$@"
  fi
  exit 0
fi

if [ -x "$repo_root/node_modules/.bin/lefthook" ]; then
  exec "$repo_root/node_modules/.bin/lefthook" run "$hook_name" "$@"
fi

if command -v lefthook >/dev/null 2>&1; then
  exec lefthook run "$hook_name" "$@"
fi

printf '[worktree-bootstrap] lefthook unavailable for %s; skipping.\n' "$hook_name" >&2
