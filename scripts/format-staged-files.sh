#!/usr/bin/env bash
set -euo pipefail

check_only=0
if [ "${1:-}" = '--check' ]; then
  check_only=1
  surface='all'
  shift
  candidate_files=()
  while IFS= read -r -d '' staged_file; do
    candidate_files+=("$staged_file")
  done < <(git diff --cached --name-only --diff-filter=ACMR -z)
else
  surface="${1:?formatter surface is required}"
  shift
  candidate_files=("$@")
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd "$script_dir/.." && pwd -P)"
cd "$repo_root"
files=()

for file in "${candidate_files[@]}"; do
  # Deleted paths are still present in Git's staged-file list, but cannot be formatted.
  case "$file" in
    /*|..|../*|*/..|*/../*) continue ;;
  esac
  path="$repo_root/$file"
  # Formatters follow symlinks, so no path component may escape the worktree.
  [ -L "$path" ] && continue
  [ -f "$path" ] || continue
  resolved_path="$(cd "$(dirname "$path")" && pwd -P)/$(basename "$path")"
  case "$resolved_path" in
    "$repo_root"/*) ;;
    *) continue ;;
  esac
  [ "$resolved_path" = "$path" ] || continue

  case "$surface" in
    web)
      case "$file" in
        web/*.js|web/*.ts|web/*.cjs|web/*.mjs|web/*.d.cts|web/*.d.mts|web/*.jsx|web/*.tsx|web/*.json|web/*.jsonc)
          files+=("$file")
          ;;
      esac
      ;;
    rust)
      case "$file" in *.rs) files+=("$file") ;; esac
      ;;
    markdown)
      case "$file" in *.md) files+=("$file") ;; esac
      ;;
    all)
      case "$file" in
        web/*.js|web/*.ts|web/*.cjs|web/*.mjs|web/*.d.cts|web/*.d.mts|web/*.jsx|web/*.tsx|web/*.json|web/*.jsonc|*.rs|*.md)
          files+=("$file")
          ;;
      esac
      ;;
  esac
done

[ "${#files[@]}" -gt 0 ] || exit 0

if [ "$check_only" -eq 1 ]; then
  for file in "${files[@]}"; do
    if ! git diff --quiet -- "$file"; then
      printf 'refusing to auto-format partially staged file: %s\n' "$file" >&2
      printf 'stage or unstage all changes in this file before committing\n' >&2
      exit 1
    fi
  done
  exit 0
fi

case "$surface" in
  web)
    biome_bin="${CODEX_HOOK_BIOME_BIN:-$repo_root/node_modules/.bin/biome}"
    "$biome_bin" check --write "${files[@]}"
    ;;
  rust)
    rustfmt_bin="${CODEX_HOOK_RUSTFMT_BIN:-rustfmt}"
    "$rustfmt_bin" --edition 2024 "${files[@]}"
    ;;
  markdown)
    dprint_bin="${CODEX_HOOK_DPRINT_BIN:-$repo_root/node_modules/.bin/dprint}"
    "$dprint_bin" fmt "${files[@]}"
    ;;
  *)
    printf 'unknown formatter surface: %s\n' "$surface" >&2
    exit 2
    ;;
esac
