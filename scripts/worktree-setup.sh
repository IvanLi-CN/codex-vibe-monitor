#!/usr/bin/env bash
set -uo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"

failed_labels=()

run_step() {
  label="$1"
  dir="$2"
  shift 2

  printf '[worktree-setup] installing %s\n' "$label"
  if (
    cd "$dir" &&
    "$@"
  ); then
    printf '[worktree-setup] installed %s\n' "$label"
  else
    printf '[worktree-setup] failed %s\n' "$label" >&2
    failed_labels+=("$label")
  fi
}

run_step "repo Bun dependencies" "$repo_root" bun install --frozen-lockfile
run_step "web Bun dependencies" "$repo_root/web" bun install --frozen-lockfile
run_step "docs-site Bun dependencies" "$repo_root/docs-site" bun install --frozen-lockfile
run_step "Rust dependencies" "$repo_root" cargo fetch --locked

if [ "${#failed_labels[@]}" -gt 0 ]; then
  printf '[worktree-setup] dependency setup failed: %s\n' "${failed_labels[*]}" >&2
  exit 1
fi

printf '[worktree-setup] dependencies installed\n'
