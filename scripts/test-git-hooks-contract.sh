#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/codex-vibe-monitor-git-hooks.XXXXXX")"
tmp_dir="$(cd "$tmp_dir" && pwd -P)"
cleanup() {
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

for surface in web markdown rust; do
  assert_contains "$repo_root/lefthook.yml" "name: format-$surface"
done
assert_contains "$repo_root/lefthook.yml" 'name: check-rust-line-budget'
assert_contains "$repo_root/lefthook.yml" 'check_staged_rust_line_budget.py'
assert_contains "$repo_root/lefthook.yml" '{staged_files}'
assert_contains "$repo_root/lefthook.yml" '"web/*.{js,ts,cjs,mjs,d.cts,d.mts,jsx,tsx,json,jsonc}"'
assert_contains "$repo_root/lefthook.yml" '"*.md"'
assert_contains "$repo_root/lefthook.yml" '"**/*.md"'
assert_contains "$repo_root/lefthook.yml" 'parallel: false'
[ "$(sed -n '2p' "$repo_root/lefthook.yml")" = '  parallel: false' ] \
  || fail 'staged formatter jobs must serialize their shared index writes'
assert_not_contains "$repo_root/lefthook.yml" 'stage_fixed: true'
assert_not_contains "$repo_root/lefthook.yml" 'cargo '
assert_not_contains "$repo_root/lefthook.yml" 'cargo clippy'
assert_not_contains "$repo_root/lefthook.yml" 'tsc -b'
assert_not_contains "$repo_root/lefthook.yml" 'bun run lint:web'
assert_not_contains "$repo_root/scripts/install-lefthook-hooks.sh" 'lefthook uninstall'
assert_not_contains "$repo_root/scripts/format-staged-files.sh" 'lefthook-unstaged.patch'
assert_contains "$repo_root/scripts/check-staged-formatter-safety.sh" 'format-staged-files.sh" --check'
assert_contains "$repo_root/scripts/run-pre-commit-hook.sh" 'pre-commit.flock'
assert_contains "$repo_root/scripts/run-pre-commit-hook.sh" "'--no-auto-install'"

fake_bin="$tmp_dir/fake-bin"
formatter_log="$tmp_dir/formatter.log"
mkdir -p "$fake_bin"
for formatter in biome rustfmt dprint; do
  cat > "$fake_bin/$formatter" <<'EOF_FORMATTER'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\t%s\n' "$(basename "$0")" "$*" >> "${FORMATTER_LOG:?}"
case "$(basename "$0")" in
  biome|dprint) cat ;;
esac
EOF_FORMATTER
  chmod +x "$fake_bin/$formatter"
done
export FORMATTER_LOG="$formatter_log"

(
  cd "$repo_root"
  CODEX_HOOK_BIOME_BIN="$fake_bin/biome" \
    bash scripts/format-staged-files.sh web web/src/test-setup.ts missing.ts web/../../package.json web/../package.json
  CODEX_HOOK_RUSTFMT_BIN="$fake_bin/rustfmt" \
    bash scripts/format-staged-files.sh rust src/main.rs deleted.rs vendor/hotpath/src/lib_on/sql/normalize.rs
  CODEX_HOOK_DPRINT_BIN="$fake_bin/dprint" \
    bash scripts/format-staged-files.sh markdown README.md removed.md docs/../README.md vendor/hotpath/README.md
)
assert_contains "$formatter_log" $'biome\tcheck --write --stdin-file-path=web/src/test-setup.ts'
assert_contains "$formatter_log" $'rustfmt\t--edition 2024 --config skip_children=true -- '
assert_contains "$formatter_log" '/src/main.rs'
assert_contains "$formatter_log" $'dprint\tfmt --stdin README.md'
assert_not_contains "$formatter_log" 'missing.ts'
assert_not_contains "$formatter_log" '../../package.json'
assert_not_contains "$formatter_log" 'web/../package.json'
assert_not_contains "$formatter_log" 'deleted.rs'
assert_not_contains "$formatter_log" 'removed.md'
assert_not_contains "$formatter_log" 'docs/../README.md'
assert_not_contains "$formatter_log" 'vendor/hotpath/'

