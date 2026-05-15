#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"

run_install() {
  label="$1"
  dir="$2"

  printf '[worktree-setup] installing %s dependencies\n' "$label"
  (
    cd "$dir"
    bun install
  )
}

run_install "repo" "$repo_root"
run_install "web" "$repo_root/web"
run_install "docs-site" "$repo_root/docs-site"

printf '[worktree-setup] dependencies installed\n'
