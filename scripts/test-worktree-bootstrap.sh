#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/codex-vibe-monitor-worktree-bootstrap.XXXXXX")"
tmp_dir="$(cd "$tmp_dir" && pwd -P)"
cleanup() {
  set +e
  for worktree in "${worktree_one:-}" "${worktree_two:-}"; do
    [ -n "$worktree" ] && git -C "${fixture_repo:-$tmp_dir}" worktree remove --force "$worktree" >/dev/null 2>&1 || true
  done
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

fail() {
  printf '%s\n' "$*" >&2
  exit 1
}

assert_contains() {
  grep -Fq -- "$2" "$1" || fail "expected $1 to contain $2"
}

assert_not_contains() {
  if grep -Fq -- "$2" "$1"; then
    fail "expected $1 not to contain $2"
  fi
}

assert_log_only_surface() {
  expected_directory="$1"
  expected_cargo="$2"
  if [ "$expected_directory" = 'none' ]; then
    [ ! -s "$bun_install_log" ] || fail 'unexpected Bun setup call'
  else
    assert_contains "$bun_install_log" "$expected_directory"$'\t''install --frozen-lockfile'
    [ "$(wc -l < "$bun_install_log" | tr -d ' ')" = '1' ] || fail 'expected exactly one Bun setup call'
  fi
  if [ "$expected_cargo" = 'yes' ]; then
    assert_contains "$cargo_fetch_log" $'\t''fetch --locked'
  else
    [ ! -s "$cargo_fetch_log" ] || fail 'unexpected Cargo setup call'
  fi
}

copy_repo() {
  source_repo="$1"
  destination="$2"
  mkdir -p "$destination"
  rsync -a \
    --exclude '.git' \
    --exclude '.env.local' \
    --exclude 'web/.env.local' \
    --exclude 'node_modules' \
    --exclude 'web/node_modules' \
    --exclude 'docs-site/node_modules' \
    --exclude 'target' \
    --exclude 'web/dist' \
    "$source_repo/" "$destination/"
}

init_repo() {
  git -C "$1" init -qb main
  git -C "$1" config user.name 'Codex Test'
  git -C "$1" config user.email 'codex-test@example.com'
  git -C "$1" add .
  LEFTHOOK=0 git -C "$1" commit -qm 'fixture'
}

write_fake_bun() {
  mkdir -p "$1"
  cat > "$1/bun" <<'EOF_BUN'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\t%s\n' "$(pwd)" "$*" >> "${BUN_INSTALL_LOG:?}"
surface=repo
case "$(basename "$(pwd)")" in
  web|docs-site) surface="$(basename "$(pwd)")" ;;
esac
mkdir -p node_modules
mkdir -p node_modules/.bin
case "$surface" in
  repo) executable=biome ;;
  web) executable=vitest ;;
  docs-site) executable=rspress ;;
esac
printf '#!/usr/bin/env bash\nexit 0\n' > "node_modules/.bin/$executable"
chmod +x "node_modules/.bin/$executable"
if [ "${FAKE_BUN_FAIL_SURFACE:-}" = "$surface" ]; then
  exit 11
fi
EOF_BUN
  chmod +x "$1/bun"
}

write_fake_cargo() {
  mkdir -p "$1"
  cat > "$1/cargo" <<'EOF_CARGO'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\t%s\n' "$(pwd)" "$*" >> "${CARGO_FETCH_LOG:?}"
EOF_CARGO
  chmod +x "$1/cargo"
}

hold_advisory_lock() {
  lock_path="$1"
  ready_path="$2"
  rm -f "$ready_path"
  perl -MFcntl=:flock -e '
    my ($path, $ready) = @ARGV;
    open my $lock, ">>", $path or die "open lock: $!";
    flock($lock, LOCK_EX) or die "flock: $!";
    open my $signal, ">", $ready or die "open ready: $!";
    print {$signal} "ready\n";
    sleep 2;
  ' "$lock_path" "$ready_path" &
  lock_holder_pid=$!
  for attempt in {1..50}; do
    [ -f "$ready_path" ] && return 0
    sleep 0.05
  done
  fail 'advisory lock holder did not become ready'
}

release_advisory_lock() {
  wait "${lock_holder_pid:?missing advisory lock holder}"
  unset lock_holder_pid
}

