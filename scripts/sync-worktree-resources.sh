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
case "$common_dir" in
  /*) ;;
  *) common_dir="$repo_root/$common_dir" ;;
esac

source_root="${WORKTREE_BOOTSTRAP_SOURCE_ROOT:-$(cd "$(dirname "$common_dir")" && pwd)}"
manifest_path="${WORKTREE_BOOTSTRAP_MANIFEST:-$repo_root/scripts/worktree-sync.paths}"

if [ ! -f "$manifest_path" ]; then
  exit 0
fi

source_root="$(cd "$source_root" && pwd)"

if [ "$repo_root" = "$source_root" ]; then
  exit 0
fi

sync_lock_dir="$common_dir/worktree-bootstrap-sync.lock"
lock_acquired=0
lock_attempt=0
while [ "$lock_attempt" -lt 200 ]; do
  if mkdir "$sync_lock_dir" 2>/dev/null; then
    printf '%s\n' "$$" > "$sync_lock_dir/pid"
    lock_acquired=1
    break
  fi
  lock_owner="$(cat "$sync_lock_dir/pid" 2>/dev/null || true)"
  if [ -n "$lock_owner" ] && ! kill -0 "$lock_owner" 2>/dev/null; then
    rm -f "$sync_lock_dir/pid"
    rmdir "$sync_lock_dir" >/dev/null 2>&1 || true
    continue
  fi
  lock_attempt=$((lock_attempt + 1))
  sleep 0.05
done
if [ "$lock_acquired" -ne 1 ]; then
  log "sync lock is busy; skipping resource sync"
  exit 1
fi
release_sync_lock() {
  rm -f "$sync_lock_dir/pid"
  rmdir "$sync_lock_dir" >/dev/null 2>&1 || true
}
trap release_sync_lock EXIT

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