lefthook_bin="$(command -v lefthook 2>/dev/null || true)"
[ -n "$lefthook_bin" ] || fail 'git hook contract smoke requires an external Lefthook binary'
real_git="$(command -v git)"
case "$(cd "$(dirname "$lefthook_bin")" && pwd -P)" in
  "$repo_root"|"$repo_root"/*) fail 'git hook contract smoke cannot use repo-local Lefthook' ;;
esac

partial_repo="$tmp_dir/partial-stage"
mkdir -p "$partial_repo/scripts" "$partial_repo/src" "$partial_repo/.github/scripts" "$partial_repo/web"
cp "$repo_root/lefthook.yml" "$partial_repo/lefthook.yml"
cp "$repo_root/scripts/format-staged-files.sh" "$partial_repo/scripts/format-staged-files.sh"
cp "$repo_root/scripts/check-staged-formatter-safety.sh" "$partial_repo/scripts/check-staged-formatter-safety.sh"
cp "$repo_root/scripts/run-pre-commit-hook.sh" "$partial_repo/scripts/run-pre-commit-hook.sh"
cp "$repo_root/scripts/install-lefthook-hooks.sh" "$partial_repo/scripts/install-lefthook-hooks.sh"
cp "$repo_root/.github/scripts/check_staged_rust_line_budget.py" "$partial_repo/.github/scripts/check_staged_rust_line_budget.py"
cat > "$partial_repo/.github/rust-source-quality-policy.json" <<'EOF_POLICY'
{
  "files": [
    {
      "path": "src/sample.rs",
      "line_budget": 2
    }
  ]
}
EOF_POLICY
chmod +x "$partial_repo/scripts/format-staged-files.sh"
chmod +x "$partial_repo/scripts/check-staged-formatter-safety.sh"
chmod +x "$partial_repo/scripts/run-pre-commit-hook.sh"
chmod +x "$partial_repo/scripts/install-lefthook-hooks.sh"
cat > "$partial_repo/src/sample.rs" <<'EOF_RUST'
fn main() {
    println!("base");
}
EOF_RUST
cat > "$partial_repo/src/module_root.rs" <<'EOF_MODULE_ROOT'
mod child;

fn main(){ child::run(); }
EOF_MODULE_ROOT
cat > "$partial_repo/src/child.rs" <<'EOF_MODULE_CHILD'
pub fn run(){ println!("child"); }
EOF_MODULE_CHILD
printf '# fixture README\n' > "$partial_repo/README.md"
printf '{"name":"fixture"}\n' > "$partial_repo/web/package.json"
git -C "$partial_repo" init -q
git -C "$partial_repo" config user.name 'Codex Test'
git -C "$partial_repo" config user.email 'codex-test@example.com'
git -C "$partial_repo" add .
git -C "$partial_repo" commit -qm 'fixture'
(
  cd "$partial_repo"
  CODEX_HOOK_RUSTFMT_BIN=rustfmt bash scripts/format-staged-files.sh rust src/module_root.rs
)
grep -Fqx 'fn main() {' "$partial_repo/src/module_root.rs" \
  || fail 'rustfmt must format a module root without resolving temporary children'
root_glob_formatter_log="$tmp_dir/root-glob-formatter.log"
: > "$root_glob_formatter_log"
(
  cd "$partial_repo"
  FORMATTER_LOG="$root_glob_formatter_log" \
    CODEX_HOOK_BIOME_BIN="$fake_bin/biome" CODEX_HOOK_DPRINT_BIN="$fake_bin/dprint" \
    CODEX_HOOK_RUSTFMT_BIN="$fake_bin/rustfmt" \
    "$lefthook_bin" run pre-commit --all-files --no-auto-install --no-tty \
    > "$tmp_dir/root-glob.log" 2>&1
)
assert_contains "$root_glob_formatter_log" $'biome\tcheck --write --stdin-file-path=web/package.json'
assert_contains "$root_glob_formatter_log" $'dprint\tfmt --stdin README.md'
assert_contains "$root_glob_formatter_log" $'rustfmt\t--edition 2024 --config skip_children=true -- '
assert_contains "$root_glob_formatter_log" '/src/module_root.rs'
cat > "$partial_repo/-evil.rs" <<'EOF_DASH_RUST'
fn main() {}
EOF_DASH_RUST
git -C "$partial_repo" add -- -evil.rs
cat > "$fake_bin/option-guard-rustfmt" <<'EOF_OPTION_GUARD_RUSTFMT'
#!/usr/bin/env bash
set -euo pipefail
[ "${1:-}" = '--edition' ] && shift
[ "${1:-}" = '2024' ] && shift
[ "${1:-}" = '--config' ] && shift
[ "${1:-}" = 'skip_children=true' ] && shift
[ "${1:-}" = '--' ] && shift
case "${1:-}" in
  */-evil.rs) ;;
  *) exit 1 ;;
esac
cat "$1"
EOF_OPTION_GUARD_RUSTFMT
chmod +x "$fake_bin/option-guard-rustfmt"
(
  cd "$partial_repo"
  CODEX_HOOK_RUSTFMT_BIN="$fake_bin/option-guard-rustfmt" bash scripts/format-staged-files.sh rust -evil.rs
)
rm -f "$partial_repo/-evil.rs"
git -C "$partial_repo" rm -q --cached -- -evil.rs
(
  cd "$partial_repo"
  PATH="$(dirname "$lefthook_bin"):$PATH" bash scripts/install-lefthook-hooks.sh >/dev/null
)
partial_pre_commit_hook="$partial_repo/.git/hooks/pre-commit"
assert_contains "$partial_pre_commit_hook" 'bash scripts/run-pre-commit-hook.sh'
cat > "$partial_repo/src/sample.rs" <<'EOF_STAGED'
fn main(){ println!("staged"); }
EOF_STAGED
git -C "$partial_repo" diff > "$tmp_dir/staged.patch"
cat > "$partial_repo/src/sample.rs" <<'EOF_WORKTREE'
fn main(){ println!("staged"); }
// unstaged hunk must survive
EOF_WORKTREE
git -C "$partial_repo" apply --cached "$tmp_dir/staged.patch"
cat > "$fake_bin/partial-rustfmt" <<'EOF_PARTIAL_RUSTFMT'
#!/usr/bin/env bash
set -euo pipefail
format_path=''
for path in "$@"; do
  case "$path" in
    --edition|2024|--config|skip_children=true|--) continue ;;
  esac
  [ -f "$path" ] || continue
  format_path="$path"
  perl -0pi -e 's/fn main\(\)\{ println!\("staged"\); \}/fn main() {\n    println!("staged");\n}/' "$path"
done
cat "$format_path"
EOF_PARTIAL_RUSTFMT
chmod +x "$fake_bin/partial-rustfmt"
if (
  cd "$partial_repo"
  LEFTHOOK_BIN="$lefthook_bin" CODEX_HOOK_RUSTFMT_BIN="$fake_bin/partial-rustfmt" "$partial_pre_commit_hook" \
    > "$tmp_dir/partial-stage.log" 2>&1
); then
  fail 'partially staged formatter rewrite must fail before modifying the worktree'
