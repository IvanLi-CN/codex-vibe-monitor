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

sync_lock_path="$common_dir/worktree-bootstrap-sync.lock"
lock_owner_start="$(ps -p $$ -o lstart= 2>/dev/null | sed 's/^[[:space:]]*//')"
lock_owner_token="$$|$lock_owner_start"
legacy_empty_lock_grace_seconds=10
lock_acquired=0
lock_attempt=0
while [ "$lock_attempt" -lt 200 ]; do
  if [ ! -e "$sync_lock_path" ] && [ ! -L "$sync_lock_path" ] \
    && ln -s "$lock_owner_token" "$sync_lock_path" 2>/dev/null; then
    lock_acquired=1
    break
  fi
  legacy_lock_dir=0
  if [ -d "$sync_lock_path" ]; then
    legacy_lock_dir=1
    lock_token=''
    lock_owner="$(sed -n '1p' "$sync_lock_path/owner" 2>/dev/null || true)"
    lock_owner_start="$(sed -n '2p' "$sync_lock_path/owner" 2>/dev/null || true)"
  else
    lock_token="$(readlink "$sync_lock_path" 2>/dev/null || true)"
    lock_owner="${lock_token%%|*}"
    lock_owner_start="${lock_token#*|}"
  fi
  legacy_empty_lock_old=0
  if [ "$legacy_lock_dir" -eq 1 ] && [ -z "$lock_owner" ]; then
    lock_mtime="$(stat -f %m "$sync_lock_path" 2>/dev/null || stat -c %Y "$sync_lock_path" 2>/dev/null || true)"
    current_time="$(date +%s)"
    if [ -n "$lock_mtime" ] && [ "$lock_mtime" -le $((current_time - legacy_empty_lock_grace_seconds)) ]; then
      legacy_empty_lock_old=1
    fi
  fi
  lock_stale=0
  if [ -n "$lock_owner" ] && ! kill -0 "$lock_owner" 2>/dev/null; then
    lock_stale=1
  elif [ -n "$lock_owner" ] && [ -n "$lock_owner_start" ]; then
    current_owner_start="$(ps -p "$lock_owner" -o lstart= 2>/dev/null | sed 's/^[[:space:]]*//')"
    [ -n "$current_owner_start" ] && [ "$current_owner_start" != "$lock_owner_start" ] && lock_stale=1
  fi
  if [ "$lock_stale" -eq 1 ]; then
    if [ "$legacy_lock_dir" -eq 1 ]; then
      rm -f "$sync_lock_path/owner"
      rmdir "$sync_lock_path" >/dev/null 2>&1 || true
    elif [ "$(readlink "$sync_lock_path" 2>/dev/null || true)" = "$lock_token" ]; then
      rm -f "$sync_lock_path"
    fi
    continue
  fi
  if [ "$legacy_lock_dir" -eq 1 ] && [ "$legacy_empty_lock_old" -eq 1 ] && [ "$lock_attempt" -ge 20 ]; then
    rmdir "$sync_lock_path" >/dev/null 2>&1 || true
    [ -e "$sync_lock_path" ] || continue
  fi
  lock_attempt=$((lock_attempt + 1))
  sleep 0.05
done
if [ "$lock_acquired" -ne 1 ]; then
  log "sync lock is busy; skipping resource sync"
  exit 1
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
