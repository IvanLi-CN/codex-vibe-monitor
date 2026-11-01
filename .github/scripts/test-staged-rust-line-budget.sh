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

race_git_bin="$tmp_dir/race-git-bin"
mkdir -p "$race_git_bin"
race_git_count="$tmp_dir/race-git-count"
printf '0\n' > "$race_git_count"
cat > "$race_git_bin/git" <<'EOF_RACE_GIT'
#!/usr/bin/env bash
set -euo pipefail
if [ "${3:-}" = 'cat-file' ]; then
  race_count="$(cat "${RACE_GIT_COUNT:?}")"
  if [ "$race_count" -eq 1 ]; then
    printf '2\n' > "$RACE_GIT_COUNT"
    cat > "$RACE_FIXTURE_REPO/src/selected.rs" <<'EOF_RACE_CONTENT'
fn raced_one() {}
fn raced_two() {}
fn raced_three() {}
fn raced_four() {}
EOF_RACE_CONTENT
    "$REAL_GIT" -C "$RACE_FIXTURE_REPO" add src/selected.rs
  elif [ "$race_count" -eq 0 ]; then
    printf '1\n' > "$RACE_GIT_COUNT"
  fi
fi
exec "$REAL_GIT" "$@"
EOF_RACE_GIT
chmod +x "$race_git_bin/git"
cat > "$fixture_repo/src/selected.rs" <<'EOF_RACE_BASELINE'
fn selected_one() {}
fn selected_two() {}
fn selected_three() {}
EOF_RACE_BASELINE
git -C "$fixture_repo" add src/selected.rs
if (
  cd "$fixture_repo"
  PATH="$race_git_bin:$PATH" REAL_GIT="$(command -v git)" RACE_GIT_COUNT="$race_git_count" \
    RACE_FIXTURE_REPO="$fixture_repo" python3 "$checker" --policy policy.json
) >"$tmp_dir/index-race.stdout" 2>"$tmp_dir/index-race.stderr"; then
  fail 'checker must reject an index change during the blob snapshot'
fi
assert_contains "$tmp_dir/index-race.stderr" 'Git index changed during staged Rust line-budget check; retry the check'
cat > "$fixture_repo/src/selected.rs" <<'EOF_RACE_RESTORED'
fn selected_one() {}
fn selected_two() {}
fn selected_three() {}
EOF_RACE_RESTORED
git -C "$fixture_repo" add src/selected.rs

cat > "$fixture_repo/src/selected.rs" <<'EOF_DELETED_SIBLING'
fn selected_one() {}
fn selected_two() {}
fn selected_three() {}
EOF_DELETED_SIBLING
git -C "$fixture_repo" add src/selected.rs
git -C "$fixture_repo" rm -q -f 'src/selected with spaces.rs'
run_checker

rm -f "$fixture_repo/src/selected.rs"
ln -s selected-target.rs "$fixture_repo/src/selected.rs"
git -C "$fixture_repo" add -- src/selected.rs
if run_checker >"$tmp_dir/symlink-source.stdout" 2>"$tmp_dir/symlink-source.stderr"; then
  fail 'selected staged Rust symlink must fail closed'
fi
assert_contains "$tmp_dir/symlink-source.stderr" 'src/selected.rs: staged index entry must be a regular file'
rm -f "$fixture_repo/src/selected.rs"
cat > "$fixture_repo/src/selected.rs" <<'EOF_RESTORED_SELECTED'
fn selected_one() {}
fn selected_two() {}
fn selected_three() {}
EOF_RESTORED_SELECTED
git -C "$fixture_repo" add src/selected.rs

printf 'staged Rust line-budget contract: passed\n'
