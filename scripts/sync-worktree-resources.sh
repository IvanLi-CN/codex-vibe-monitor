#!/usr/bin/env bash
set -euo pipefail

log() {
  printf '[worktree-bootstrap] %s\n' "$*"
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
default_repo_root="$(cd "$script_dir/.." && pwd)"
repo_root="${WORKTREE_BOOTSTRAP_TARGET_ROOT:-$default_repo_root}"
[ -n "$repo_root" ] || exit 0
repo_root="$(cd "$repo_root" && pwd)"

common_dir="${WORKTREE_BOOTSTRAP_GIT_COMMON_DIR:-$(git -C "$repo_root" rev-parse --git-common-dir 2>/dev/null || true)}"
[ -n "$common_dir" ] || exit 0
case "$common_dir" in
  /*) ;;
  *) common_dir="$repo_root/$common_dir" ;;
esac
common_dir="$(cd "$common_dir" && pwd)"

source_root="${WORKTREE_BOOTSTRAP_SOURCE_ROOT:-$(cd "$(dirname "$common_dir")" && pwd)}"
manifest_path="${WORKTREE_BOOTSTRAP_MANIFEST:-$repo_root/scripts/worktree-sync.paths}"
[ -f "$manifest_path" ] || exit 0
source_root="$(cd "$source_root" && pwd)"
[ "$repo_root" != "$source_root" ] || exit 0

git_dir="$(git -C "$repo_root" rev-parse --git-dir)"
case "$git_dir" in
  /*) ;;
  *) git_dir="$repo_root/$git_dir" ;;
esac
git_dir="$(cd "$git_dir" && pwd)"
sync_lock_path="${WORKTREE_BOOTSTRAP_SYNC_LOCK_PATH:-$git_dir/worktree-bootstrap-sync.lock}"
lock_owner_start="$(ps -p $$ -o lstart= 2>/dev/null | sed 's/^[[:space:]]*//')"
lock_owner_token="$$|$lock_owner_start"

acquire_lock() {
  if [ ! -e "$sync_lock_path" ] && [ ! -L "$sync_lock_path" ] \
    && ln -s "$lock_owner_token" "$sync_lock_path" 2>/dev/null; then
    return 0
  fi

  existing_token="$(readlink "$sync_lock_path" 2>/dev/null || true)"
  existing_pid="${existing_token%%|*}"
  existing_start="${existing_token#*|}"
  if [ -n "$existing_pid" ] && [ "$existing_pid" != "$existing_token" ] \
    && ! kill -0 "$existing_pid" 2>/dev/null; then
    if [ "$(readlink "$sync_lock_path" 2>/dev/null || true)" = "$existing_token" ]; then
      rm -f "$sync_lock_path"
    fi
    if [ ! -e "$sync_lock_path" ] && [ ! -L "$sync_lock_path" ] \
      && ln -s "$lock_owner_token" "$sync_lock_path" 2>/dev/null; then
      return 0
    fi
  elif [ -n "$existing_pid" ] && [ "$existing_pid" != "$existing_token" ] && [ -n "$existing_start" ]; then
    current_start="$(ps -p "$existing_pid" -o lstart= 2>/dev/null | sed 's/^[[:space:]]*//')"
    if [ -n "$current_start" ] && [ "$current_start" != "$existing_start" ] \
      && [ "$(readlink "$sync_lock_path" 2>/dev/null || true)" = "$existing_token" ]; then
      rm -f "$sync_lock_path"
      if ln -s "$lock_owner_token" "$sync_lock_path" 2>/dev/null; then
        return 0
      fi
    fi
  fi
  return 1
}

if ! acquire_lock; then
  log 'sync lock is busy; skipping resource sync'
  exit 0
fi

release_sync_lock() {
  if [ "$(readlink "$sync_lock_path" 2>/dev/null || true)" = "$lock_owner_token" ]; then
    rm -f "$sync_lock_path"
  fi
}
trap release_sync_lock EXIT

copied_count=0
missing_count=0
while IFS= read -r raw_line || [ -n "$raw_line" ]; do
  line="${raw_line%%#*}"
  line="${line#${line%%[![:space:]]*}}"
  line="${line%${line##*[![:space:]]}}"
  [ -n "$line" ] || continue

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
  log 'nothing to sync'
fi
