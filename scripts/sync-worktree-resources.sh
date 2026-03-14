#!/usr/bin/env bash
set -euo pipefail

log() {
  printf '[worktree-bootstrap] %s\n' "$*"
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
default_repo_root="$(cd "$script_dir/.." && pwd)"
repo_root="${WORKTREE_BOOTSTRAP_TARGET_ROOT:-$default_repo_root}"
if [ -z "$repo_root" ]; then
  exit 0
fi
repo_root="$(cd "$repo_root" && pwd)"

common_dir="${WORKTREE_BOOTSTRAP_GIT_COMMON_DIR:-$(git -C "$repo_root" rev-parse --git-common-dir 2>/dev/null || true)}"
if [ -z "$common_dir" ]; then
  exit 0
fi

source_root="${WORKTREE_BOOTSTRAP_SOURCE_ROOT:-$(cd "$(dirname "$common_dir")" && pwd)}"
manifest_path="${WORKTREE_BOOTSTRAP_MANIFEST:-$repo_root/scripts/worktree-sync.paths}"

if [ ! -f "$manifest_path" ]; then
  exit 0
fi

source_root="$(cd "$source_root" && pwd)"

if [ "$repo_root" = "$source_root" ]; then
  exit 0
fi

copied_count=0
missing_count=0

while IFS= read -r raw_line || [ -n "$raw_line" ]; do
  line="${raw_line%%#*}"
  line="${line#${line%%[![:space:]]*}}"
  line="${line%${line##*[![:space:]]}}"

  if [ -z "$line" ]; then
    continue
  fi

  src="$source_root/$line"
  dest="$repo_root/$line"

  if [ -e "$dest" ] || [ -L "$dest" ]; then
    continue
  fi

  if [ ! -e "$src" ] && [ ! -L "$src" ]; then
    log "source missing, skipped: $line"
    missing_count=$((missing_count + 1))
    continue
  fi

  mkdir -p "$(dirname "$dest")"
  cp -pR "$src" "$dest"
  log "copied $line"
  copied_count=$((copied_count + 1))
done < "$manifest_path"

if [ "$copied_count" -eq 0 ] && [ "$missing_count" -eq 0 ]; then
  log "nothing to sync"
fi
