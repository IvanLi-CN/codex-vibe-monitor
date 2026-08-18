#!/usr/bin/env bash
set -uo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
automatic=0
force=0

for argument in "$@"; do
  case "$argument" in
    --automatic) automatic=1 ;;
    --force) force=1 ;;
    *)
      printf '[worktree-setup] unknown argument: %s\n' "$argument" >&2
      exit 2
      ;;
  esac
done

state_path="${WORKTREE_SETUP_STATE_PATH:-$(git -C "$repo_root" rev-parse --git-path worktree-setup-state-v1)}"
case "$state_path" in
  /*) ;;
  *) state_path="$repo_root/$state_path" ;;
esac
state_dir="$(dirname "$state_path")"
mkdir -p "$state_dir"

surface_names=(root-bun web-bun docs-bun cargo)

digest_for() {
  local input
  {
    for input in "$@"; do
      printf '%s\t' "$input"
      if [ -f "$repo_root/$input" ]; then
        shasum -a 256 "$repo_root/$input"
      else
        printf 'missing\n'
      fi
    done
  } | shasum -a 256 | awk '{print $1}'
}

surface_digest() {
  case "$1" in
    root-bun) digest_for .bun-version package.json bun.lock ;;
    web-bun) digest_for .bun-version web/package.json web/bun.lock ;;
    docs-bun) digest_for .bun-version docs-site/package.json docs-site/bun.lock ;;
    cargo) digest_for Cargo.toml Cargo.lock rust-toolchain.toml ;;
  esac
}

state_value() {
  local lookup_surface="$1"
  local field="$2"
  [ -f "$state_path" ] || return 0
  awk -F '\t' -v surface="$lookup_surface" -v field="$field" '$1 == surface { print $field; exit }' "$state_path"
}

write_state() {
  local updated_surface="$1"
  local updated_status="$2"
  local updated_digest="$3"
  local tmp_state="$state_path.tmp.$$"
  local retained_surface previous_status previous_digest

  {
    printf 'version\t1\n'
    for retained_surface in "${surface_names[@]}"; do
      if [ "$retained_surface" = "$updated_surface" ]; then
        printf '%s\t%s\t%s\n' "$retained_surface" "$updated_status" "$updated_digest"
      else
        previous_status="$(state_value "$retained_surface" 2)"
        previous_digest="$(state_value "$retained_surface" 3)"
        if [ -n "$previous_status" ] && [ -n "$previous_digest" ]; then
          printf '%s\t%s\t%s\n' "$retained_surface" "$previous_status" "$previous_digest"
        fi
      fi
    done
  } > "$tmp_state"
  mv "$tmp_state" "$state_path"
}

surface_is_present() {
  case "$1" in
    root-bun) [ -x "$repo_root/node_modules/.bin/biome" ] ;;
    web-bun) [ -x "$repo_root/web/node_modules/.bin/vitest" ] ;;
    docs-bun) [ -x "$repo_root/docs-site/node_modules/.bin/rspress" ] ;;
    cargo) return 0 ;;
  esac
}

run_surface() {
  local surface="$1"
  local label directory current_digest previous_status previous_digest
  local -a command
  case "$surface" in
    root-bun)
      label='repo Bun dependencies'
      directory="$repo_root"
      command=(bun install --frozen-lockfile)
      ;;
    web-bun)
      label='web Bun dependencies'
      directory="$repo_root/web"
      command=(bun install --frozen-lockfile)
      ;;
    docs-bun)
      label='docs-site Bun dependencies'
      directory="$repo_root/docs-site"
      command=(bun install --frozen-lockfile)
      ;;
    cargo)
      label='Rust dependencies'
      directory="$repo_root"
      command=(cargo fetch --locked)
      ;;
  esac

  current_digest="$(surface_digest "$surface")"
  previous_status="$(state_value "$surface" 2)"
  previous_digest="$(state_value "$surface" 3)"

  if [ "$force" -ne 1 ] && [ "$automatic" -eq 1 ] \
    && [ "$previous_status" = 'failed' ] && [ "$previous_digest" = "$current_digest" ]; then
    printf '[worktree-setup] skipping previously failed %s; run `bun run worktree:setup` to retry\n' "$label" >&2
    return 0
  fi

  if [ "$force" -ne 1 ] && [ "$previous_status" = 'ok' ] \
    && [ "$previous_digest" = "$current_digest" ] && surface_is_present "$surface"; then
    printf '[worktree-setup] %s is up to date\n' "$label"
    return 0
  fi

  printf '[worktree-setup] installing %s\n' "$label"
  if (
    cd "$directory" &&
    "${command[@]}"
  ); then
    write_state "$surface" ok "$current_digest"
    printf '[worktree-setup] installed %s\n' "$label"
    return 0
  fi

  write_state "$surface" failed "$current_digest"
  printf '[worktree-setup] failed %s\n' "$label" >&2
  return 1
}

failed_labels=()
for surface in "${surface_names[@]}"; do
  if ! run_surface "$surface"; then
    failed_labels+=("$surface")
  fi
done

if [ "${#failed_labels[@]}" -gt 0 ]; then
  printf '[worktree-setup] dependency setup failed: %s\n' "${failed_labels[*]}" >&2
  if [ "$automatic" -eq 1 ]; then
    exit 0
  fi
  exit 1
fi

printf '[worktree-setup] dependencies are ready\n'
