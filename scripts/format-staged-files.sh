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

deduplicated_files=()
if [ "${#files[@]}" -gt 0 ]; then
  for file in "${files[@]}"; do
    already_selected=0
    if [ "${#deduplicated_files[@]}" -gt 0 ]; then
      for selected_file in "${deduplicated_files[@]}"; do
        if [ "$selected_file" = "$file" ]; then
          already_selected=1
          break
        fi
      done
    fi
    if [ "$already_selected" -eq 0 ]; then
      deduplicated_files+=("$file")
    fi
  done
fi
if [ "${#deduplicated_files[@]}" -gt 0 ]; then
  files=("${deduplicated_files[@]}")
else
  files=()
fi

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
index_path="$(git rev-parse --path-format=absolute --git-path index)"
original_index_snapshot="$format_tmp_dir/original-index"
original_index_identity=''
private_index_path=''
index_file_override=''
index_lock_path=''
index_lock_held=0
rollback_needed=0

index_identity() {
  local stat_identity index_hash

  [ -f "$index_path" ] || return 1
  [ ! -L "$index_path" ] || return 1
  if stat_identity="$(stat -c '%d:%i:%s:%Y' "$index_path" 2>/dev/null)"; then
    :
  else
    stat_identity="$(stat -f '%d:%i:%z:%m' "$index_path" 2>/dev/null)" || return 1
  fi
  index_hash="$(git hash-object -- "$index_path")" || return 1
  printf '%s:%s\n' "$stat_identity" "$index_hash"
}

capture_index_snapshot() {
  local snapshot_path="$1"
  local attempt before after

  for attempt in 1 2 3; do
    before="$(index_identity)" || return 1
    cp -p -- "$index_path" "$snapshot_path" || return 1
    after="$(index_identity)" || return 1
    if [ "$before" = "$after" ] && cmp -s "$index_path" "$snapshot_path"; then
      printf '%s\n' "$after"
      return 0
    fi
  done

  return 1
}

index_matches_snapshot() {
  local snapshot_path="$1"
  local snapshot_identity="$2"
  local current_identity

  [ -n "$snapshot_identity" ] || return 1
  current_identity="$(index_identity 2>/dev/null || true)"
  [ "$current_identity" = "$snapshot_identity" ] \
    && cmp -s "$index_path" "$snapshot_path"
}

acquire_index_lock() {
  [ "$index_lock_held" -eq 0 ] || return 0
  index_lock_path="$index_path.lock"
  if ! perl -MFcntl=:DEFAULT -e '
    my ($path) = @ARGV;
    sysopen my $lock, $path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0666 or exit 1;
    if (!close $lock) {
      unlink $path;
      exit 1;
    }
  ' "$index_lock_path"; then
    index_lock_path=''
    return 1
  fi
  index_lock_held=1
}

release_index_lock() {
  if [ "$index_lock_held" -eq 1 ]; then
    rm -f -- "$index_lock_path"
    index_lock_held=0
    index_lock_path=''
  fi
}

update_index() {
  if [ -n "$index_file_override" ]; then
    GIT_INDEX_FILE="$index_file_override" git update-index "$@"
  else
    git update-index "$@"
  fi
}

cleanup_format_tmp() {
  local exit_code=$?

  trap - EXIT INT TERM
  if [ "$rollback_needed" -eq 1 ] && declare -F restore_original_state >/dev/null 2>&1; then
    set +e
    restore_original_state
    set -e
  fi
  release_index_lock
  rm -rf "$format_tmp_dir"
  exit "$exit_code"
}
trap cleanup_format_tmp EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

if ! original_index_identity="$(capture_index_snapshot "$original_index_snapshot")"; then
  printf 'refusing to format while the Git index is changing\n' >&2
  exit 1
fi

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
    "$rustfmt_bin" --edition 2024 --config skip_children=true -- "${format_tmp_files[@]}"
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

# Restore the complete index only while this formatter still owns its snapshot.
restore_index_metadata() {
  local index="$1"
  local file="${files[$index]}"

  if [ "${original_assume_unchanged[$index]}" -eq 1 ]; then
    update_index --assume-unchanged -- "$file"
  else
    update_index --no-assume-unchanged -- "$file"
  fi
  if [ "${original_skip_worktree[$index]}" -eq 1 ]; then
    update_index --skip-worktree -- "$file"
  else
    update_index --no-skip-worktree -- "$file"
  fi
}

