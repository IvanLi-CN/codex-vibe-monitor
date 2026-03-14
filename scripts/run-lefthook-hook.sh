#!/usr/bin/env bash
set -euo pipefail

hook_name="${1:?hook name is required}"
shift || true

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [ -z "$repo_root" ]; then
  exit 0
fi
cd "$repo_root"

sync_script="$repo_root/scripts/sync-worktree-resources.sh"

if [ "$hook_name" = "post-checkout" ]; then
  if [ -x "$sync_script" ]; then
    "$sync_script" "$@"
  fi
  exit 0
fi

os_arch="$(uname | tr '[:upper:]' '[:lower:]')"
cpu_arch="$(uname -m | sed 's/aarch64/arm64/;s/x86_64/x64/')"

candidate_roots() {
  printf '%s\n' "$repo_root"
  git worktree list --porcelain 2>/dev/null | awk '/^worktree / {print substr($0, 10)}'
}

resolve_lefthook_binary() {
  seen=''
  while IFS= read -r candidate_root; do
    if [ -z "$candidate_root" ] || [ ! -d "$candidate_root" ]; then
      continue
    fi

    case ":$seen:" in
      *":$candidate_root:"*) continue ;;
      *) seen="$seen:$candidate_root" ;;
    esac

    native_bin="$candidate_root/node_modules/lefthook-$os_arch-$cpu_arch/bin/lefthook"
    if [ -x "$native_bin" ]; then
      printf '%s\n' "$native_bin"
      return 0
    fi

    legacy_bin="$candidate_root/node_modules/@evilmartians/lefthook/bin/lefthook-$os_arch-$cpu_arch/lefthook"
    if [ -x "$legacy_bin" ]; then
      printf '%s\n' "$legacy_bin"
      return 0
    fi
  done < <(candidate_roots)

  return 1
}

if lefthook_bin="$(resolve_lefthook_binary)"; then
  exec "$lefthook_bin" run "$hook_name" "$@"
fi

if command -v lefthook >/dev/null 2>&1; then
  exec lefthook run "$hook_name" "$@"
fi

printf '[worktree-bootstrap] lefthook unavailable for %s; skipping.\n' "$hook_name" >&2
