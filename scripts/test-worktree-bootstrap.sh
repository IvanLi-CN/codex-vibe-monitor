#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/codex-vibe-monitor-worktree-bootstrap.XXXXXX")"
tmp_dir="$(cd "$tmp_dir" && pwd)"

cleanup() {
  set +e
  if [ -n "${fixture_repo:-}" ] && [ -n "${worktree_dir:-}" ] && [ -d "$fixture_repo" ]; then
    git -C "$fixture_repo" worktree remove --force "$worktree_dir" >/dev/null 2>&1 || true
  fi
  rm -rf "$tmp_dir" >/dev/null 2>&1 || true
}
trap cleanup EXIT

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
    --exclude 'docs-site/node_modules' \
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
  if ! grep -Fq -- "$needle" "$file"; then
    printf 'expected %s to contain %s\n' "$file" "$needle" >&2
    exit 1
  fi
}

assert_log_order() {
  log_file="$1"
  first="$2"
  second="$3"
  third="$4"
  fourth="$5"
  first_line="$(grep -nF -- "$first" "$log_file" | head -1 | cut -d: -f1)"
  second_line="$(grep -nF -- "$second" "$log_file" | head -1 | cut -d: -f1)"
  third_line="$(grep -nF -- "$third" "$log_file" | head -1 | cut -d: -f1)"
  fourth_line="$(grep -nF -- "$fourth" "$log_file" | head -1 | cut -d: -f1)"
  if [ -z "$first_line" ] || [ -z "$second_line" ] || [ -z "$third_line" ] || [ -z "$fourth_line" ] || [ "$first_line" -ge "$second_line" ] || [ "$second_line" -ge "$third_line" ] || [ "$third_line" -ge "$fourth_line" ]; then
    printf 'expected dependency setup order in %s\n' "$log_file" >&2
    exit 1
  fi
}

resolve_lefthook() {
  if [ -x "$repo_root/node_modules/.bin/lefthook" ]; then
    printf '%s\n' "$repo_root/node_modules/.bin/lefthook"
    return 0
  fi

  command -v lefthook 2>/dev/null
}

write_fake_bun() {
  bin_dir="$1"
  mkdir -p "$bin_dir"
  cat > "$bin_dir/bun" <<'EOF_FAKE'
#!/usr/bin/env bash
set -euo pipefail

printf '%s\t%s\n' "$(pwd)" "$*" >> "${BUN_INSTALL_LOG:?}"
printf '%s\t%s\n' "$(pwd)" "$*" >> "${BOOTSTRAP_ORDER_LOG:?}"
surface=repo
case "$(basename "$(pwd)")" in
  web|docs-site) surface="$(basename "$(pwd)")" ;;
esac

if [ "$surface" = 'web' ]; then
  mkdir -p node_modules/.bin
  printf '#!/usr/bin/env bash\nexit 0\n' > node_modules/.bin/vitest
  chmod +x node_modules/.bin/vitest
fi

if [ "${FAKE_BUN_FAIL_SURFACE:-}" = "$surface" ]; then
  exit 11
fi
EOF_FAKE
  chmod +x "$bin_dir/bun"
}

write_fake_cargo() {
  bin_dir="$1"
  mkdir -p "$bin_dir"
  cat > "$bin_dir/cargo" <<'EOF_FAKE'
#!/usr/bin/env bash
set -euo pipefail

printf '%s\t%s\n' "$(pwd)" "$*" >> "${CARGO_FETCH_LOG:?}"
printf '%s\t%s\n' "$(pwd)" "$*" >> "${BOOTSTRAP_ORDER_LOG:?}"
if [ "${FAKE_CARGO_FAIL:-0}" = '1' ]; then
  exit 12
fi
EOF_FAKE
  chmod +x "$bin_dir/cargo"
}

fixture_repo="$tmp_dir/fixture"
copy_repo "$repo_root" "$fixture_repo"
init_repo "$fixture_repo"

printf 'PRIMARY_SECRET=from-primary\n' > "$fixture_repo/.env.local"

lefthook_bin="$(resolve_lefthook)" || {
  printf 'worktree bootstrap smoke requires lefthook on PATH or repo-local dependencies\n' >&2
  exit 1
}
bun_bin="$(command -v bun 2>/dev/null || true)"
if [ -z "$bun_bin" ]; then
  printf 'worktree bootstrap smoke requires bun on PATH\n' >&2
  exit 1
fi

