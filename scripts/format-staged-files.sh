#!/usr/bin/env bash
set -euo pipefail

surface="${1:?formatter surface is required}"
shift

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd "$script_dir/.." && pwd -P)"
cd "$repo_root"
files=()

for file in "$@"; do
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
  esac
done

[ "${#files[@]}" -gt 0 ] || exit 0

case "$surface" in
  web)
    biome_bin="${CODEX_HOOK_BIOME_BIN:-$repo_root/node_modules/.bin/biome}"
    exec "$biome_bin" check --write "${files[@]}"
    ;;
  rust)
    rustfmt_bin="${CODEX_HOOK_RUSTFMT_BIN:-rustfmt}"
    exec "$rustfmt_bin" --edition 2024 "${files[@]}"
    ;;
  markdown)
    dprint_bin="${CODEX_HOOK_DPRINT_BIN:-$repo_root/node_modules/.bin/dprint}"
    exec "$dprint_bin" fmt "${files[@]}"
    ;;
  *)
    printf 'unknown formatter surface: %s\n' "$surface" >&2
    exit 2
    ;;
esac