bun_bin="$(command -v bun 2>/dev/null || true)"
global_lefthook="$(command -v lefthook 2>/dev/null || true)"
[ -n "$bun_bin" ] || fail 'worktree bootstrap smoke requires bun on PATH'
if [ -z "$global_lefthook" ] && [ -x "$repo_root/node_modules/.bin/lefthook" ]; then
  global_lefthook="$repo_root/node_modules/.bin/lefthook"
fi
[ -n "$global_lefthook" ] || fail 'worktree bootstrap smoke requires a Lefthook binary'

fixture_repo="$tmp_dir/fixture"
copy_repo "$repo_root" "$fixture_repo"
init_repo "$fixture_repo"
printf 'PRIMARY_SECRET=from-primary\n' > "$fixture_repo/.env.local"

fake_bin="$tmp_dir/fake-bin"
write_fake_bun "$fake_bin"
write_fake_cargo "$fake_bin"
bun_install_log="$tmp_dir/bun-install.log"
cargo_fetch_log="$tmp_dir/cargo-fetch.log"
export BUN_INSTALL_LOG="$bun_install_log"
export CARGO_FETCH_LOG="$cargo_fetch_log"
export PATH="$fake_bin:$(dirname "$global_lefthook"):$PATH"
: > "$bun_install_log"
: > "$cargo_fetch_log"

(
  cd "$fixture_repo"
  "$bun_bin" run hooks:install >/dev/null
)
hooks_dir="$(git -C "$fixture_repo" rev-parse --absolute-git-dir)/hooks"
for hook_name in pre-commit commit-msg post-checkout; do
  [ -f "$hooks_dir/$hook_name" ] || fail "missing installed $hook_name hook"
done

: > "$bun_install_log"
: > "$cargo_fetch_log"
(
  cd "$fixture_repo"
  bash scripts/run-lefthook-hook.sh post-checkout HEAD HEAD 1 >/dev/null
)
assert_log_only_surface none no

worktree_one="$tmp_dir/linked-one"
git -C "$fixture_repo" worktree add --detach "$worktree_one" HEAD >/dev/null
assert_contains "$worktree_one/.env.local" 'PRIMARY_SECRET=from-primary'
for directory in "$worktree_one" "$worktree_one/web" "$worktree_one/docs-site"; do
  assert_contains "$bun_install_log" "$directory"$'\t''install --frozen-lockfile'