missing_global_repo="$tmp_dir/missing-global"
mkdir -p "$missing_global_repo/scripts" "$missing_global_repo/node_modules/.bin"
cp "$repo_root/package.json" "$missing_global_repo/package.json"
cp "$repo_root/scripts/install-lefthook-hooks.sh" "$missing_global_repo/scripts/install-lefthook-hooks.sh"
chmod +x "$missing_global_repo/scripts/install-lefthook-hooks.sh"
ln -s "$lefthook_bin" "$missing_global_repo/node_modules/.bin/lefthook"
git -C "$missing_global_repo" init -q
missing_global_output="$tmp_dir/missing-global-output.log"
if (cd "$missing_global_repo" && env PATH=/usr/bin:/bin "$bun_bin" run hooks:install > "$missing_global_output" 2>&1); then
  printf 'hooks:install must reject a repo-local Lefthook without a global binary\n' >&2
  exit 1
fi
assert_file_contains "$missing_global_output" 'global lefthook is required'

masquerade_repo="$tmp_dir/masquerade-global"
masquerade_bin="$tmp_dir/masquerade-bin"
mkdir -p "$masquerade_repo/scripts" "$masquerade_repo/node_modules/.bin" "$masquerade_bin"
cp "$repo_root/package.json" "$masquerade_repo/package.json"
cp "$repo_root/scripts/install-lefthook-hooks.sh" "$masquerade_repo/scripts/install-lefthook-hooks.sh"
chmod +x "$masquerade_repo/scripts/install-lefthook-hooks.sh"
cat > "$masquerade_repo/node_modules/.bin/lefthook-local" <<EOF_LOCAL
#!/usr/bin/env bash
exec "$lefthook_bin" "\$@"
EOF_LOCAL
chmod +x "$masquerade_repo/node_modules/.bin/lefthook-local"
ln -s "$masquerade_repo/node_modules/.bin/lefthook-local" "$masquerade_repo/node_modules/.bin/lefthook"
ln -s "$masquerade_repo/node_modules/.bin/lefthook" "$masquerade_bin/lefthook"
git -C "$masquerade_repo" init -q
masquerade_output="$tmp_dir/masquerade-global-output.log"
if (cd "$masquerade_repo" && env PATH="$masquerade_bin:/usr/bin:/bin" "$bun_bin" run hooks:install > "$masquerade_output" 2>&1); then
  printf 'hooks:install must reject an external symlink to a repo-local Lefthook\n' >&2
  exit 1
fi
assert_file_contains "$masquerade_output" 'global lefthook is required'

fake_bin="$tmp_dir/fake-bin"
global_bin="$tmp_dir/global-bin"
bun_install_log="$tmp_dir/bun-install.log"
cargo_fetch_log="$tmp_dir/cargo-fetch.log"
bootstrap_order_log="$tmp_dir/bootstrap-order.log"
mkdir -p "$global_bin"
cat > "$global_bin/lefthook" <<EOF_GLOBAL
#!/usr/bin/env bash
exec "$lefthook_bin" "\$@"
EOF_GLOBAL
chmod +x "$global_bin/lefthook"
write_fake_bun "$fake_bin"
write_fake_cargo "$fake_bin"
export PATH="$global_bin:$fake_bin:$PATH"
export BUN_INSTALL_LOG="$bun_install_log"
export CARGO_FETCH_LOG="$cargo_fetch_log"
export BOOTSTRAP_ORDER_LOG="$bootstrap_order_log"
export LEFTHOOK_BIN="$global_bin/lefthook"
: > "$bun_install_log"
: > "$cargo_fetch_log"
: > "$bootstrap_order_log"

(
  cd "$fixture_repo"
  "$bun_bin" run hooks:install >/dev/null
)

hooks_dir="$(git -C "$fixture_repo" rev-parse --absolute-git-dir)/hooks"
assert_file_contains "$hooks_dir/post-checkout" 'lefthook'

worktree_dir="$tmp_dir/linked"
git -C "$fixture_repo" worktree add --detach "$worktree_dir" HEAD >/dev/null
assert_file_contains "$worktree_dir/.env.local" 'PRIMARY_SECRET=from-primary'
assert_file_contains "$bun_install_log" "$worktree_dir"$'\t''install --frozen-lockfile'
assert_file_contains "$bun_install_log" "$worktree_dir/web"$'\t''install --frozen-lockfile'
assert_file_contains "$bun_install_log" "$worktree_dir/docs-site"$'\t''install --frozen-lockfile'
assert_file_contains "$cargo_fetch_log" "$worktree_dir"$'\t''fetch --locked'
assert_log_order "$bootstrap_order_log" \
  "$worktree_dir"$'\t''install --frozen-lockfile' \
  "$worktree_dir/web"$'\t''install --frozen-lockfile' \
  "$worktree_dir/docs-site"$'\t''install --frozen-lockfile' \
  "$worktree_dir"$'\t''fetch --locked'
