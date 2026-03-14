#!/usr/bin/env bash
set -euo pipefail

managed_marker='# managed by codex-vibe-monitor hooks:install'
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
common_dir="$(git -C "$repo_root" rev-parse --git-common-dir)"
case "$common_dir" in
  /*) ;;
  *) common_dir="$repo_root/$common_dir" ;;
esac
hooks_dir="$common_dir/hooks"
custom_hooks_path="$(git -C "$repo_root" config --get core.hooksPath || true)"

if [ -n "$custom_hooks_path" ]; then
  printf '[worktree-bootstrap] core.hooksPath is set to %s; leaving hooks untouched\n' "$custom_hooks_path" >&2
  exit 0
fi

mkdir -p "$hooks_dir"

write_wrapper() {
  hook_name="$1"
  hook_path="$hooks_dir/$hook_name"
  cat > "$hook_path" <<EOF_HOOK
#!/bin/sh
$managed_marker
repo_root="\$(git rev-parse --show-toplevel 2>/dev/null || printf '')"
[ -n "\$repo_root" ] || exit 0
runner="\$repo_root/scripts/run-lefthook-hook.sh"
[ -x "\$runner" ] || exit 0
exec "\$runner" "$hook_name" "\$@"
EOF_HOOK
  chmod +x "$hook_path"
}

write_wrapper pre-commit
write_wrapper commit-msg
write_wrapper post-checkout

printf '[worktree-bootstrap] installed shared hooks in %s\n' "$hooks_dir"