fi
git -C "$partial_repo" show :src/sample.rs > "$tmp_dir/index.contents"
cp "$partial_repo/src/sample.rs" "$tmp_dir/worktree.contents"
cat > "$tmp_dir/expected-index.contents" <<'EOF_PARTIAL_INDEX'
fn main(){ println!("staged"); }
EOF_PARTIAL_INDEX
cat > "$tmp_dir/expected-worktree.contents" <<'EOF_PARTIAL_WORKTREE'
fn main(){ println!("staged"); }
// unstaged hunk must survive
EOF_PARTIAL_WORKTREE
assert_contains "$tmp_dir/partial-stage.log" 'refusing to auto-format partially staged file: src/sample.rs'
cmp -s "$tmp_dir/expected-index.contents" "$tmp_dir/index.contents" \
  || fail 'partial-stage guard modified the index'
cmp -s "$tmp_dir/expected-worktree.contents" "$tmp_dir/worktree.contents" \
  || fail 'partial-stage guard modified the worktree'

cat > "$partial_repo/src/sample.rs" <<'EOF_LOCK_STAGE'
fn main() {
    println!("staged");
}
EOF_LOCK_STAGE
git -C "$partial_repo" add src/sample.rs

cat > "$fake_bin/lock-lefthook" <<'EOF_LOCK_LEFTHOOK'
#!/usr/bin/env bash
set -euo pipefail
[ "${1:-}" = 'run' ]
[ "${2:-}" = 'pre-commit' ]
[ "${3:-}" = '--no-auto-install' ]
printf 'start\n' >> "${LOCK_TEST_LOG:?}"
sleep 0.4
printf 'end\n' >> "${LOCK_TEST_LOG:?}"
EOF_LOCK_LEFTHOOK
chmod +x "$fake_bin/lock-lefthook"
lock_test_log="$tmp_dir/pre-commit-lock.log"
(
  cd "$partial_repo"
  LEFTHOOK_BIN="$fake_bin/lock-lefthook" LOCK_TEST_LOG="$lock_test_log" bash scripts/run-pre-commit-hook.sh
) &
first_lock_pid=$!
for attempt in {1..40}; do
  grep -Fqx 'start' "$lock_test_log" 2>/dev/null && break
  sleep 0.05
done
(
  cd "$partial_repo"
  LEFTHOOK_BIN="$fake_bin/lock-lefthook" LOCK_TEST_LOG="$lock_test_log" bash scripts/run-pre-commit-hook.sh
) &
second_lock_pid=$!
wait "$first_lock_pid"
wait "$second_lock_pid"
awk 'NR == 1 && $0 == "start" { next } NR == 2 && $0 == "end" { next } NR == 3 && $0 == "start" { next } NR == 4 && $0 == "end" { found = 1 } END { exit found ? 0 : 1 }' "$lock_test_log" \
  || fail 'concurrent pre-commit invocations were not serialized'
partial_git_dir="$(git -C "$partial_repo" rev-parse --absolute-git-dir)"
pre_commit_lock_path="$partial_git_dir/pre-commit.flock"
(
  cd "$partial_repo"
  LEFTHOOK=false LEFTHOOK_BIN="$fake_bin/lock-lefthook" LOCK_TEST_LOG="$lock_test_log" bash scripts/run-pre-commit-hook.sh
)
outside_lock="$tmp_dir/outside-pre-commit.lock"
printf 'outside lock sentinel\n' > "$outside_lock"
rm -f "$pre_commit_lock_path"
ln -s "$outside_lock" "$pre_commit_lock_path"
if (
  cd "$partial_repo"
  LEFTHOOK_BIN="$fake_bin/lock-lefthook" LOCK_TEST_LOG="$lock_test_log" bash scripts/run-pre-commit-hook.sh \
    > "$tmp_dir/symlink-lock.log" 2>&1
); then
  fail 'symlinked pre-commit lock must be rejected'
fi
assert_contains "$tmp_dir/symlink-lock.log" 'pre-commit lock must not be a symlink'
assert_contains "$outside_lock" 'outside lock sentinel'
rm -f "$pre_commit_lock_path"
mkfifo "$pre_commit_lock_path"
(
  cd "$partial_repo"
  LEFTHOOK_BIN="$fake_bin/lock-lefthook" LOCK_TEST_LOG="$lock_test_log" bash scripts/run-pre-commit-hook.sh \
    > "$tmp_dir/fifo-lock.log" 2>&1
) &
fifo_lock_pid=$!
sleep 0.2
if kill -0 "$fifo_lock_pid" 2>/dev/null; then
  kill "$fifo_lock_pid" 2>/dev/null || true
  wait "$fifo_lock_pid" 2>/dev/null || true
  fail 'FIFO pre-commit lock must not block hook startup'
fi
wait "$fifo_lock_pid" 2>/dev/null || true
rm -f "$pre_commit_lock_path"
cat > "$partial_repo/src/sample.rs" <<'EOF_FULL_STAGE'
fn main(){ println!("staged"); }
EOF_FULL_STAGE
git -C "$partial_repo" add src/sample.rs
if (
  cd "$partial_repo"
  LEFTHOOK_BIN="$lefthook_bin" CODEX_HOOK_RUSTFMT_BIN="$fake_bin/partial-rustfmt" "$partial_pre_commit_hook" \
    > "$tmp_dir/formatted-stage.log" 2>&1
); then
  fail 'selected formatted Rust file over its budget must fail after formatting'
fi
assert_contains "$tmp_dir/formatted-stage.log" 'src/sample.rs: 3 physical lines exceeds explicit budget 2'
git -C "$partial_repo" show :src/sample.rs > "$tmp_dir/formatted-index.contents"
cp "$partial_repo/src/sample.rs" "$tmp_dir/formatted-worktree.contents"
cat > "$tmp_dir/expected-formatted.contents" <<'EOF_FORMATTED'
fn main() {
    println!("staged");
}
EOF_FORMATTED
cmp -s "$tmp_dir/expected-formatted.contents" "$tmp_dir/formatted-index.contents" \
  || fail 'formatter rewrite was not staged exactly'
