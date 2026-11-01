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
  # Preserve the pinned third-party snapshot and its upstream hash manifest.
  # Its two Rust patches are formatted by cargo fmt using the crate's edition.
  case "$file" in vendor/hotpath/*) continue ;; esac
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

for file in "${files[@]}"; do
  if ! git ls-files --stage --error-unmatch -- "$file" 2>/dev/null | awk '$3 == "0" { found = 1 } END { exit found ? 0 : 1 }'; then
    printf 'refusing to format a file that is not staged: %s\n' "$file" >&2
    exit 1
  fi
done

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
    original_modes=()
    original_objects=()
    original_worktree_hashes=()
    format_tmp_dir="$(mktemp -d "$repo_root/.codex-rustfmt.XXXXXX")"
    format_tmp_files=()
    cleanup_format_tmp() {
      rm -rf "$format_tmp_dir"
    }
    trap cleanup_format_tmp EXIT
    for file in "${files[@]}"; do
      if ! git diff --quiet -- "$file"; then
        printf 'refusing to auto-format partially staged file: %s\n' "$file" >&2
        printf 'stage or unstage all changes in this file before committing\n' >&2
        exit 1
      fi
    done
    for file in "${files[@]}"; do
      index_entry="$(git ls-files --stage --error-unmatch -- "$file" | awk '$3 == "0" { print; exit }')"
      read -r original_mode original_object original_stage _ <<< "$index_entry"
      original_modes+=("$original_mode")
      original_objects+=("$original_object")
      original_worktree_hashes+=("$(git hash-object -- "$file")")
      format_tmp_file="$format_tmp_dir/$file"
      mkdir -p "$(dirname "$format_tmp_file")"
      cp "$repo_root/$file" "$format_tmp_file"
      format_tmp_files+=("$format_tmp_file")
    done
    "$rustfmt_bin" --edition 2024 -- "${format_tmp_files[@]}"
    for index in "${!files[@]}"; do
      file="${files[$index]}"
      if [ "$(git hash-object -- "$file")" != "${original_worktree_hashes[$index]}" ]; then
        printf 'refusing to format concurrently modified file: %s\n' "$file" >&2
        exit 1
      fi
      formatted_hash="$(git hash-object -- "${format_tmp_files[$index]}")"
      cp "${format_tmp_files[$index]}" "$repo_root/$file"
      if [ "$(git hash-object -- "$file")" != "$formatted_hash" ]; then
        printf 'refusing to stage concurrently modified file: %s\n' "$file" >&2
        exit 1
      fi
      git add -- "$file"
      staged_hash="$(git rev-parse --verify ":$file")"
      if [ "$staged_hash" != "$formatted_hash" ]; then
        git update-index --add --cacheinfo "${original_modes[$index]},${original_objects[$index]},$file"
        printf 'refusing to stage concurrently modified file: %s\n' "$file" >&2
        exit 1
      fi
    done
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
