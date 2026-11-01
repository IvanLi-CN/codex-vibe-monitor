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

original_modes=()
original_objects=()
original_worktree_hashes=()
original_assume_unchanged=()
original_skip_worktree=()
format_tmp_dir="$(mktemp -d "$repo_root/.codex-format.XXXXXX")"
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
  index_entry="$(git ls-files --stage --error-unmatch -- "$file" | awk '$3 == "0" { print; exit }')"
  read -r original_mode original_object _ _ <<< "$index_entry"
  index_state="$(git ls-files -v -- "$file" | cut -c1)"
  original_modes+=("$original_mode")
  original_objects+=("$original_object")
  case "$index_state" in
    [a-z]) original_assume_unchanged+=(1) ;;
    *) original_assume_unchanged+=(0) ;;
  esac
  case "$index_state" in
    [Ss]) original_skip_worktree+=(1) ;;
    *) original_skip_worktree+=(0) ;;
  esac
  original_worktree_hashes+=("$(git hash-object -- "$file")")
  format_tmp_file="$format_tmp_dir/$file"
  mkdir -p "$(dirname "$format_tmp_file")"
  git cat-file blob "$original_object" > "$format_tmp_file"
  format_tmp_files+=("$format_tmp_file")
done

is_safe_worktree_path() {
  local file="$1"
  local worktree_path resolved_path

  worktree_path="$repo_root/$file"
  if [ -L "$worktree_path" ] || [ ! -f "$worktree_path" ]; then
    return 1
  fi
  resolved_path="$(cd "$(dirname "$worktree_path")" && pwd -P)/$(basename "$worktree_path")"
  [ "$resolved_path" = "$worktree_path" ]
}

case "$surface" in
  web)
    biome_bin="${CODEX_HOOK_BIOME_BIN:-$repo_root/node_modules/.bin/biome}"
    for index in "${!files[@]}"; do
      file="${files[$index]}"
      format_tmp_file="${format_tmp_files[$index]}"
      format_output="$format_tmp_dir/.biome-output-$index"
      "$biome_bin" check --write --stdin-file-path="$file" < "$format_tmp_file" > "$format_output"
      mv "$format_output" "$format_tmp_file"
    done
    ;;
  rust)
    rustfmt_bin="${CODEX_HOOK_RUSTFMT_BIN:-rustfmt}"
    "$rustfmt_bin" --edition 2024 -- "${format_tmp_files[@]}"
    ;;
  markdown)
    dprint_bin="${CODEX_HOOK_DPRINT_BIN:-$repo_root/node_modules/.bin/dprint}"
    for index in "${!files[@]}"; do
      file="${files[$index]}"
      format_tmp_file="${format_tmp_files[$index]}"
      format_output="$format_tmp_dir/.dprint-output-$index"
      "$dprint_bin" fmt --stdin "$file" < "$format_tmp_file" > "$format_output"
      mv "$format_output" "$format_tmp_file"
    done
    ;;
  *)
    printf 'unknown formatter surface: %s\n' "$surface" >&2
    exit 2
    ;;
esac

formatted_hashes=()
format_patch_file="$format_tmp_dir/formatted.patch"
: > "$format_patch_file"
for index in "${!files[@]}"; do
  file="${files[$index]}"
  format_tmp_file="${format_tmp_files[$index]}"
  worktree_path="$repo_root/$file"
  if ! is_safe_worktree_path "$file"; then
    printf 'refusing to stage concurrently modified file: %s\n' "$file" >&2
    exit 1
  fi
  current_index="$(git rev-parse --verify ":$file")"
  current_worktree_hash="$(git hash-object -- "$file")"
  if [ "$current_index" != "${original_objects[$index]}" ] \
    || [ "$current_worktree_hash" != "${original_worktree_hashes[$index]}" ]; then
    printf 'refusing to stage concurrently modified file: %s\n' "$file" >&2
    exit 1
  fi

  formatted_hash="$(git hash-object -- "$format_tmp_file")"
  formatted_hashes+=("$formatted_hash")
  if [ "$formatted_hash" = "${original_worktree_hashes[$index]}" ]; then
    continue
  fi

  context_lines="$(wc -l < "$file")"
  formatted_lines="$(wc -l < "$format_tmp_file")"
  if [ "$formatted_lines" -gt "$context_lines" ]; then
    context_lines="$formatted_lines"
  fi
  context_lines=$((context_lines + 1))
  set +e
  diff -u -U "$context_lines" -L "a/$file" -L "b/$file" "$worktree_path" "$format_tmp_file" >> "$format_patch_file"
  diff_status=$?
  set -e
  if [ "$diff_status" -ne 1 ]; then
    printf 'unable to create formatter patch for: %s\n' "$file" >&2
    exit 1
  fi