cmp -s "$tmp_dir/expected-formatted.contents" "$tmp_dir/formatted-worktree.contents" \
  || fail 'formatter rewrite was not applied exactly'

cat > "$fake_bin/during-format-rustfmt" <<'EOF_DURING_FORMAT_RUSTFMT'
#!/usr/bin/env bash
set -euo pipefail
format_path=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    --edition|2024|--config|skip_children=true|--) shift ;;
    *)
      format_path="$1"
      perl -0pi -e 's/fn main\(\)\{ println!\("staged"\); \}/fn main() {\n    println!("staged");\n}/' "$1"
      shift
      ;;
  esac
done
printf '\n// concurrent editor change during rustfmt\n' >> "${RACE_DURING_TARGET:?}"
cat "$format_path"
EOF_DURING_FORMAT_RUSTFMT
chmod +x "$fake_bin/during-format-rustfmt"
cat > "$partial_repo/src/sample.rs" <<'EOF_DURING_FORMAT_STAGE'
fn main(){ println!("staged"); }
EOF_DURING_FORMAT_STAGE
git -C "$partial_repo" add src/sample.rs
cat > "$tmp_dir/expected-concurrent-index.contents" <<'EOF_EXPECTED_CONCURRENT_INDEX_BASE'
fn main(){ println!("staged"); }
EOF_EXPECTED_CONCURRENT_INDEX_BASE
if (
  cd "$partial_repo"
  RACE_DURING_TARGET="$partial_repo/src/sample.rs" CODEX_HOOK_RUSTFMT_BIN="$fake_bin/during-format-rustfmt" \
    bash scripts/format-staged-files.sh rust src/sample.rs > "$tmp_dir/during-format.log" 2>&1
); then
  fail 'formatter must reject an edit made during rustfmt'
fi
assert_contains "$tmp_dir/during-format.log" 'refusing to stage concurrently modified file: src/sample.rs'
git -C "$partial_repo" show :src/sample.rs > "$tmp_dir/during-format-index.contents"
cmp -s "$tmp_dir/expected-concurrent-index.contents" "$tmp_dir/during-format-index.contents" \
  || fail 'during-rustfmt edit was left staged'
assert_contains "$partial_repo/src/sample.rs" '// concurrent editor change during rustfmt'

race_git_bin="$tmp_dir/race-git-bin"
mkdir -p "$race_git_bin"
cat > "$race_git_bin/git" <<'EOF_RACE_GIT'
#!/usr/bin/env bash
set -euo pipefail
if [ "${1:-}" = 'apply' ]; then
  printf '\n// concurrent editor change\n' >> "${RACE_TARGET:?}"
fi
exec "${REAL_GIT:?}" "$@"
EOF_RACE_GIT
chmod +x "$race_git_bin/git"
cat > "$partial_repo/src/sample.rs" <<'EOF_RACE_STAGE'
fn main(){ println!("staged"); }
EOF_RACE_STAGE
git -C "$partial_repo" add src/sample.rs
if (
  cd "$partial_repo"
  PATH="$race_git_bin:$PATH" REAL_GIT="$real_git" RACE_TARGET="$partial_repo/src/sample.rs" \
    CODEX_HOOK_RUSTFMT_BIN="$fake_bin/partial-rustfmt" bash scripts/format-staged-files.sh rust src/sample.rs \
    > "$tmp_dir/concurrent-format.log" 2>&1
); then
  fail 'formatter must reject a concurrent edit before staging it'
fi
assert_contains "$tmp_dir/concurrent-format.log" 'refusing to stage concurrently modified file: src/sample.rs'
git -C "$partial_repo" show :src/sample.rs > "$tmp_dir/concurrent-index.contents"
cat > "$tmp_dir/expected-concurrent-index.contents" <<'EOF_EXPECTED_CONCURRENT_INDEX'
fn main(){ println!("staged"); }
EOF_EXPECTED_CONCURRENT_INDEX
cmp -s "$tmp_dir/expected-concurrent-index.contents" "$tmp_dir/concurrent-index.contents" \
  || fail 'concurrent formatter edit was left staged'
assert_contains "$partial_repo/src/sample.rs" '// concurrent editor change'

symlink_race_git_bin="$tmp_dir/symlink-race-git-bin"
mkdir -p "$symlink_race_git_bin"
cat > "$symlink_race_git_bin/git" <<'EOF_SYMLINK_RACE_GIT'
#!/usr/bin/env bash
set -euo pipefail
if [ "${1:-}" = 'apply' ]; then
  rm -f "${RACE_TARGET:?}"
  ln -s "${RACE_SYMLINK_TARGET:?}" "${RACE_TARGET:?}"
fi
exec "${REAL_GIT:?}" "$@"
EOF_SYMLINK_RACE_GIT
chmod +x "$symlink_race_git_bin/git"
symlink_race_target="$tmp_dir/outside-during-apply.rs"
cat > "$symlink_race_target" <<'EOF_SYMLINK_RACE_TARGET'
fn main(){ println!("staged"); }
EOF_SYMLINK_RACE_TARGET
cat > "$partial_repo/src/sample.rs" <<'EOF_SYMLINK_RACE_STAGE'
fn main(){ println!("staged"); }
EOF_SYMLINK_RACE_STAGE
git -C "$partial_repo" add src/sample.rs
if (
  cd "$partial_repo"
  PATH="$symlink_race_git_bin:$PATH" REAL_GIT="$real_git" \
    RACE_TARGET="$partial_repo/src/sample.rs" RACE_SYMLINK_TARGET="$symlink_race_target" \
    CODEX_HOOK_RUSTFMT_BIN="$fake_bin/partial-rustfmt" bash scripts/format-staged-files.sh rust src/sample.rs \
    > "$tmp_dir/symlink-concurrent-format.log" 2>&1
); then
  fail 'formatter must reject a symlink replacement before applying its patch'
