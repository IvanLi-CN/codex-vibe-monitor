#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"

if ! command -v lefthook >/dev/null 2>&1; then
  printf '[worktree-bootstrap] lefthook is required on PATH\n' >&2
  exit 1
fi

(cd "$repo_root" && lefthook install)
bash "$repo_root/scripts/sync-worktree-resources.sh"
bash "$repo_root/scripts/worktree-setup.sh"