restore_original_state() {
  local index file current_worktree_hash original_file rollback_file
  local context_lines original_lines diff_status rollback_failed=0
  local rollback_patches=()

  if [ "$index_lock_held" -ne 1 ] || [ -z "$private_index_path" ]; then
    printf 'refusing to roll back formatter state without the formatter index lock\n' >&2
    return 1
  fi
  if ! index_matches_snapshot "$original_index_snapshot" "$original_index_identity"; then
    printf 'refusing to roll back formatter state after a concurrent Git index change\n' >&2
    release_index_lock
    return 1
  fi

  for index in "${!files[@]}"; do
    file="${files[$index]}"
    current_worktree_hash="$(git hash-object -- "$file" 2>/dev/null || true)"
    rollback_patches+=("")
    if [ "$current_worktree_hash" = "${original_worktree_hashes[$index]}" ]; then
      continue
    fi
    if [ "$current_worktree_hash" != "${formatted_hashes[$index]}" ] \
      || ! is_safe_worktree_path "$file"; then
      rollback_failed=1
      continue
    fi
    if [ "$formatted_hashes[$index]" != "${original_worktree_hashes[$index]}" ]; then
      original_file="$format_tmp_dir/.original-$index"
      rollback_file="$format_tmp_dir/.rollback-$index.patch"
      git cat-file blob "${original_objects[$index]}" > "$original_file"
      context_lines="$(wc -l < "${format_tmp_files[$index]}")"
      original_lines="$(wc -l < "$original_file")"
      if [ "$original_lines" -gt "$context_lines" ]; then
        context_lines="$original_lines"
      fi
      context_lines=$((context_lines + 1))
      set +e
      diff -u -U "$context_lines" -L "a/$file" -L "b/$file" \
        "${format_tmp_files[$index]}" "$original_file" > "$rollback_file"
      diff_status=$?
      set -e
      if [ "$diff_status" -eq 1 ]; then
        rollback_patches[$index]="$rollback_file"
      else
        printf 'unable to create formatter rollback patch for: %s\n' "$file" >&2
        rollback_failed=1
      fi
    fi
  done

  if [ "$rollback_failed" -ne 0 ]; then
    printf 'refusing to roll back formatter state after a concurrent worktree change\n' >&2
    release_index_lock
    return 1
  fi

  for index in "${!files[@]}"; do
    rollback_file="${rollback_patches[$index]:-}"
    if [ -n "$rollback_file" ] && [ -s "$rollback_file" ]; then
      if ! git apply --binary "$rollback_file"; then
        rollback_failed=1
        break
      fi
    fi
  done

  if [ "$rollback_failed" -ne 0 ]; then
    printf 'refusing to roll back formatter state after a concurrent worktree change\n' >&2
    release_index_lock
    return 1
  fi

  for index in "${!files[@]}"; do
    file="${files[$index]}"
    if [ "${formatted_hashes[$index]}" != "${original_worktree_hashes[$index]}" ] \
      && [ "$(git hash-object -- "$file" 2>/dev/null || true)" != "${original_worktree_hashes[$index]}" ]; then
      rollback_failed=1
      break
    fi
  done
  if [ "$rollback_failed" -ne 0 ]; then
    printf 'refusing to roll back formatter state after a concurrent worktree change\n' >&2
    release_index_lock
    return 1
  fi

  release_index_lock
}

if [ -s "$format_patch_file" ]; then
  if ! index_matches_snapshot "$original_index_snapshot" "$original_index_identity"; then
    printf 'refusing to stage concurrently modified files: Git index changed during formatting\n' >&2
    exit 1
  fi
  if ! acquire_index_lock; then
    printf 'refusing to stage while the Git index is concurrently locked\n' >&2
    exit 1
  fi
  if ! index_matches_snapshot "$original_index_snapshot" "$original_index_identity"; then
    printf 'refusing to stage concurrently modified files: Git index changed during formatting\n' >&2
    release_index_lock
    exit 1
  fi

  private_index_path="$format_tmp_dir/formatted-index"
  if ! cp -p -- "$original_index_snapshot" "$private_index_path"; then
    printf 'unable to prepare the private Git index for formatting\n' >&2
    release_index_lock
    exit 1
  fi
  index_file_override="$private_index_path"
  if ! GIT_INDEX_FILE="$private_index_path" git apply --check --index --binary "$format_patch_file"; then
    index_file_override=''
    private_index_path=''
    release_index_lock
    for file in "${files[@]}"; do
      printf 'refusing to stage concurrently modified file: %s\n' "$file" >&2
    done
    exit 1
  fi
  for index in "${!files[@]}"; do
    file="${files[$index]}"
    if ! is_safe_worktree_path "$file" \
      || [ "$(git hash-object -- "$file")" != "${original_worktree_hashes[$index]}" ]; then
      for concurrent_file in "${files[@]}"; do
        printf 'refusing to stage concurrently modified file: %s\n' "$concurrent_file" >&2
      done
      index_file_override=''
      private_index_path=''
      release_index_lock
      exit 1
    fi
  done
  rollback_needed=1
  if ! GIT_INDEX_FILE="$private_index_path" git apply --index --binary "$format_patch_file"; then
    restore_original_state || true
    rollback_needed=0
    index_file_override=''
    private_index_path=''
    for file in "${files[@]}"; do
      printf 'refusing to stage concurrently modified file: %s\n' "$file" >&2
    done
    exit 1
  fi

  for index in "${!files[@]}"; do
    restore_index_metadata "$index"
  done

  verification_failed=0
  for index in "${!files[@]}"; do
    file="${files[$index]}"
    if ! is_safe_worktree_path "$file" \
      || [ "$(git hash-object -- "$file")" != "${formatted_hashes[$index]}" ] \
      || [ "$(GIT_INDEX_FILE="$private_index_path" git rev-parse --verify ":$file")" != "${formatted_hashes[$index]}" ]; then
      verification_failed=1
    fi
  done

  if [ "$verification_failed" -ne 0 ]; then
    restore_original_state || true
    rollback_needed=0
    index_file_override=''
    private_index_path=''
    for file in "${files[@]}"; do
      printf 'refusing to stage concurrently modified file: %s\n' "$file" >&2
    done
    exit 1
  fi

  if ! cp -p -- "$private_index_path" "$index_lock_path"; then
    restore_original_state || true
    rollback_needed=0
    index_file_override=''
    private_index_path=''
    printf 'unable to prepare the formatted Git index\n' >&2
    exit 1
  fi
  if ! mv -f -- "$index_lock_path" "$index_path"; then
    restore_original_state || true
    rollback_needed=0
    index_file_override=''
    private_index_path=''
    printf 'unable to install the formatted Git index\n' >&2
    exit 1
  fi
  index_lock_held=0
  index_lock_path=''
  index_file_override=''
  private_index_path=''
  rollback_needed=0
fi