fi
assert_contains "$tmp_dir/symlink-concurrent-format.log" 'refusing to stage concurrently modified file: src/sample.rs'
git -C "$partial_repo" show :src/sample.rs > "$tmp_dir/symlink-concurrent-index.contents"
cmp -s "$tmp_dir/expected-concurrent-index.contents" "$tmp_dir/symlink-concurrent-index.contents" \
  || fail 'symlink replacement was left staged'
[ -L "$partial_repo/src/sample.rs" ] || fail 'symlink replacement fixture did not run'
assert_contains "$symlink_race_target" 'fn main(){ println!("staged"); }'
rm -f "$partial_repo/src/sample.rs"

multi_race_git_bin="$tmp_dir/multi-race-git-bin"
mkdir -p "$multi_race_git_bin"
cat > "$multi_race_git_bin/git" <<'EOF_MULTI_RACE_GIT'
#!/usr/bin/env bash
set -euo pipefail
if [ "${1:-}" = 'apply' ]; then
  printf '\n// concurrent second-file edit\n' >> "${RACE_SECOND_TARGET:?}"
fi
exec "${REAL_GIT:?}" "$@"
EOF_MULTI_RACE_GIT
chmod +x "$multi_race_git_bin/git"
cat > "$partial_repo/src/sample.rs" <<'EOF_MULTI_RACE_SAMPLE'
fn main(){ println!("staged"); }
EOF_MULTI_RACE_SAMPLE
cat > "$partial_repo/src/second.rs" <<'EOF_MULTI_RACE_SECOND'
fn main(){ println!("second"); }
EOF_MULTI_RACE_SECOND
git -C "$partial_repo" add src/sample.rs src/second.rs
cat > "$tmp_dir/expected-multi-sample.contents" <<'EOF_EXPECTED_MULTI_SAMPLE'
fn main(){ println!("staged"); }
EOF_EXPECTED_MULTI_SAMPLE
cat > "$tmp_dir/expected-multi-second.contents" <<'EOF_EXPECTED_MULTI_SECOND'
fn main(){ println!("second"); }
EOF_EXPECTED_MULTI_SECOND
if (
  cd "$partial_repo"
  PATH="$multi_race_git_bin:$PATH" REAL_GIT="$real_git" \
    RACE_SECOND_TARGET="$partial_repo/src/second.rs" \
    CODEX_HOOK_RUSTFMT_BIN="$fake_bin/partial-rustfmt" bash scripts/format-staged-files.sh rust src/sample.rs src/second.rs \
    > "$tmp_dir/multi-concurrent-format.log" 2>&1
); then
  fail 'multi-file formatter failure must not leave a partial apply'
fi
assert_contains "$tmp_dir/multi-concurrent-format.log" 'refusing to stage concurrently modified file: src/sample.rs'
git -C "$partial_repo" show :src/sample.rs > "$tmp_dir/multi-sample-index.contents"
git -C "$partial_repo" show :src/second.rs > "$tmp_dir/multi-second-index.contents"
cmp -s "$tmp_dir/expected-multi-sample.contents" "$tmp_dir/multi-sample-index.contents" \
  || fail 'multi-file formatter failure changed the first index entry'
cmp -s "$tmp_dir/expected-multi-second.contents" "$tmp_dir/multi-second-index.contents" \
  || fail 'multi-file formatter failure changed the second index entry'
cmp -s "$tmp_dir/expected-multi-sample.contents" "$partial_repo/src/sample.rs" \
  || fail 'multi-file formatter failure changed the first worktree file'
assert_contains "$partial_repo/src/second.rs" '// concurrent second-file edit'
rm -f "$partial_repo/src/second.rs"

partial_linked_repo="$tmp_dir/partial-stage-linked"
git -C "$partial_repo" branch partial-stage-linked
git -C "$partial_repo" worktree add -q "$partial_linked_repo" partial-stage-linked
cat > "$partial_linked_repo/src/sample.rs" <<'EOF_LINKED_STAGED'
fn main(){ println!("staged"); }
EOF_LINKED_STAGED
git -C "$partial_linked_repo" diff > "$tmp_dir/linked-staged.patch"
cat > "$partial_linked_repo/src/sample.rs" <<'EOF_LINKED_WORKTREE'
fn main(){ println!("staged"); }
// linked-worktree unstaged hunk must survive
EOF_LINKED_WORKTREE
git -C "$partial_linked_repo" apply --cached "$tmp_dir/linked-staged.patch"
linked_common_dir="$(git -C "$partial_linked_repo" rev-parse --git-common-dir)"
printf 'stale shared Lefthook patch\n' > "$linked_common_dir/info/lefthook-unstaged.patch"
if (
  cd "$partial_linked_repo"
  LEFTHOOK_BIN="$lefthook_bin" CODEX_HOOK_RUSTFMT_BIN="$fake_bin/partial-rustfmt" "$partial_pre_commit_hook" \
    > "$tmp_dir/linked-partial-stage.log" 2>&1
); then
  fail 'linked worktree partial-stage formatter rewrite must fail before modifying the worktree'
