#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/codex-vibe-monitor-worktree-bootstrap.XXXXXX")"
trap 'rm -rf "$tmp_dir"' EXIT

copy_repo() {
  src="$1"
  dest="$2"
  mkdir -p "$dest"
  rsync -a \
    --exclude '.git' \
    --exclude '.env.local' \
    --exclude 'web/.env.local' \
    --exclude 'node_modules' \
    --exclude 'web/node_modules' \
    --exclude 'target' \
    --exclude 'web/dist' \
    --exclude '.codex/logs' \
    --exclude '.codex/evidence' \
    "$src/" "$dest/"
}

init_repo() {
  repo="$1"
  git -C "$repo" init -b main >/dev/null
  git -C "$repo" config user.name 'Codex Test'
  git -C "$repo" config user.email 'codex-test@example.com'
  git -C "$repo" add .
  LEFTHOOK=0 git -C "$repo" commit -m 'test fixture' >/dev/null
}

assert_file_contains() {
  file="$1"
  needle="$2"
  if ! grep -Fq "$needle" "$file"; then
    printf 'expected %s to contain %s\n' "$file" "$needle" >&2
    exit 1
  fi
}

assert_equal_file() {
  expected="$1"
  actual="$2"
  if ! cmp -s "$expected" "$actual"; then
    printf 'expected %s to match %s\n' "$actual" "$expected" >&2
    exit 1
  fi
}

write_fake_lefthook() {
  repo="$1"
  os_arch="$(uname | tr '[:upper:]' '[:lower:]')"
  cpu_arch="$(uname -m | sed 's/aarch64/arm64/;s/x86_64/x64/')"
  native_dir="$repo/node_modules/lefthook-$os_arch-$cpu_arch/bin"
  mkdir -p "$native_dir"
  cat > "$native_dir/lefthook" <<'EOF_FAKE'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" > .lefthook-run.log
EOF_FAKE
  chmod +x "$native_dir/lefthook"
}

fixture_repo="$tmp_dir/fixture"
copy_repo "$repo_root" "$fixture_repo"
init_repo "$fixture_repo"

printf 'PRIMARY_SECRET=from-primary\n' > "$fixture_repo/.env.local"
write_fake_lefthook "$fixture_repo"

install_output="$(bash "$fixture_repo/scripts/install-hooks.sh" 2>&1)"
assert_file_contains <(printf '%s' "$install_output") 'installed shared hooks'

hooks_dir="$(git -C "$fixture_repo" rev-parse --absolute-git-dir)/hooks"
assert_file_contains "$hooks_dir/pre-commit" '# managed by codex-vibe-monitor hooks:install'
assert_file_contains "$hooks_dir/commit-msg" '# managed by codex-vibe-monitor hooks:install'
assert_file_contains "$hooks_dir/post-checkout" '# managed by codex-vibe-monitor hooks:install'

worktree_dir="$tmp_dir/linked"
git -C "$fixture_repo" worktree add --detach "$worktree_dir" HEAD >/dev/null
assert_equal_file "$fixture_repo/.env.local" "$worktree_dir/.env.local"

printf 'TARGET_SECRET=keep-me\n' > "$worktree_dir/.env.local"
git -C "$worktree_dir" checkout --detach HEAD >/dev/null 2>&1
assert_file_contains "$worktree_dir/.env.local" 'TARGET_SECRET=keep-me'

rm -rf "$worktree_dir/node_modules"
(
  cd "$worktree_dir"
  "$hooks_dir/pre-commit" >/dev/null
)
assert_file_contains "$worktree_dir/.lefthook-run.log" 'run pre-commit'

bash "$worktree_dir/scripts/worktree-bootstrap.sh" >/dev/null
assert_file_contains "$worktree_dir/.env.local" 'TARGET_SECRET=keep-me'

rm -f "$fixture_repo/scripts/run-lefthook-hook.sh" \
  "$fixture_repo/scripts/sync-worktree-resources.sh" \
  "$fixture_repo/scripts/worktree-bootstrap.sh" \
  "$fixture_repo/scripts/worktree-sync.paths"
git -C "$fixture_repo" add -A
LEFTHOOK=0 git -C "$fixture_repo" commit -m 'legacy fixture without bootstrap scripts' >/dev/null
legacy_sha="$(git -C "$fixture_repo" rev-parse HEAD)"
head_sha="$(git -C "$fixture_repo" rev-parse HEAD^)"

git -C "$worktree_dir" checkout --detach "$legacy_sha" >/dev/null
git -C "$worktree_dir" checkout --detach "$head_sha" >/dev/null
assert_file_contains "$worktree_dir/.env.local" 'TARGET_SECRET=keep-me'

preserve_repo="$tmp_dir/preserve-existing-hook"
copy_repo "$repo_root" "$preserve_repo"
init_repo "$preserve_repo"
printf '#!/bin/sh\necho custom-pre-commit\n' > "$preserve_repo/.git/hooks/pre-commit"
chmod +x "$preserve_repo/.git/hooks/pre-commit"
preserve_output="$(bash "$preserve_repo/scripts/install-hooks.sh" 2>&1)"
assert_file_contains <(printf '%s' "$preserve_output") 'pre-commit already exists and is unmanaged'
assert_file_contains "$preserve_repo/.git/hooks/pre-commit" 'custom-pre-commit'
assert_file_contains "$preserve_repo/.git/hooks/post-checkout" '# managed by codex-vibe-monitor hooks:install'

path_repo="$tmp_dir/path-resolution"
copy_repo "$repo_root" "$path_repo"
init_repo "$path_repo"
caller_dir="$tmp_dir/outside-caller"
mkdir -p "$caller_dir"
printf 'CALLER_SECRET=outside\n' > "$caller_dir/.env.local"
(
  cd "$caller_dir"
  bash "$path_repo/scripts/sync-worktree-resources.sh" >/dev/null
)
if [ -e "$path_repo/.env.local" ]; then
  printf 'sync script must not resolve source_root from the caller working directory\n' >&2
  exit 1
fi

custom_repo="$tmp_dir/custom-hooks"
copy_repo "$repo_root" "$custom_repo"
init_repo "$custom_repo"
git -C "$custom_repo" config core.hooksPath .custom-hooks
mkdir -p "$custom_repo/.custom-hooks"
custom_output="$(bash "$custom_repo/scripts/install-hooks.sh" 2>&1)"
assert_file_contains <(printf '%s' "$custom_output") 'core.hooksPath is set'
if [ -e "$custom_repo/.custom-hooks/post-checkout" ]; then
  printf 'custom hooks path should remain untouched\n' >&2
  exit 1
fi

printf 'worktree bootstrap smoke passed\n'
