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
  assert_contains "$repo_root/lefthook.yml" "format-$surface:"
done
assert_contains "$repo_root/lefthook.yml" '{staged_files}'
assert_contains "$repo_root/lefthook.yml" 'glob: "**/*.md"'
assert_not_contains "$repo_root/lefthook.yml" 'cargo clippy'
assert_not_contains "$repo_root/lefthook.yml" 'tsc -b'
assert_not_contains "$repo_root/lefthook.yml" 'bun run lint:web'
assert_not_contains "$repo_root/scripts/install-lefthook-hooks.sh" 'lefthook uninstall'

fake_bin="$tmp_dir/fake-bin"
formatter_log="$tmp_dir/formatter.log"
mkdir -p "$fake_bin"
for formatter in biome rustfmt dprint; do
  cat > "$fake_bin/$formatter" <<'EOF_FORMATTER'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\t%s\n' "$(basename "$0")" "$*" >> "${FORMATTER_LOG:?}"
EOF_FORMATTER
  chmod +x "$fake_bin/$formatter"
done
export FORMATTER_LOG="$formatter_log"

(
  cd "$repo_root"
  CODEX_HOOK_BIOME_BIN="$fake_bin/biome" \
    bash scripts/format-staged-files.sh web web/src/test-setup.ts missing.ts web/../../package.json web/../package.json
  CODEX_HOOK_RUSTFMT_BIN="$fake_bin/rustfmt" \
    bash scripts/format-staged-files.sh rust src/main.rs deleted.rs
  CODEX_HOOK_DPRINT_BIN="$fake_bin/dprint" \
    bash scripts/format-staged-files.sh markdown README.md removed.md docs/../README.md
)
assert_contains "$formatter_log" $'biome\tcheck --write web/src/test-setup.ts'
assert_contains "$formatter_log" $'rustfmt\t--edition 2024 src/main.rs'
assert_contains "$formatter_log" $'dprint\tfmt README.md'
assert_not_contains "$formatter_log" 'missing.ts'
assert_not_contains "$formatter_log" '../../package.json'
assert_not_contains "$formatter_log" 'web/../package.json'
assert_not_contains "$formatter_log" 'deleted.rs'
assert_not_contains "$formatter_log" 'removed.md'
assert_not_contains "$formatter_log" 'docs/../README.md'

lefthook_source="$(command -v lefthook 2>/dev/null || true)"
if [ -z "$lefthook_source" ] && [ -x "$repo_root/node_modules/.bin/lefthook" ]; then
  lefthook_source="$repo_root/node_modules/.bin/lefthook"
fi
[ -n "$lefthook_source" ] || fail 'git hook contract smoke requires a Lefthook binary'
external_lefthook_dir="$tmp_dir/external-lefthook-bin"
mkdir -p "$external_lefthook_dir"
cp -L "$(realpath "$lefthook_source")" "$external_lefthook_dir/lefthook"
chmod +x "$external_lefthook_dir/lefthook"
lefthook_bin="$external_lefthook_dir/lefthook"

partial_repo="$tmp_dir/partial-stage"
mkdir -p "$partial_repo/scripts" "$partial_repo/src"
cp "$repo_root/lefthook.yml" "$partial_repo/lefthook.yml"
cp "$repo_root/scripts/format-staged-files.sh" "$partial_repo/scripts/format-staged-files.sh"
chmod +x "$partial_repo/scripts/format-staged-files.sh"
cat > "$partial_repo/src/sample.rs" <<'EOF_RUST'
fn main() {
    println!("base");
}
EOF_RUST
git -C "$partial_repo" init -q
git -C "$partial_repo" config user.name 'Codex Test'
git -C "$partial_repo" config user.email 'codex-test@example.com'
git -C "$partial_repo" add .
git -C "$partial_repo" commit -qm 'fixture'
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
for path in "$@"; do
  [ "$path" = '--edition' ] && shift && continue
  [ "$path" = '2024' ] && continue
  [ -f "$path" ] || continue
  printf '// formatter result\n' >> "$path"
done
EOF_PARTIAL_RUSTFMT
chmod +x "$fake_bin/partial-rustfmt"
(
  cd "$partial_repo"
  CODEX_HOOK_RUSTFMT_BIN="$fake_bin/partial-rustfmt" "$lefthook_bin" run pre-commit --no-auto-install
)
git -C "$partial_repo" diff --cached -- src/sample.rs > "$tmp_dir/index.diff"
git -C "$partial_repo" diff -- src/sample.rs > "$tmp_dir/worktree.diff"
assert_contains "$tmp_dir/index.diff" '// formatter result'
assert_not_contains "$tmp_dir/index.diff" 'unstaged hunk must survive'
assert_contains "$tmp_dir/worktree.diff" 'unstaged hunk must survive'
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
cp "$repo_root/lefthook.yml" "$legacy_repo/lefthook.yml"
cp "$repo_root/scripts/install-lefthook-hooks.sh" "$legacy_repo/scripts/install-lefthook-hooks.sh"
chmod +x "$legacy_repo/scripts/install-lefthook-hooks.sh"
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
  cp "$repo_root/lefthook.yml" lefthook.yml
  PATH="$(dirname "$lefthook_bin"):$PATH" bash scripts/install-lefthook-hooks.sh >/dev/null
)
legacy_hook="$legacy_repo/.git/hooks/prepare-commit-msg"
if [ -e "$legacy_hook" ] || [ -L "$legacy_hook" ]; then
  fail 'exact obsolete Lefthook prepare-commit-msg wrapper was not removed'
fi
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
printf '1.13.6\n'
EOF_OLD_LEFTHOOK
chmod +x "$fake_bin/lefthook"
old_lefthook_output="$tmp_dir/old-lefthook-output.log"
if (
  cd "$old_lefthook_repo"
  PATH="$fake_bin:$PATH" bash scripts/install-lefthook-hooks.sh > "$old_lefthook_output" 2>&1
); then
  fail 'hooks:install must reject Lefthook versions older than 2.1.7'
fi
assert_contains "$old_lefthook_output" 'Lefthook 2.1.7 or newer is required'

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
