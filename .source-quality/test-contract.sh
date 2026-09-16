#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline_command="$repo_root/tools/source-structure-check/baseline"
check_all_command="$repo_root/tools/source-structure-check/check-all"
check_staged_command="$repo_root/tools/source-structure-check/check-staged"
require_zero_command="$repo_root/tools/source-structure-check/require-zero"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-"$repo_root/target"}
biome_binary=$(command -v biome 2>/dev/null || true)
if [ -z "$biome_binary" ]; then
  printf '%s\n' 'contract failure: Biome executable is required' >&2
  exit 1
fi
export SOURCE_QUALITY_BIOME_BIN="$biome_binary"

test_root=$(mktemp -d "${TMPDIR:-/tmp}/source-quality-contract.XXXXXX")
cleanup() {
  rm -rf "$test_root"
}
trap cleanup EXIT HUP INT TERM

fixture="$test_root/fixture"
mkdir -p "$fixture/.source-quality" "$fixture/src"
git -C "$fixture" init -q
cp "$repo_root/.source-quality/config.json" "$fixture/.source-quality/config.json"
cp "$repo_root/biome.json" "$fixture/biome.json"

emit_rust_function() {
  body_lines=$1
  name=$2
  printf 'fn %s() {\n' "$name"
  line=1
  while [ "$line" -le "$body_lines" ]; do
    printf '    let _line_%s = %s;\n' "$line" "$line"
    line=$((line + 1))
  done
  printf '%s\n' '}'
}

write_rust_function() {
  emit_rust_function "$1" "$2" >"$3"
}

append_rust_function() {
  emit_rust_function "$1" "$2" >>"$3"
}

run_expect() {
  expected=$1
  description=$2
  shift 2
  command_output=
  if command_output=$("$@" 2>"$test_root/last-stderr"); then
    command_status=0
  else
    command_status=$?
  fi
  if [ "$command_status" -ne "$expected" ]; then
    printf 'contract failure: %s (expected %s, got %s)\n' \
      "$description" "$expected" "$command_status" >&2
    if [ -s "$test_root/last-stderr" ]; then
      sed -n '1,120p' "$test_root/last-stderr" >&2
    fi
    if [ -n "$command_output" ]; then
      printf '%s\n' "$command_output" >&2
    fi
    exit 1
  fi
}

assert_report() {
  description=$1
  report=$2
  filter=$3
  if ! printf '%s\n' "$report" | jq -e "$filter" >/dev/null; then
    printf 'contract failure: %s\n%s\n' "$description" "$report" >&2
    exit 1
  fi
}

write_rust_function 99 baseline_function "$fixture/src/fixture.rs"
git -C "$fixture" add .source-quality/config.json src/fixture.rs

run_expect 0 'first baseline' \
  "$baseline_command" --repo-root "$fixture"
assert_report 'first baseline report' "$command_output" \
  '.status == "pass" and .mode == "all" and .baseline_written == true and
   .scope.tracked_source_count == 1 and .scope.checked_source_count == 1 and
   (.scope.excluded_tracked_paths | length) == 0 and
   .ratchet.new_diagnostics == 0 and
   any(.diagnostics[]; .rule == "function-lines" and
     .path == "src/fixture.rs" and .subject == "baseline_function" and
     .actual == 101 and .limit == 100)'
cp "$fixture/.source-quality/baseline.json" "$test_root/baseline-before-parse.json"

append_rust_function 99 new_function "$fixture/src/fixture.rs"
run_expect 1 'new diagnostic identity' \
  "$check_all_command" --repo-root "$fixture"
assert_report 'new diagnostic ratchet report' "$command_output" \
  '.status == "fail" and .ratchet.new_diagnostics == 1 and
   .ratchet.metric_growth == 0 and
   any(.diagnostics[]; .rule == "ratchet/new-diagnostic" and
     .path == "src/fixture.rs" and .actual == 101 and .limit == 0)'

write_rust_function 100 baseline_function "$fixture/src/fixture.rs"
run_expect 1 'metric growth' "$check_all_command" --repo-root "$fixture"
assert_report 'metric growth ratchet report' "$command_output" \
  '.status == "fail" and .ratchet.new_diagnostics == 0 and
   .ratchet.metric_growth == 1 and
   any(.diagnostics[]; .rule == "ratchet/metric-growth" and
     .path == "src/fixture.rs" and .subject == "function-lines\u001fsrc/fixture.rs\u001fbaseline_function" and
     .actual == 102 and .limit == 101)'