done

# Roll back only formatter-owned entries so concurrent edits remain untouched.
restore_index_metadata() {
  local index="$1"
  local file="${files[$index]}"

  if [ "${original_assume_unchanged[$index]}" -eq 1 ]; then
    git update-index --assume-unchanged -- "$file"
  else
    git update-index --no-assume-unchanged -- "$file"
  fi
  if [ "${original_skip_worktree[$index]}" -eq 1 ]; then
    git update-index --skip-worktree -- "$file"
  else
    git update-index --no-skip-worktree -- "$file"
  fi
}

restore_original_state() {
  local index file worktree_path current_index current_worktree_hash original_file
  local context_lines original_lines diff_status rollback_patch
  local rollback_owned=()

  rollback_patch="$format_tmp_dir/rollback.patch"
  : > "$rollback_patch"

  for index in "${!files[@]}"; do
    file="${files[$index]}"
    worktree_path="$repo_root/$file"
    current_index="$(git rev-parse --verify ":$file" 2>/dev/null || true)"
    current_worktree_hash="$(git hash-object -- "$file" 2>/dev/null || true)"
    if [ "$current_index" = "${formatted_hashes[$index]}" ]; then
      rollback_owned+=(1)
    else
      rollback_owned+=(0)
    fi
    if [ "${rollback_owned[$index]}" -eq 1 ] \
      && [ "$current_worktree_hash" = "${formatted_hashes[$index]}" ] \
      && is_safe_worktree_path "$file"; then
      original_file="$format_tmp_dir/.original-$index"
      git cat-file blob "${original_objects[$index]}" > "$original_file"
      context_lines="$(wc -l < "${format_tmp_files[$index]}")"
      original_lines="$(wc -l < "$original_file")"
      if [ "$original_lines" -gt "$context_lines" ]; then
        context_lines="$original_lines"
      fi
      context_lines=$((context_lines + 1))
      set +e
      diff -u -U "$context_lines" -L "a/$file" -L "b/$file" \
        "${format_tmp_files[$index]}" "$original_file" >> "$rollback_patch"
      diff_status=$?
      set -e
      if [ "$diff_status" -ne 1 ]; then
        printf 'unable to create formatter rollback patch for: %s\n' "$file" >&2
      fi
    fi
  done

  if [ -s "$rollback_patch" ]; then
    git apply --index --binary "$rollback_patch" || true
  fi

  for index in "${!files[@]}"; do
    file="${files[$index]}"
    current_index="$(git rev-parse --verify ":$file" 2>/dev/null || true)"
    if [ "${rollback_owned[$index]}" -eq 1 ] \
      && [ "$current_index" = "${formatted_hashes[$index]}" ]; then
      git update-index --add --cacheinfo "${original_modes[$index]},${original_objects[$index]},$file"
      restore_index_metadata "$index"
    elif [ "${rollback_owned[$index]}" -eq 1 ] \
      && [ "$current_index" = "${original_objects[$index]}" ]; then
      restore_index_metadata "$index"
    fi
  done
}

if [ -s "$format_patch_file" ]; then
  if ! git apply --index --binary "$format_patch_file"; then
    restore_original_state
    for file in "${files[@]}"; do
      printf 'refusing to stage concurrently modified file: %s\n' "$file" >&2
    done
    exit 1
  fi
fi

verification_failed=0
for index in "${!files[@]}"; do
  file="${files[$index]}"
  worktree_path="$repo_root/$file"
  if ! is_safe_worktree_path "$file"; then
    verification_failed=1
    continue
  fi
  current_index="$(git rev-parse --verify ":$file")"
  current_worktree_hash="$(git hash-object -- "$file")"
  if [ "$current_index" != "${formatted_hashes[$index]}" ] \
    || [ "$current_worktree_hash" != "${formatted_hashes[$index]}" ]; then
    verification_failed=1
  fi
done

if [ "$verification_failed" -ne 0 ]; then
  restore_original_state
  for file in "${files[@]}"; do
    printf 'refusing to stage concurrently modified file: %s\n' "$file" >&2
  done
  exit 1
fi

for index in "${!files[@]}"; do
  restore_index_metadata "$index"
done