if [ ! -x "$worktree_dir/web/node_modules/.bin/vitest" ]; then
  printf 'linked worktree bootstrap must leave a runnable web Vitest binary\n' >&2
  exit 1
fi

printf 'TARGET_SECRET=keep-me\n' > "$worktree_dir/.env.local"
git -C "$worktree_dir" checkout --detach HEAD >/dev/null 2>&1
assert_file_contains "$worktree_dir/.env.local" 'TARGET_SECRET=keep-me'

: > "$bun_install_log"
: > "$cargo_fetch_log"
: > "$bootstrap_order_log"
(
  cd "$fixture_repo"
  "$hooks_dir/post-checkout" HEAD HEAD 1 >/dev/null 2>&1
)
if [ -s "$bun_install_log" ] || [ -s "$cargo_fetch_log" ]; then
  printf 'primary worktree post-checkout must not install dependencies\n' >&2
  exit 1
fi

(
  cd "$worktree_dir"
  bash scripts/worktree-bootstrap.sh >/dev/null
)
assert_file_contains "$worktree_dir/.env.local" 'TARGET_SECRET=keep-me'

preserve_repo="$tmp_dir/preserve-existing-hook"
copy_repo "$repo_root" "$preserve_repo"
init_repo "$preserve_repo"
printf '#!/bin/sh\n# lefthook is intentionally disabled for this local hook\necho custom-pre-commit\n' > "$preserve_repo/.git/hooks/pre-commit"
chmod +x "$preserve_repo/.git/hooks/pre-commit"
(cd "$preserve_repo" && bash scripts/install-lefthook-hooks.sh >/dev/null)
assert_file_contains "$preserve_repo/.git/hooks/pre-commit" 'custom-pre-commit'
assert_file_contains "$preserve_repo/.git/hooks/post-checkout" 'lefthook'

custom_hooks_repo="$tmp_dir/custom-hooks-path"
copy_repo "$repo_root" "$custom_hooks_repo"
init_repo "$custom_hooks_repo"
mkdir -p "$custom_hooks_repo/custom-hooks"
printf '#!/bin/sh\necho custom-hooks-path\n' > "$custom_hooks_repo/custom-hooks/pre-commit"
chmod +x "$custom_hooks_repo/custom-hooks/pre-commit"
git -C "$custom_hooks_repo" config core.hooksPath custom-hooks
(cd "$custom_hooks_repo" && bash scripts/install-lefthook-hooks.sh >/dev/null)
assert_file_contains "$custom_hooks_repo/custom-hooks/pre-commit" 'custom-hooks-path'

: > "$bun_install_log"
: > "$cargo_fetch_log"
failure_output="$tmp_dir/failure-output.log"
if (
  cd "$worktree_dir"
  FAKE_BUN_FAIL_SURFACE=web FAKE_CARGO_FAIL=1 \
    "$hooks_dir/post-checkout" HEAD HEAD 1 > "$failure_output" 2>&1
); then
  assert_file_contains "$failure_output" 'dependency setup failed'
else
  printf 'post-checkout dependency failures must not fail the hook\n' >&2
  exit 1
fi
assert_file_contains "$bun_install_log" "$worktree_dir"$'\t''install --frozen-lockfile'
assert_file_contains "$bun_install_log" "$worktree_dir/web"$'\t''install --frozen-lockfile'
assert_file_contains "$bun_install_log" "$worktree_dir/docs-site"$'\t''install --frozen-lockfile'
assert_file_contains "$cargo_fetch_log" "$worktree_dir"$'\t''fetch --locked'
assert_file_contains "$failure_output" 'web Bun dependencies'
assert_file_contains "$failure_output" 'Rust dependencies'

if (
  cd "$worktree_dir"
  FAKE_BUN_FAIL_SURFACE=web FAKE_CARGO_FAIL=1 \
    bash scripts/worktree-bootstrap.sh > "$failure_output" 2>&1
); then
  printf 'manual worktree bootstrap must report dependency failures\n' >&2
  exit 1
fi
assert_file_contains "$failure_output" 'dependency setup failed'

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

printf 'worktree bootstrap smoke passed\n'