fi
git -C "$partial_linked_repo" show :src/sample.rs > "$tmp_dir/linked-index.contents"
cp "$partial_linked_repo/src/sample.rs" "$tmp_dir/linked-worktree.contents"
cat > "$tmp_dir/expected-linked-worktree.contents" <<'EOF_EXPECTED_LINKED_WORKTREE'
fn main(){ println!("staged"); }
// linked-worktree unstaged hunk must survive
EOF_EXPECTED_LINKED_WORKTREE
assert_contains "$tmp_dir/linked-partial-stage.log" 'refusing to auto-format partially staged file: src/sample.rs'
cmp -s "$tmp_dir/expected-index.contents" "$tmp_dir/linked-index.contents" \
  || fail 'linked partial-stage guard modified the index'
cmp -s "$tmp_dir/expected-linked-worktree.contents" "$tmp_dir/linked-worktree.contents" \
  || fail 'linked partial-stage guard modified the worktree'
git -C "$partial_repo" worktree remove --force "$partial_linked_repo"
printf 'outside target\n' > "$tmp_dir/outside.rs"
ln -s "$tmp_dir/outside.rs" "$partial_repo/src/escape.rs"
(
  cd "$partial_repo"
  CODEX_HOOK_RUSTFMT_BIN="$fake_bin/rustfmt" bash scripts/format-staged-files.sh rust src/escape.rs
)
assert_not_contains "$formatter_log" 'src/escape.rs'
mkdir -p "$tmp_dir/outside-directory"
printf 'outside ancestor target\n' > "$tmp_dir/outside-directory/escape.rs"
ln -s "$tmp_dir/outside-directory" "$partial_repo/src/external"
(
  cd "$partial_repo"
  CODEX_HOOK_RUSTFMT_BIN="$fake_bin/rustfmt" bash scripts/format-staged-files.sh rust src/external/escape.rs
)
assert_not_contains "$formatter_log" 'src/external/escape.rs'

legacy_repo="$tmp_dir/legacy-wrapper"
mkdir -p "$legacy_repo/scripts"
cat > "$legacy_repo/lefthook.yml" <<'EOF_LEGACY_CONFIG'
pre-commit:
  parallel: true
  commands:
    format-web:
      glob: "web/**/*.{js,ts,cjs,mjs,d.cts,d.mts,jsx,tsx,json,jsonc}"
      run: bash scripts/format-staged-files.sh web {staged_files}
      stage_fixed: true
    format-markdown:
      glob: "**/*.md"
      run: bash scripts/format-staged-files.sh markdown {staged_files}
      stage_fixed: true
    format-rust:
      glob: "*.{rs}"
      run: bash scripts/format-staged-files.sh rust {staged_files}
      stage_fixed: true

commit-msg:
  commands:
    commitlint:
      run: ./node_modules/.bin/commitlint --edit {1}

post-checkout:
  commands:
    worktree-bootstrap:
      run: 'if [ -x ./scripts/run-lefthook-hook.sh ]; then ./scripts/run-lefthook-hook.sh post-checkout; fi'
EOF_LEGACY_CONFIG
cp "$repo_root/scripts/install-lefthook-hooks.sh" "$legacy_repo/scripts/install-lefthook-hooks.sh"
cp "$repo_root/scripts/format-staged-files.sh" "$legacy_repo/scripts/format-staged-files.sh"
cp "$repo_root/scripts/check-staged-formatter-safety.sh" "$legacy_repo/scripts/check-staged-formatter-safety.sh"
cp "$repo_root/scripts/run-pre-commit-hook.sh" "$legacy_repo/scripts/run-pre-commit-hook.sh"
chmod +x "$legacy_repo/scripts/install-lefthook-hooks.sh"
chmod +x "$legacy_repo/scripts/format-staged-files.sh"
chmod +x "$legacy_repo/scripts/check-staged-formatter-safety.sh"
chmod +x "$legacy_repo/scripts/run-pre-commit-hook.sh"
git -C "$legacy_repo" init -q
git -C "$legacy_repo" config user.name 'Codex Test'
git -C "$legacy_repo" config user.email 'codex-test@example.com'
(
  cd "$legacy_repo"
  cat >> lefthook.yml <<'EOF_LEGACY_HOOK'

prepare-commit-msg:
  commands:
    legacy-wrapper:
      run: "true"
EOF_LEGACY_HOOK
  PATH="$(dirname "$lefthook_bin"):$PATH" "$lefthook_bin" install prepare-commit-msg >/dev/null
  PATH="$(dirname "$lefthook_bin"):$PATH" "$lefthook_bin" install pre-commit >/dev/null
  printf '\n# managed by codex-vibe-monitor hooks:install\n' >> .git/hooks/pre-commit
  cp "$repo_root/lefthook.yml" lefthook.yml
  PATH="$(dirname "$lefthook_bin"):$PATH" bash scripts/install-lefthook-hooks.sh >/dev/null
)
legacy_hook="$legacy_repo/.git/hooks/prepare-commit-msg"
if [ -e "$legacy_hook" ] || [ -L "$legacy_hook" ]; then
  fail 'exact obsolete Lefthook prepare-commit-msg wrapper was not removed'
