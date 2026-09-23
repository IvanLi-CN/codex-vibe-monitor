#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
checker="$repo_root/.github/scripts/check_rust_source_quality.py"
fixtures_root="$repo_root/.github/scripts/fixtures/rust-source-quality"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

copy_fixture() {
  local name="$1"
  local destination="$tmp_dir/$name"
  mkdir -p "$destination"
  cp -R "$fixtures_root/$name/." "$destination/"
  printf '%s\n' "$destination"
}

expect_pass() {
  local name="$1"
  local fixture
  fixture="$(copy_fixture "$name")"
  python3 "$checker" --repo-root "$fixture" --policy "$fixture/policy.json"
}

expect_failure() {
  local name="$1"
  local pattern="$2"
  local fixture
  fixture="$(copy_fixture "$name")"
  if python3 "$checker" --repo-root "$fixture" --policy "$fixture/policy.json" \
    >"$tmp_dir/$name.stdout" 2>"$tmp_dir/$name.stderr"; then
    echo "expected fixture $name to fail" >&2
    exit 1
  fi
  grep -Fq "$pattern" "$tmp_dir/$name.stderr"
}

expect_pass accepted

growth_fixture="$(copy_fixture selected-growth)"
printf '\n// selected-path growth\n' >>"$growth_fixture/src/selected.rs"
if python3 "$checker" --repo-root "$growth_fixture" --policy "$growth_fixture/policy.json" \
  >"$tmp_dir/selected-growth.stdout" 2>"$tmp_dir/selected-growth.stderr"; then
  echo "expected selected-path growth fixture to fail" >&2
  exit 1
fi
grep -Fq "selected path has" "$tmp_dir/selected-growth.stderr"

expect_failure lower-budget "selected path has"

unselected_fixture="$(copy_fixture unselected-long)"
for ((line = 1; line <= 3001; line++)); do
  printf 'fn generated_unselected_%s() {}\n' "$line" >>"$unselected_fixture/src/unselected.rs"
done
python3 "$checker" --repo-root "$unselected_fixture" --policy "$unselected_fixture/policy.json"

expect_failure cohesive-exception-no-reason "cohesive_exception.reason must be a narrow reason"
expect_failure unrecorded-suppression "unrecorded or materially altered allow suppression"
expect_failure altered-suppression "unrecorded or materially altered allow suppression"
expect_failure include "include! control-flow composition is not allowed"

echo "rust-source-quality fixtures: passed"
