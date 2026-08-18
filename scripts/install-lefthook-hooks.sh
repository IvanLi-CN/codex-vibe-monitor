#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd "$script_dir/.." && pwd -P)"

custom_hooks_path="$(git -C "$repo_root" config --get core.hooksPath || true)"
if [ -n "$custom_hooks_path" ]; then
  printf '[worktree-bootstrap] core.hooksPath is set to %s; leaving hooks untouched\n' "$custom_hooks_path" >&2
  exit 0
fi

lefthook_path=''
if ! command -v realpath >/dev/null 2>&1; then
  printf '[worktree-bootstrap] global lefthook is required; realpath is unavailable\n' >&2
  exit 1
fi

is_repo_local_path() {
  resolved_path="$1"
  case "$resolved_path" in
    "$repo_root"|"$repo_root"/*)
      return 0
      ;;
  esac

  while IFS= read -r worktree_root; do
    case "$resolved_path" in
      "$worktree_root"|"$worktree_root"/*)
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
    [ -n "$resolved_candidate" ] || continue
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

lefthook_version="$($lefthook_path version 2>/dev/null || true)"
lefthook_triplet="$(printf '%s\n' "$lefthook_version" | sed -nE 's/^[^0-9]*([0-9]+)\.([0-9]+)\.([0-9]+).*/\1 \2 \3/p' | head -n 1)"
set -- $lefthook_triplet
lefthook_major="${1:-}"
lefthook_minor="${2:-}"
lefthook_patch="${3:-}"
if [ -z "$lefthook_major" ] \
  || [ "$lefthook_major" -lt 2 ] \
  || { [ "$lefthook_major" -eq 2 ] && [ "$lefthook_minor" -lt 1 ]; } \
  || { [ "$lefthook_major" -eq 2 ] && [ "$lefthook_minor" -eq 1 ] && [ "$lefthook_patch" -lt 7 ]; }; then
  printf '[worktree-bootstrap] Lefthook 2.1.7 or newer is required for safe staged-file restoration; found %s\n' "${lefthook_version:-unknown}" >&2
  exit 1
fi

hooks_dir="$(git -C "$repo_root" rev-parse --git-path hooks)"
case "$hooks_dir" in
  /*) ;;
  *) hooks_dir="$repo_root/$hooks_dir" ;;
esac
hooks_parent="$(dirname "$hooks_dir")"
resolved_hooks_parent="$(realpath "$hooks_parent" 2>/dev/null || true)"
if [ -z "$resolved_hooks_parent" ] || [ "$resolved_hooks_parent" != "$hooks_parent" ]; then
  printf '[worktree-bootstrap] hooks directory has a symlinked parent; leaving hooks untouched: %s\n' "$hooks_dir" >&2
  exit 0
fi
if [ -L "$hooks_dir" ]; then
  printf '[worktree-bootstrap] hooks directory is a symlink; leaving hooks untouched: %s\n' "$hooks_dir" >&2
  exit 0
fi

config_declares_prepare_commit_msg() {
  for config_file in "$repo_root"/lefthook*.yml "$repo_root"/lefthook*.yaml; do
    [ -f "$config_file" ] || continue
    if grep -Eq '^[[:space:]]*prepare-commit-msg:' "$config_file"; then
      return 0
    fi
  done
  return 1
}

customize_pre_commit_template() {
  local template_hook_path="$1"
  local original_line='call_lefthook run "pre-commit" "$@"'

  [ "$(grep -Fxc "$original_line" "$template_hook_path" || true)" = '1' ] || return 1
  perl -0pi -e 's{call_lefthook run "pre-commit" "\$@"}{if ! bash scripts/check-staged-formatter-safety.sh\nthen\n  exit 1\nfi\n\ncall_lefthook run "pre-commit" "\$@"}' "$template_hook_path"
  grep -Fq 'bash scripts/check-staged-formatter-safety.sh' "$template_hook_path"
}

cleanup_legacy_prepare_commit_msg() {
  hook_path="$hooks_dir/prepare-commit-msg"
  [ -e "$hook_path" ] || [ -L "$hook_path" ] || return 0

  if [ -L "$hook_path" ]; then
    printf '[worktree-bootstrap] prepare-commit-msg is a symlink; leaving it untouched\n' >&2
    return 0
  fi
  if config_declares_prepare_commit_msg; then
    printf '[worktree-bootstrap] prepare-commit-msg is configured; leaving it untouched\n' >&2
    return 0
  fi

  template_dir="$(mktemp -d "${TMPDIR:-/tmp}/codex-vibe-monitor-lefthook.XXXXXX")"
  if ! git -C "$template_dir" init -q \
    || ! cp "$repo_root/lefthook.yml" "$template_dir/lefthook.yml" \
    || ! printf '\nprepare-commit-msg:\n  commands:\n    legacy-wrapper:\n      run: "true"\n' >> "$template_dir/lefthook.yml" \
    || ! (
    cd "$template_dir" &&
    "$lefthook_path" install prepare-commit-msg >/dev/null 2>&1
  ); then
    rm -rf "$template_dir"
    printf '[worktree-bootstrap] could not verify prepare-commit-msg ownership; leaving it untouched\n' >&2
    return 0
  fi

  template_path="$template_dir/.git/hooks/prepare-commit-msg"
  if [ -f "$template_path" ] && cmp -s "$hook_path" "$template_path"; then
    rm -f "$hook_path"
    printf '[worktree-bootstrap] removed obsolete Lefthook prepare-commit-msg wrapper\n'
  else
    printf '[worktree-bootstrap] prepare-commit-msg is unmanaged; leaving it untouched\n' >&2
  fi
  rm -rf "$template_dir"
}

cleanup_legacy_prepare_commit_msg

install_hooks=()
is_marked_standard_pre_commit() {
  local existing_hook_path="$1"
  local template_dir template_path

  [ -L "$existing_hook_path" ] && return 1
  template_dir="$(mktemp -d "${TMPDIR:-/tmp}/codex-vibe-monitor-lefthook.XXXXXX")" || return 1
  if ! git -C "$template_dir" init -q \
    || ! cp "$repo_root/lefthook.yml" "$template_dir/lefthook.yml" \
    || ! (
    cd "$template_dir" &&
    "$lefthook_path" install pre-commit >/dev/null 2>&1
  ); then
    rm -rf "$template_dir"
    return 1
  fi

  template_path="$template_dir/.git/hooks/pre-commit"
  printf '\n# managed by codex-vibe-monitor hooks:install\n' >> "$template_path"
  if [ -f "$template_path" ] && cmp -s "$existing_hook_path" "$template_path"; then
    rm -rf "$template_dir"
    return 0
  fi
  rm -rf "$template_dir"
  return 1
}

is_managed_hook() {
  hook_name="$1"
  hook_path="$2"
  if [ ! -e "$hook_path" ] && [ ! -L "$hook_path" ]; then
    return 0
  fi
  if [ -L "$hook_path" ]; then
    return 1
  fi

  template_dir="$(mktemp -d "${TMPDIR:-/tmp}/codex-vibe-monitor-lefthook.XXXXXX")" || return 1
  if ! git -C "$template_dir" init -q \
    || ! cp "$repo_root/lefthook.yml" "$template_dir/lefthook.yml" \
    || ! (
    cd "$template_dir" &&
    "$lefthook_path" install "$hook_name" >/dev/null 2>&1
  ); then
    rm -rf "$template_dir"
    return 1
  fi

  template_path="$template_dir/.git/hooks/$hook_name"
  printf '\n# managed by codex-vibe-monitor hooks:install\n' >> "$template_path"
  if [ -f "$template_path" ] && cmp -s "$hook_path" "$template_path"; then
    rm -rf "$template_dir"
    [ "$hook_name" = 'pre-commit' ] && return 1
    return 0
  fi
  if [ "$hook_name" = 'pre-commit' ] \
    && customize_pre_commit_template "$template_path" \
    && [ -f "$template_path" ] \
    && cmp -s "$hook_path" "$template_path"; then
    rm -rf "$template_dir"
    return 0
  fi
  rm -rf "$template_dir"
  return 1
}

for hook_name in pre-commit commit-msg post-checkout; do
  hook_path="$hooks_dir/$hook_name"
  if is_managed_hook "$hook_name" "$hook_path" \
    || { [ "$hook_name" = 'pre-commit' ] && is_marked_standard_pre_commit "$hook_path"; }; then
    install_hooks+=("$hook_name")
  else
    printf '[worktree-bootstrap] %s already exists and is unmanaged; leaving it untouched\n' "$hook_name" >&2
  fi
done

if [ "${#install_hooks[@]}" -gt 0 ]; then
  (
    cd "$repo_root"
    "$lefthook_path" install "${install_hooks[@]}"
    for hook_name in "${install_hooks[@]}"; do
      if [ "$hook_name" = 'pre-commit' ] \
        && ! customize_pre_commit_template "$hooks_dir/$hook_name"; then
        printf '[worktree-bootstrap] could not install pre-commit safety check\n' >&2
        exit 1
      fi
      printf '\n# managed by codex-vibe-monitor hooks:install\n' >> "$hooks_dir/$hook_name"
    done
  )
fi