done
assert_contains "$cargo_fetch_log" "$worktree_one"$'\t''fetch --locked'
[ -x "$worktree_one/web/node_modules/.bin/vitest" ] || fail 'web dependency sentinel is missing'
state_path="$(git -C "$worktree_one" rev-parse --git-path worktree-setup-state-v1)"
case "$state_path" in
  /*) ;;
  *) state_path="$worktree_one/$state_path" ;;
esac
[ -f "$state_path" ] || fail 'per-worktree setup state was not recorded'
assert_not_contains "$state_path" 'PRIMARY_SECRET'

: > "$bun_install_log"
: > "$cargo_fetch_log"
git -C "$worktree_one" checkout --detach HEAD >/dev/null 2>&1
assert_log_only_surface none no

rm -rf "$worktree_one/web/node_modules"
: > "$bun_install_log"
: > "$cargo_fetch_log"
(
  cd "$worktree_one"
  bash scripts/run-lefthook-hook.sh post-checkout HEAD HEAD 1 >/dev/null
)
assert_log_only_surface "$worktree_one/web" no

: > "$bun_install_log"
: > "$cargo_fetch_log"
(
  cd "$worktree_one"
  bash scripts/worktree-setup.sh --force >/dev/null
)
for directory in "$worktree_one" "$worktree_one/web" "$worktree_one/docs-site"; do
  assert_contains "$bun_install_log" "$directory"$'\t''install --frozen-lockfile'
done
assert_contains "$cargo_fetch_log" "$worktree_one"$'\t''fetch --locked'

rm -rf "$worktree_one/node_modules"
mkdir -p "$worktree_one/node_modules"
: > "$bun_install_log"
: > "$cargo_fetch_log"
(
  cd "$worktree_one"
  bash scripts/run-lefthook-hook.sh post-checkout HEAD HEAD 1 >/dev/null
)
assert_log_only_surface "$worktree_one" no

rm -rf "$worktree_one/docs-site/node_modules"
mkdir -p "$worktree_one/docs-site/node_modules"
: > "$bun_install_log"
: > "$cargo_fetch_log"
(
  cd "$worktree_one"
  bash scripts/run-lefthook-hook.sh post-checkout HEAD HEAD 1 >/dev/null
)
assert_log_only_surface "$worktree_one/docs-site" no

printf '\n# fingerprint-only fixture change\n' >> "$worktree_one/Cargo.toml"
: > "$bun_install_log"
: > "$cargo_fetch_log"
(
  cd "$worktree_one"
  bash scripts/run-lefthook-hook.sh post-checkout HEAD HEAD 1 >/dev/null
)
assert_log_only_surface none yes

rm -rf "$worktree_one/web/node_modules"
: > "$bun_install_log"
: > "$cargo_fetch_log"
(
  cd "$worktree_one"
  FAKE_BUN_FAIL_SURFACE=web bash scripts/run-lefthook-hook.sh post-checkout HEAD HEAD 1 >/dev/null
)
assert_log_only_surface "$worktree_one/web" no
: > "$bun_install_log"
: > "$cargo_fetch_log"
(
  cd "$worktree_one"
  FAKE_BUN_FAIL_SURFACE=web bash scripts/run-lefthook-hook.sh post-checkout HEAD HEAD 1 >/dev/null
)
assert_log_only_surface none no

: > "$bun_install_log"
: > "$cargo_fetch_log"
(
  cd "$worktree_one"
  bash scripts/worktree-setup.sh >/dev/null
)
assert_log_only_surface "$worktree_one/web" no

printf '\n# fingerprint-only fixture change\n' >> "$worktree_one/web/bun.lock"
: > "$bun_install_log"
: > "$cargo_fetch_log"
(
  cd "$worktree_one"
  bash scripts/run-lefthook-hook.sh post-checkout HEAD HEAD 1 >/dev/null
)
assert_log_only_surface "$worktree_one/web" no

worktree_two="$tmp_dir/linked-two"
git -C "$fixture_repo" worktree add --detach "$worktree_two" HEAD >/dev/null
first_git_dir="$(git -C "$worktree_one" rev-parse --git-dir)"
second_git_dir="$(git -C "$worktree_two" rev-parse --git-dir)"
case "$first_git_dir" in /*) ;; *) first_git_dir="$worktree_one/$first_git_dir" ;; esac
case "$second_git_dir" in /*) ;; *) second_git_dir="$worktree_two/$second_git_dir" ;; esac
first_lock="$first_git_dir/worktree-bootstrap-sync.flock"
second_lock="$second_git_dir/worktree-bootstrap-sync.flock"
[ "$first_lock" != "$second_lock" ] || fail 'resource locks must be per-worktree'
rm -f "$worktree_one/.env.local" "$worktree_two/.env.local"
hold_advisory_lock "$first_lock" "$tmp_dir/first-sync-lock.ready"
(
  cd "$worktree_one"
  bash scripts/sync-worktree-resources.sh > "$tmp_dir/first-lock.log"
)
assert_contains "$tmp_dir/first-lock.log" 'sync lock is busy; skipping resource sync'
(
  cd "$worktree_two"
  bash scripts/sync-worktree-resources.sh >/dev/null
)
assert_contains "$worktree_two/.env.local" 'PRIMARY_SECRET=from-primary'
release_advisory_lock
(
  cd "$worktree_one"
  bash scripts/sync-worktree-resources.sh >/dev/null
)
assert_contains "$worktree_one/.env.local" 'PRIMARY_SECRET=from-primary'
[ -f "$first_lock" ] || fail 'advisory resource lock file was not retained'

setup_lock="$(dirname "$state_path")/worktree-setup.flock"
rm -rf "$worktree_one/docs-site/node_modules"
: > "$bun_install_log"
: > "$cargo_fetch_log"
hold_advisory_lock "$setup_lock" "$tmp_dir/setup-lock.ready"
(
  cd "$worktree_one"
  bash scripts/run-lefthook-hook.sh post-checkout HEAD HEAD 1 >/dev/null
)
assert_log_only_surface none no
release_advisory_lock
(
  cd "$worktree_one"
  bash scripts/run-lefthook-hook.sh post-checkout HEAD HEAD 1 >/dev/null
)
assert_log_only_surface "$worktree_one/docs-site" no
printf 'TARGET_SECRET=keep-me\n' > "$worktree_one/.env.local"
(
  cd "$worktree_one"
  bash scripts/sync-worktree-resources.sh >/dev/null
)
assert_contains "$worktree_one/.env.local" 'TARGET_SECRET=keep-me'

printf 'worktree bootstrap smoke passed\n'