printf '%s\n' 'fn broken( {' >"$fixture/src/fixture.rs"
run_expect 2 'parse failure' "$baseline_command" --repo-root "$fixture"
assert_report 'parse failure report' "$command_output" \
  '.status == "parse-error" and .baseline_written == false and
   (.parse_errors | length) > 0'
if ! cmp -s "$fixture/.source-quality/baseline.json" "$test_root/baseline-before-parse.json"; then
  printf '%s\n' 'contract failure: parse failure changed the baseline' >&2
  exit 1
fi

write_rust_function 99 baseline_function "$fixture/src/fixture.rs"
git -C "$fixture" add src/fixture.rs
write_rust_function 100 baseline_function "$fixture/src/fixture.rs"
run_expect 0 'staged check uses index contents' \
  "$check_staged_command" --repo-root "$fixture"
assert_report 'staged clean report' "$command_output" \
  '.status == "pass" and .mode == "staged" and
   .scope.checked_paths == ["src/fixture.rs"] and
   .ratchet.metric_growth == 0'

mkdir -p "$fixture/web/src"
printf '%s\n' 'export const staged_value = 1;' >"$fixture/web/src/staged.ts"
git -C "$fixture" add web/src/staged.ts
printf '%s\n' 'const staged_value = ;' >"$fixture/web/src/staged.ts"
run_expect 0 'staged Biome check uses index contents' \
  "$check_staged_command" --repo-root "$fixture"
assert_report 'staged Biome report' "$command_output" \
   '.status == "pass" and .mode == "staged" and
   ((.scope.checked_paths | index("web/src/staged.ts")) != null) and
   (.parse_errors | length) == 0 and
   ([.diagnostics[] | select(.path == "web/src/staged.ts")] | length) == 0'

git -C "$fixture" add src/fixture.rs
run_expect 1 'staged metric growth' \
  "$check_staged_command" --repo-root "$fixture"
assert_report 'staged growth report' "$command_output" \
  '.status == "fail" and .mode == "staged" and
   .ratchet.metric_growth == 1 and
   any(.diagnostics[]; .rule == "ratchet/metric-growth" and .actual == 102)'

write_rust_function 99 baseline_function "$fixture/src/fixture.rs"
printf '%s\n' 'export const staged_value = 1;' >"$fixture/web/src/staged.ts"
git -C "$fixture" add src/fixture.rs
git -C "$fixture" add web/src/staged.ts
run_expect 1 'require-zero rejects diagnostics' \
  "$require_zero_command" --repo-root "$fixture"
assert_report 'require-zero rejection report' "$command_output" \
  '.status == "fail" and .zero_transitioned == false and
   any(.diagnostics[]; .rule == "function-lines" and .actual == 101)'
if [ -e "$fixture/.source-quality/state.json" ]; then
  printf '%s\n' 'contract failure: rejected zero transition created state' >&2
  exit 1
fi

write_rust_function 98 baseline_function "$fixture/src/fixture.rs"
git -C "$fixture" add src/fixture.rs
run_expect 0 'require-zero transition' \
  "$require_zero_command" --repo-root "$fixture"
assert_report 'zero transition report' "$command_output" \
  '.status == "pass" and .zero_transitioned == true and
   (.diagnostics | length) == 0'
if [ -e "$fixture/.source-quality/baseline.json" ]; then
  printf '%s\n' 'contract failure: zero transition kept the baseline' >&2
  exit 1
fi
assert_report 'zero state file' "$(sed -n '1,120p' "$fixture/.source-quality/state.json")" \
  '.schema_version == 1 and .mode == "zero"'

run_expect 2 'baseline rollback after zero' \
  "$baseline_command" --repo-root "$fixture"
assert_report 'irreversible zero report' "$command_output" \
  '.status == "error" and any(.errors[]; contains("irreversible"))'

cp "$test_root/baseline-before-parse.json" "$fixture/.source-quality/baseline.json"
run_expect 2 'zero mode rejects restored baseline' \
  "$check_all_command" --repo-root "$fixture"
assert_report 'restored baseline rejection report' "$command_output" \
  '.status == "error" and
   any(.errors[]; contains("zero mode cannot have a baseline file"))'

printf '%s\n' 'source-quality contract tests passed'
