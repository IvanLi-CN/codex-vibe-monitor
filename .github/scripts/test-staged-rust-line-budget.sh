#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
checker="$repo_root/.github/scripts/check_staged_rust_line_budget.py"
fixture_root="$repo_root/.github/scripts/fixtures/staged-rust-line-budget"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

fail() {
  printf '%s\n' "$*" >&2
  exit 1
}

assert_contains() {
  grep -Fq -- "$2" "$1" || fail "expected $1 to contain $2"
}

fixture_repo="$tmp_dir/repo"
mkdir -p "$fixture_repo"
cp -R "$fixture_root/." "$fixture_repo/"
git -C "$fixture_repo" init -q
git -C "$fixture_repo" config user.name 'Codex Test'
git -C "$fixture_repo" config user.email 'codex-test@example.com'
git -C "$fixture_repo" add .
git -C "$fixture_repo" commit -qm 'fixture'

run_checker() {
  (cd "$fixture_repo" && python3 "$checker" --policy policy.json)
}

cat > "$fixture_repo/src/selected.rs" <<'EOF_OVER_BUDGET'
fn selected_one() {}
fn selected_two() {}
fn selected_three() {}
fn selected_four() {}
EOF_OVER_BUDGET
git -C "$fixture_repo" add src/selected.rs
if run_checker >"$tmp_dir/selected-over.stdout" 2>"$tmp_dir/selected-over.stderr"; then
  fail 'selected over-budget staged content must fail'
fi
assert_contains "$tmp_dir/selected-over.stderr" 'src/selected.rs: 4 physical lines exceeds explicit budget 3'

cat > "$fixture_repo/src/selected.rs" <<'EOF_AT_BUDGET'
fn selected_one() {}
fn selected_two() {}
fn selected_three() {}
EOF_AT_BUDGET
git -C "$fixture_repo" add src/selected.rs
run_checker

for ((line = 1; line <= 3001; line++)); do
  printf 'fn generated_unselected_%s() {}\n' "$line"
done > "$fixture_repo/src/unselected.rs"
git -C "$fixture_repo" add src/unselected.rs
run_checker

cat > "$fixture_repo/src/selected with spaces.rs" <<'EOF_SPACED'
fn spaced_one() {}
fn spaced_two() {}
EOF_SPACED
git -C "$fixture_repo" add 'src/selected with spaces.rs'
run_checker

cat > "$fixture_repo/src/selected.rs" <<'EOF_INDEX_CONTENT'
fn staged_one() {}
fn staged_two() {}
fn staged_three() {}
fn staged_four() {}
EOF_INDEX_CONTENT
git -C "$fixture_repo" add src/selected.rs
cat > "$fixture_repo/src/selected.rs" <<'EOF_WORKTREE_CONTENT'
fn worktree_only() {}
EOF_WORKTREE_CONTENT
git -C "$fixture_repo" diff --cached --binary >"$tmp_dir/index.before"
cp "$fixture_repo/src/selected.rs" "$tmp_dir/worktree.before"
if run_checker >"$tmp_dir/index-content.stdout" 2>"$tmp_dir/index-content.stderr"; then
  fail 'over-budget index content must fail even when worktree content is short'
fi
assert_contains "$tmp_dir/index-content.stderr" 'src/selected.rs: 4 physical lines exceeds explicit budget 3'
git -C "$fixture_repo" diff --cached --binary >"$tmp_dir/index.after"
cmp -s "$tmp_dir/index.before" "$tmp_dir/index.after" || fail 'checker modified staged content'
cmp -s "$tmp_dir/worktree.before" "$fixture_repo/src/selected.rs" || fail 'checker modified worktree content'

real_git="$(command -v git)"
race_bin="$tmp_dir/race-bin"
mkdir -p "$race_bin"
cat > "$race_bin/git" <<'EOF_RACE_GIT'
#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == '-C' && "${3:-}" == 'cat-file' && ! -e "${RACE_MARKER:?}" ]]; then
  : > "$RACE_MARKER"
  cat > "$RACE_REPO/src/selected.rs" <<'EOF_RACE_CONTENT'
fn race_one() {}
fn race_two() {}
fn race_three() {}
EOF_RACE_CONTENT
  "$RACE_REAL_GIT" -C "$RACE_REPO" add src/selected.rs
fi