fi
assert_contains "$legacy_repo/.git/hooks/pre-commit" 'bash scripts/run-pre-commit-hook.sh'
perl -0pi -e 's{bash scripts/run-pre-commit-hook.sh "\$@"}{if ! bash scripts/check-staged-formatter-safety.sh\nthen\n  exit 1\nfi\n\ncall_lefthook run "pre-commit" "\$@"}' "$legacy_repo/.git/hooks/pre-commit"
assert_contains "$legacy_repo/.git/hooks/pre-commit" 'bash scripts/check-staged-formatter-safety.sh'
(
  cd "$legacy_repo"
  PATH="$(dirname "$lefthook_bin"):$PATH" bash scripts/install-lefthook-hooks.sh >/dev/null
)
assert_contains "$legacy_repo/.git/hooks/pre-commit" 'bash scripts/run-pre-commit-hook.sh'
assert_not_contains "$legacy_repo/.git/hooks/pre-commit" 'bash scripts/check-staged-formatter-safety.sh'
idempotent_output="$tmp_dir/idempotent-install.log"
(
  cd "$legacy_repo"
  PATH="$(dirname "$lefthook_bin"):$PATH" bash scripts/install-lefthook-hooks.sh > "$idempotent_output" 2>&1
)
assert_not_contains "$idempotent_output" 'already exists and is unmanaged'
printf '#!/bin/sh\necho custom\n' > "$legacy_hook"
chmod +x "$legacy_hook"
(
  cd "$legacy_repo"
  PATH="$(dirname "$lefthook_bin"):$PATH" bash scripts/install-lefthook-hooks.sh >/dev/null
)
assert_contains "$legacy_hook" 'echo custom'
rm -f "$legacy_hook"
ln -s ../custom-prepare-hook "$legacy_hook"
(
  cd "$legacy_repo"
  PATH="$(dirname "$lefthook_bin"):$PATH" bash scripts/install-lefthook-hooks.sh >/dev/null
)
[ -L "$legacy_hook" ] || fail 'prepare-commit-msg symlink was modified'
pre_commit_hook="$legacy_repo/.git/hooks/pre-commit"
printf '\necho custom-pre-commit\n' >> "$pre_commit_hook"
(
  cd "$legacy_repo"
  PATH="$(dirname "$lefthook_bin"):$PATH" bash scripts/install-lefthook-hooks.sh >/dev/null
)
assert_contains "$pre_commit_hook" 'echo custom-pre-commit'

fifo_hooks_repo="$tmp_dir/fifo-hooks"
mkdir -p "$fifo_hooks_repo/scripts"
cp "$repo_root/lefthook.yml" "$fifo_hooks_repo/lefthook.yml"
cp "$repo_root/scripts/install-lefthook-hooks.sh" "$fifo_hooks_repo/scripts/install-lefthook-hooks.sh"
chmod +x "$fifo_hooks_repo/scripts/install-lefthook-hooks.sh"
git -C "$fifo_hooks_repo" init -q
mkfifo "$fifo_hooks_repo/.git/hooks/pre-commit"
mkfifo "$fifo_hooks_repo/.git/hooks/prepare-commit-msg"
(
  cd "$fifo_hooks_repo"
  PATH="$(dirname "$lefthook_bin"):$PATH" bash scripts/install-lefthook-hooks.sh > "$tmp_dir/fifo-hooks.log" 2>&1
) &
fifo_hooks_pid=$!
fifo_hooks_done=0
for attempt in {1..100}; do
  if ! kill -0 "$fifo_hooks_pid" 2>/dev/null; then
    fifo_hooks_done=1
    break
  fi
  sleep 0.05
done
if [ "$fifo_hooks_done" -ne 1 ]; then
  kill "$fifo_hooks_pid" 2>/dev/null || true
  wait "$fifo_hooks_pid" 2>/dev/null || true
  fail 'hooks:install must not block on FIFO hooks'
fi
wait "$fifo_hooks_pid" || fail 'hooks:install failed while preserving FIFO hooks'
[ -p "$fifo_hooks_repo/.git/hooks/pre-commit" ] || fail 'FIFO pre-commit hook was modified'
[ -p "$fifo_hooks_repo/.git/hooks/prepare-commit-msg" ] || fail 'FIFO prepare-commit-msg hook was modified'

installer_lock_linked_repo="$tmp_dir/installer-lock-linked"
installer_lock_fake_bin="$tmp_dir/installer-lock-fake-bin"
mkdir -p "$installer_lock_fake_bin"
git -C "$partial_repo" branch installer-lock-base
git -C "$partial_repo" worktree add -q "$installer_lock_linked_repo" installer-lock-base
cat > "$installer_lock_fake_bin/lefthook" <<'EOF_INSTALLER_LOCK_LEFTHOOK'
#!/usr/bin/env bash
set -euo pipefail
if [ "${1:-}" = 'version' ]; then
  exec "${REAL_LEFTHOOK:?}" "$@"
fi
if [ "${1:-}" = 'install' ]; then
  if [ -e "${INSTALL_LOCK_ACTIVE:?}" ]; then
    : > "${INSTALL_LOCK_OVERLAP:?}"
    exit 91
  fi
  : > "$INSTALL_LOCK_ACTIVE"
  sleep 0.2
  set +e
  "${REAL_LEFTHOOK:?}" "$@"
  install_status=$?
  set -e
  unlink "$INSTALL_LOCK_ACTIVE"
  exit "$install_status"
fi
exec "${REAL_LEFTHOOK:?}" "$@"
EOF_INSTALLER_LOCK_LEFTHOOK
chmod +x "$installer_lock_fake_bin/lefthook"
installer_lock_active="$tmp_dir/installer-lock.active"
installer_lock_overlap="$tmp_dir/installer-lock.overlap"
(
  cd "$partial_repo"
  PATH="$installer_lock_fake_bin:$PATH" REAL_LEFTHOOK="$lefthook_bin" \
    INSTALL_LOCK_ACTIVE="$installer_lock_active" INSTALL_LOCK_OVERLAP="$installer_lock_overlap" \
    bash scripts/install-lefthook-hooks.sh >/dev/null 2>&1
) &
first_installer_lock_pid=$!
(
  cd "$installer_lock_linked_repo"
  PATH="$installer_lock_fake_bin:$PATH" REAL_LEFTHOOK="$lefthook_bin" \
    INSTALL_LOCK_ACTIVE="$installer_lock_active" INSTALL_LOCK_OVERLAP="$installer_lock_overlap" \
    bash scripts/install-lefthook-hooks.sh >/dev/null 2>&1
) &
second_installer_lock_pid=$!
wait "$first_installer_lock_pid" || fail 'first linked hooks:install invocation failed'
wait "$second_installer_lock_pid" || fail 'second linked hooks:install invocation failed'
[ ! -e "$installer_lock_overlap" ] || fail 'linked hooks:install invocations were not serialized'
git -C "$partial_repo" worktree remove --force "$installer_lock_linked_repo"

