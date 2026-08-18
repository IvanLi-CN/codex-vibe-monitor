#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"

status=0
if ! bash "$repo_root/scripts/install-lefthook-hooks.sh"; then
  status=1
fi
if ! bash "$repo_root/scripts/sync-worktree-resources.sh"; then
  status=1
fi
if ! bash "$repo_root/scripts/worktree-setup.sh" "$@"; then
  status=1
fi
exit "$status"