exec "$RACE_REAL_GIT" "$@"
EOF_RACE_GIT
chmod +x "$race_bin/git"
race_marker="$tmp_dir/race.marker"
if (
  cd "$fixture_repo"
  PATH="$race_bin:$PATH" RACE_REAL_GIT="$real_git" RACE_MARKER="$race_marker" \
    RACE_REPO="$fixture_repo" python3 "$checker" --policy policy.json
) >"$tmp_dir/index-race.stdout" 2>"$tmp_dir/index-race.stderr"; then
  fail 'index mutation during the check must fail closed'
fi
assert_contains "$tmp_dir/index-race.stderr" 'Git index changed during staged Rust line-budget check; retry the check'

cat > "$fixture_repo/src/selected.rs" <<'EOF_RESTORED_INDEX_CONTENT'
fn staged_one() {}
fn staged_two() {}
fn staged_three() {}
fn staged_four() {}
EOF_RESTORED_INDEX_CONTENT
git -C "$fixture_repo" add src/selected.rs
cat > "$fixture_repo/src/selected.rs" <<'EOF_RESTORED_WORKTREE_CONTENT'
fn worktree_only() {}
EOF_RESTORED_WORKTREE_CONTENT

cp "$fixture_repo/policy.json" "$tmp_dir/policy.before"
sed -i.bak 's/"line_budget": 3/"line_budget": 99/' "$fixture_repo/policy.json"
rm -f "$fixture_repo/policy.json.bak"
if run_checker >"$tmp_dir/policy-skew.stdout" 2>"$tmp_dir/policy-skew.stderr"; then
  fail 'staged policy must control the budget when the worktree policy is changed'
fi
assert_contains "$tmp_dir/policy-skew.stderr" 'src/selected.rs: 4 physical lines exceeds explicit budget 3'
cp "$tmp_dir/policy.before" "$fixture_repo/policy.json"

printf '{"files":[{"path":"src/selected.rs","line_budget":99}]}\n' > "$tmp_dir/permissive-policy.json"
mv "$fixture_repo/policy.json" "$tmp_dir/policy.index-path"
ln -s "$tmp_dir/permissive-policy.json" "$fixture_repo/policy.json"
if run_checker >"$tmp_dir/policy-symlink.stdout" 2>"$tmp_dir/policy-symlink.stderr"; then
  fail 'unstaged policy symlink must not redirect the staged policy lookup'
fi
assert_contains "$tmp_dir/policy-symlink.stderr" 'src/selected.rs: 4 physical lines exceeds explicit budget 3'
unlink "$fixture_repo/policy.json"
mv "$tmp_dir/policy.index-path" "$fixture_repo/policy.json"

sed -i.bak 's#src/selected.rs#src/./selected.rs#' "$fixture_repo/policy.json"
rm -f "$fixture_repo/policy.json.bak"
git -C "$fixture_repo" add policy.json
if run_checker >"$tmp_dir/policy-alias.stdout" 2>"$tmp_dir/policy-alias.stderr"; then
  fail 'non-canonical policy paths must fail closed'
fi
assert_contains "$tmp_dir/policy-alias.stderr" 'policy.files[0].path must be a canonical repository-relative path'
cp "$tmp_dir/policy.before" "$fixture_repo/policy.json"
git -C "$fixture_repo" add policy.json

rm -f "$fixture_repo/src/selected with spaces.rs"
ln -s selected.rs "$fixture_repo/src/selected with spaces.rs"
git -C "$fixture_repo" add -A -- 'src/selected with spaces.rs'
if run_checker >"$tmp_dir/type-change.stdout" 2>"$tmp_dir/type-change.stderr"; then
  fail 'staged Rust type changes must fail closed'
fi
assert_contains "$tmp_dir/type-change.stderr" 'src/selected with spaces.rs: staged index entry must be a regular file'
unlink "$fixture_repo/src/selected with spaces.rs"
cat > "$fixture_repo/src/selected with spaces.rs" <<'EOF_RESTORED_SPACED'
fn spaced_one() {}
fn spaced_two() {}
EOF_RESTORED_SPACED
git -C "$fixture_repo" add 'src/selected with spaces.rs'

cat > "$fixture_repo/src/selected.rs" <<'EOF_DELETED_SIBLING'
fn selected_one() {}
fn selected_two() {}
fn selected_three() {}
EOF_DELETED_SIBLING
git -C "$fixture_repo" add src/selected.rs
git -C "$fixture_repo" rm -q -f 'src/selected with spaces.rs'
run_checker

printf 'staged Rust line-budget contract: passed\n'