core_hooks_path_repo="$tmp_dir/core-hooks-path"
mkdir -p "$core_hooks_path_repo/scripts"
cp "$repo_root/scripts/install-lefthook-hooks.sh" "$core_hooks_path_repo/scripts/install-lefthook-hooks.sh"
chmod +x "$core_hooks_path_repo/scripts/install-lefthook-hooks.sh"
git -C "$core_hooks_path_repo" init -q
git -C "$core_hooks_path_repo" config core.hooksPath "$tmp_dir/custom-hooks"
core_hooks_path_git_dir="$(git -C "$core_hooks_path_repo" rev-parse --absolute-git-dir)"
(
  cd "$core_hooks_path_repo"
  bash scripts/install-lefthook-hooks.sh > "$tmp_dir/core-hooks-path.log" 2>&1
)
assert_contains "$tmp_dir/core-hooks-path.log" 'core.hooksPath is set'
[ ! -e "$core_hooks_path_git_dir/hooks-install.flock" ] \
  || fail 'core.hooksPath no-op created the shared installer lock'

pre_commit_template_repo="$tmp_dir/pre-commit-template"
mkdir -p "$pre_commit_template_repo"
cp "$repo_root/lefthook.yml" "$pre_commit_template_repo/lefthook.yml"
git -C "$pre_commit_template_repo" init -q
(
  cd "$pre_commit_template_repo"
  PATH="$(dirname "$lefthook_bin"):$PATH" "$lefthook_bin" install pre-commit >/dev/null
)
pre_commit_symlink_target="$tmp_dir/standard-pre-commit"
cp "$pre_commit_template_repo/.git/hooks/pre-commit" "$pre_commit_symlink_target"
printf '\n# managed by codex-vibe-monitor hooks:install\n' >> "$pre_commit_symlink_target"
cp "$pre_commit_symlink_target" "$tmp_dir/standard-pre-commit.before"
rm -f "$pre_commit_hook"
ln -s "$pre_commit_symlink_target" "$pre_commit_hook"
(
  cd "$legacy_repo"
  PATH="$(dirname "$lefthook_bin"):$PATH" bash scripts/install-lefthook-hooks.sh >/dev/null
)
[ -L "$pre_commit_hook" ] || fail 'pre-commit symlink was modified'
cmp -s "$tmp_dir/standard-pre-commit.before" "$pre_commit_symlink_target" \
  || fail 'pre-commit symlink target was modified'

historical_repo="$tmp_dir/historical-checkout"
mkdir -p "$historical_repo"
cp "$repo_root/lefthook.yml" "$historical_repo/lefthook.yml"
git -C "$historical_repo" init -q
(
  cd "$historical_repo"
  "$lefthook_bin" run post-checkout --no-auto-install
)

old_lefthook_repo="$tmp_dir/old-lefthook"
mkdir -p "$old_lefthook_repo/scripts"
cp "$repo_root/scripts/install-lefthook-hooks.sh" "$old_lefthook_repo/scripts/install-lefthook-hooks.sh"
chmod +x "$old_lefthook_repo/scripts/install-lefthook-hooks.sh"
git -C "$old_lefthook_repo" init -q
cat > "$fake_bin/lefthook" <<'EOF_OLD_LEFTHOOK'
#!/usr/bin/env bash
printf '2.1.10\n'
EOF_OLD_LEFTHOOK
chmod +x "$fake_bin/lefthook"
old_lefthook_output="$tmp_dir/old-lefthook-output.log"
if (
  cd "$old_lefthook_repo"
  PATH="$fake_bin:$PATH" bash scripts/install-lefthook-hooks.sh > "$old_lefthook_output" 2>&1
); then
  fail 'hooks:install must reject Lefthook versions older than 2.2.1'
fi
assert_contains "$old_lefthook_output" 'Lefthook 2.2.1 or newer is required'

local_only_repo="$tmp_dir/local-only-lefthook"
mkdir -p "$local_only_repo/scripts" "$local_only_repo/node_modules/.bin"
cp "$repo_root/scripts/install-lefthook-hooks.sh" "$local_only_repo/scripts/install-lefthook-hooks.sh"
chmod +x "$local_only_repo/scripts/install-lefthook-hooks.sh"
git -C "$local_only_repo" init -q
cat > "$local_only_repo/node_modules/.bin/lefthook" <<'EOF_LOCAL_ONLY_LEFTHOOK'
#!/usr/bin/env bash
printf '2.1.10\n'
EOF_LOCAL_ONLY_LEFTHOOK
chmod +x "$local_only_repo/node_modules/.bin/lefthook"
local_only_output="$tmp_dir/local-only-lefthook-output.log"
if (
  cd "$local_only_repo"
  PATH="$local_only_repo/node_modules/.bin:/usr/bin:/bin" bash scripts/install-lefthook-hooks.sh > "$local_only_output" 2>&1
); then
  fail 'hooks:install must reject a repo-local Lefthook binary'
fi
assert_contains "$local_only_output" 'repo-local binary is not sufficient'

printf 'git hook contract smoke passed\n'
