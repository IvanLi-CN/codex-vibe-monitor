#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd "$script_dir/.." && pwd -P)"

lefthook_path=''
is_repo_local_path() {
  resolved_path="$1"
  case "$resolved_path" in
    "$repo_root"/node_modules/*|"$repo_root"/*/node_modules/*)
      return 0
      ;;
  esac

  while IFS= read -r worktree_root; do
    case "$resolved_path" in
      "$worktree_root"/node_modules/*|"$worktree_root"/*/node_modules/*)
        return 0
        ;;
    esac
  done < <(git -C "$repo_root" worktree list --porcelain 2>/dev/null | awk '/^worktree / {print substr($0, 10)}')

  return 1
}

old_ifs="$IFS"
IFS=:
for path_entry in $PATH; do
  [ -n "$path_entry" ] || path_entry='.'
  path_entry="$(cd "$path_entry" 2>/dev/null && pwd -P || true)"
  [ -n "$path_entry" ] || continue
  candidate="$path_entry/lefthook"
  case "$candidate" in
    "$repo_root"/node_modules/.bin/*|"$repo_root"/*/node_modules/.bin/*)
      continue
      ;;
  esac
  if [ -x "$candidate" ]; then
    resolved_candidate="$(realpath "$candidate" 2>/dev/null || true)"
    if is_repo_local_path "$resolved_candidate"; then
        continue
    fi
    lefthook_path="$candidate"
    break
  fi
done
IFS="$old_ifs"

if [ -z "$lefthook_path" ]; then
  printf '[worktree-bootstrap] global lefthook is required; repo-local binary is not sufficient\n' >&2
  exit 1
fi

custom_hooks_path="$(git -C "$repo_root" config --get core.hooksPath || true)"
if [ -n "$custom_hooks_path" ]; then
  printf '[worktree-bootstrap] core.hooksPath is set to %s; leaving hooks untouched\n' "$custom_hooks_path" >&2
  exit 0
fi

hooks_dir="$(git -C "$repo_root" rev-parse --git-path hooks)"
case "$hooks_dir" in
  /*) ;;
  *) hooks_dir="$repo_root/$hooks_dir" ;;
esac

install_hooks=()
is_managed_hook() {
  hook_path="$1"
  if [ ! -e "$hook_path" ] && [ ! -L "$hook_path" ]; then
    return 0
  fi

  if grep -Fq '# managed by codex-vibe-monitor hooks:install' "$hook_path" 2>/dev/null; then
    return 0
  fi

  grep -Eq '^call_lefthook\(\)$' "$hook_path" 2>/dev/null \
    && grep -Eq '^[[:space:]]*call_lefthook run ' "$hook_path" 2>/dev/null
}

for hook_name in pre-commit commit-msg post-checkout; do
  hook_path="$hooks_dir/$hook_name"
  if ! is_managed_hook "$hook_path"; then
    printf '[worktree-bootstrap] %s already exists and is unmanaged; leaving it untouched\n' "$hook_name" >&2
    continue
  fi
  install_hooks+=("$hook_name")
done

if [ "${#install_hooks[@]}" -gt 0 ]; then
  (
    cd "$repo_root"
    "$lefthook_path" install "${install_hooks[@]}"
  )
fi
