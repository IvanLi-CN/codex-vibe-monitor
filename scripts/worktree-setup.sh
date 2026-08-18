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
setup_lock_path="${WORKTREE_SETUP_LOCK_PATH:-$state_dir/worktree-setup.flock}"
if [ "${WORKTREE_SETUP_LOCK_HELD:-}" != '1' ]; then
  if perl -MFcntl=:flock -e '
    my ($automatic, $lock_path, $script, @args) = @ARGV;
    open my $lock, ">>", $lock_path or exit 1;
    my $mode = LOCK_EX | ($automatic ? LOCK_NB : 0);
    exit 75 unless flock($lock, $mode);
    $ENV{WORKTREE_SETUP_LOCK_HELD} = 1;
    my $status = system { $script } $script, @args;
    exit($status == -1 ? 1 : $status >> 8);
  ' "$automatic" "$setup_lock_path" "$script_dir/worktree-setup.sh" "$@"; then
    exit 0
  else
    lock_status=$?
  fi
  if [ "$lock_status" -eq 75 ] && [ "$automatic" -eq 1 ]; then
    printf '[worktree-setup] setup lock is busy; skipping automatic recovery\n'
    exit 0
  fi
  exit "$lock_status"
fi

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
  local updated_presence="$4"
  local tmp_state="$state_path.tmp.$$"
  local retained_surface previous_status previous_digest previous_presence

  {
    printf 'version\t2\n'
    for retained_surface in "${surface_names[@]}"; do
      if [ "$retained_surface" = "$updated_surface" ]; then
        printf '%s\t%s\t%s\t%s\n' \
          "$retained_surface" "$updated_status" "$updated_digest" "$updated_presence"
      else
        previous_status="$(state_value "$retained_surface" 2)"
        previous_digest="$(state_value "$retained_surface" 3)"
        if [ -n "$previous_status" ] && [ -n "$previous_digest" ]; then
          previous_presence="$(state_value "$retained_surface" 4)"
          printf '%s\t%s\t%s\t%s\n' \
            "$retained_surface" "$previous_status" "$previous_digest" "${previous_presence:--}"
        fi
      fi
    done
  } > "$tmp_state"
  mv "$tmp_state" "$state_path"
}

bun_surface_is_present() {
  local manifest_path="$1"
  local modules_path="$2"

  python3 - "$manifest_path" "$modules_path" <<'PY'
import json
from pathlib import Path
import sys

manifest_path = Path(sys.argv[1])
modules_path = Path(sys.argv[2])
try:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
except (OSError, json.JSONDecodeError):
    raise SystemExit(1)

packages = set()
for field in ("dependencies", "devDependencies"):
    values = manifest.get(field, {})
    if not isinstance(values, dict):
        raise SystemExit(1)
    packages.update(name for name in values if isinstance(name, str) and name)

if not packages:
    raise SystemExit(1)

for package in packages:
    package_path = modules_path.joinpath(*package.split("/"))
    if not (package_path / "package.json").is_file():
        raise SystemExit(1)
PY
}

cargo_registry_archives() {
  awk '
    function emit() {
      if (registry && name != "" && version != "") {
        print name "-" version ".crate"
      }
    }
    /^\[\[package\]\]$/ {
      emit()
      name = ""
      version = ""
      registry = 0
      next
    }
    /^name = "/ {
      value = $0
      sub(/^name = "/, "", value)
      sub(/"$/, "", value)
      name = value
      next
    }
    /^version = "/ {
      value = $0
      sub(/^version = "/, "", value)
      sub(/"$/, "", value)
      version = value
      next
    }
    /^source = "registry\+/ {
      registry = 1
    }
    END { emit() }
  ' "$repo_root/Cargo.lock"
}

cargo_surface_is_present() {
  local cargo_home="${CARGO_HOME:-${HOME}/.cargo}"
  local cache_path="$cargo_home/registry/cache"

  [ -d "$cache_path" ] || return 1
  awk 'NR == FNR { available[$0] = 1; next } !available[$0] { exit 1 }' \
    <(find "$cache_path" -type f -name '*.crate' -exec basename {} \; 2>/dev/null) \
    <(cargo_registry_archives)
}

cargo_cache_layout_fingerprint() {
  local cargo_home="${CARGO_HOME:-${HOME}/.cargo}"
  local cache_path="$cargo_home/registry/cache"

  [ -d "$cache_path" ] || return 1
  awk '
    NR == FNR { required[$0] = 1; next }
    {
      count = split($0, parts, "/")
      archive = parts[count]
      if (archive in required) {
        print parts[count - 1] "/" archive
      }
    }
  ' <(cargo_registry_archives) <(find "$cache_path" -type f -name '*.crate' -print 2>/dev/null) \
    | LC_ALL=C sort \
    | shasum -a 256 \
    | awk '{print $1}'
}

surface_presence_fingerprint() {
  case "$1" in
    cargo) cargo_cache_layout_fingerprint ;;
    *) printf '%s\n' '-' ;;
  esac
}

surface_is_present() {
  case "$1" in
    root-bun) bun_surface_is_present "$repo_root/package.json" "$repo_root/node_modules" ;;
    web-bun) bun_surface_is_present "$repo_root/web/package.json" "$repo_root/web/node_modules" ;;
    docs-bun) bun_surface_is_present "$repo_root/docs-site/package.json" "$repo_root/docs-site/node_modules" ;;
    cargo) cargo_surface_is_present ;;
  esac
}

run_surface() {
  local surface="$1"
  local label directory current_digest previous_status previous_digest previous_presence current_presence
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
  previous_presence="$(state_value "$surface" 4)"
  current_presence="$(surface_presence_fingerprint "$surface" || true)"
  if [ "$surface" != 'cargo' ] && [ -z "$previous_presence" ]; then
    previous_presence='-'
  fi

  if [ "$force" -ne 1 ] && [ "$automatic" -eq 1 ] \
    && [ "$previous_status" = 'failed' ] && [ "$previous_digest" = "$current_digest" ]; then
    printf '[worktree-setup] skipping previously failed %s; run `bun run worktree:setup` to retry\n' "$label" >&2
    return 0
  fi

  if [ "$force" -ne 1 ] && [ "$previous_status" = 'ok' ] \
    && [ "$previous_digest" = "$current_digest" ] \
    && [ -n "$previous_presence" ] \
    && [ "$previous_presence" = "$current_presence" ] \
    && surface_is_present "$surface"; then
    printf '[worktree-setup] %s is up to date\n' "$label"
    return 0
  fi

  printf '[worktree-setup] installing %s\n' "$label"
  if (
    cd "$directory" &&
    "${command[@]}"
  ); then
    current_presence="$(surface_presence_fingerprint "$surface" || true)"
    if [ -z "$current_presence" ]; then
      printf '[worktree-setup] could not verify %s after installation\n' "$label" >&2
      write_state "$surface" failed "$current_digest" '-'
      return 1
    fi
    write_state "$surface" ok "$current_digest" "$current_presence"
    printf '[worktree-setup] installed %s\n' "$label"
    return 0
  fi

  write_state "$surface" failed "$current_digest" '-'
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
