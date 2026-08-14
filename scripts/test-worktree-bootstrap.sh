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

fake_bin="$tmp_dir/fake-bin"
bun_install_log="$tmp_dir/bun-install.log"
cargo_fetch_log="$tmp_dir/cargo-fetch.log"
write_fake_bun "$fake_bin"
write_fake_cargo "$fake_bin"
export PATH="$fake_bin:$PATH"
export BUN_INSTALL_LOG="$bun_install_log"
export CARGO_FETCH_LOG="$cargo_fetch_log"
export LEFTHOOK_BIN="$lefthook_bin"
: > "$bun_install_log"
: > "$cargo_fetch_log"

(
  cd "$fixture_repo"
  "$lefthook_bin" install >/dev/null
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
if [ ! -x "$worktree_dir/web/node_modules/.bin/vitest" ]; then
  printf 'linked worktree bootstrap must leave a runnable web Vitest binary\n' >&2
  exit 1
fi

printf 'TARGET_SECRET=keep-me\n' > "$worktree_dir/.env.local"
git -C "$worktree_dir" checkout --detach HEAD >/dev/null 2>&1
assert_file_contains "$worktree_dir/.env.local" 'TARGET_SECRET=keep-me'

: > "$bun_install_log"
: > "$cargo_fetch_log"
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
