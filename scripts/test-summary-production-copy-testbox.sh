#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
runner_script="$repo_root/scripts/run-summary-production-copy-testbox.sh"
validator_script="$repo_root/scripts/validate-summary-production-fixture.sh"
expect_failure() {
  local expected="$1"
  shift
  set +e
  "$@" >/dev/null 2>&1
  local status="$?"
  set -e
  [[ "$status" -eq "$expected" ]] || {
    printf 'expected exit %s, got %s\n' "$expected" "$status" >&2
    exit 1
  }
}
expect_failure_message() {
  local expected_status="$1"
  local expected_message="$2"
  shift 2
  set +e
  local output
  output="$("$@" 2>&1)"
  local status="$?"
  set -e
  [[ "$status" -eq "$expected_status" ]] || {
    printf 'expected exit %s, got %s\n' "$expected_status" "$status" >&2
    exit 1
  }
  grep -Fq "$expected_message" <<<"$output" || {
    printf 'expected failure output to contain: %s\n' "$expected_message" >&2
    exit 1
  }
}

expect_failure 64 env \
  CODEX_THREAD_ID=fixture-agent \
  SUMMARY_TESTBOX_PRODUCTION_COPY_SOURCE=/tmp/not-authorized \
  "$repo_root/scripts/run-summary-production-copy-testbox.sh"
expect_failure 64 env \
  CODEX_THREAD_ID=fixture-agent \
  SUMMARY_TESTBOX_PRODUCTION_COPY_SOURCE=/srv/codex/example \
  "$repo_root/scripts/run-summary-production-copy-testbox.sh"
if grep -Fq 'SUMMARY_PRODUCTION_VALIDATION_COMMAND' "$runner_script" \
  || grep -Fq 'SUMMARY_PRODUCTION_VALIDATION_COMMAND' "$validator_script" \
  || grep -Fq 'shared-testbox-runner' "$runner_script"; then
  printf 'validation command overrides are not allowed in the production-copy gate\n' >&2
  exit 1
fi
if grep -Eq '/srv/codex/workspaces|/Users/ivan/.codex/skills/shared-testbox-runner' "$runner_script"; then
  printf 'shared-testbox adapter must use the Agent Directory contract\n' >&2
  exit 1
fi
grep -Fq 'CODEX_THREAD_ID' "$runner_script"
grep -Fq '/srv/codex/agents/' "$runner_script"
grep -Fq 'docker_args=(' "$runner_script"
grep -Fq 'testbox-sync-worktree' "$runner_script" && {
  printf 'project adapter must not require the helper transport layer\n' >&2
  exit 1
}
grep -Eq 'summary-production-(cache-mode|network-mode|sqlite|health|startup-phases|window|bootstrap|exactness|overlay|recovery(-telemetry)?|validation)=' "$runner_script"
grep -Fq 'touch "$run_path/READY"' "$runner_script"
grep -Fq 'export SUMMARY_PRODUCTION_COPY=/codex-scratch/production-copy' "$runner_script"
if grep -Fq 'CARGO_TARGET_DIR=/codex-scratch/target' "$runner_script"; then
  printf 'shared-testbox adapter must not force a Cargo target path\n' >&2
  exit 1
fi
grep -Fq 'SUMMARY_PRODUCTION_RECENT_READY_DEADLINE_SECS' "$runner_script"
grep -Fq 'SUMMARY_PRODUCTION_HISTORICAL_READY_DEADLINE_SECS' "$runner_script"
grep -Fq 'SUMMARY_PRODUCTION_RECOVERY_DIAGNOSTICS' "$runner_script"
grep -Fq 'CARGO_HOME_INPUT' "$runner_script"
grep -Fq 'CARGO_TARGET_INPUT' "$runner_script"
grep -Fq 'source_validation_status=' "$runner_script"
grep -Fq '10|11|12)' "$runner_script"
grep -Fq 'source validation was unavailable' "$runner_script"
grep -Fq 'trap cleanup_remote EXIT' "$runner_script"
grep -Fq 'rm -rf -- "$run_path"' "$runner_script"
if grep -Fq 'must be true or false' "$runner_script"; then
  printf 'shared-testbox adapter must pass CARGO_NET_OFFLINE through unchanged\n' >&2
  exit 1
fi
grep -Fq 'recent_ready_deadline_secs="${SUMMARY_PRODUCTION_RECENT_READY_DEADLINE_SECS:-30}"' "$validator_script"
grep -Fq 'database_path="$copy_path/codex_vibe_monitor.db"' "$validator_script"
grep -Fq 'archive_dir="$copy_path/archives"' "$validator_script"
grep -Fq 'summary-production-archive-remap=verified=' "$validator_script"
grep -Fq 'summary-production-exact-oracle.py' "$validator_script"
grep -Fq 'summary_validate_exact_response' "$validator_script"
grep -Fq 'repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"' "$validator_script"
grep -Fq 'runtime_dir_input=' "$validator_script"
grep -Fq 'canonical_dir_path()' "$validator_script"
grep -Fq 'trap cleanup_runtime EXIT' "$validator_script"
grep -Fq 'source_snapshot_root=' "$validator_script"
if grep -Eq '/workspace|/codex-scratch|/srv/app/data' "$validator_script"; then
  printf 'production-copy validator must not depend on runner-private paths\n' >&2
  exit 1
fi
grep -Fq 'CARGO_HOME' "$validator_script"
grep -Fq 'CARGO_TARGET_DIR' "$validator_script"
grep -Fq 'summary-production-exactness=' \
  "$repo_root/scripts/summary-production-exact-oracle.py"
grep -Fq 'UPDATE archive_batches SET file_path = ?1 WHERE id = ?2' "$validator_script"
grep -Fq 'summary-production-recovery-telemetry=' "$validator_script"
grep -Fq 'ansi_pattern = re.compile' "$validator_script"
tmp_dir="$(mktemp -d)"
copy_dir="$tmp_dir/production-copy"
runtime_root="$tmp_dir/runtime-root"
mkdir -p "$copy_dir" "$runtime_root"
copy_dir="$(cd "$copy_dir" && pwd -P)"
trap 'rm -rf "$tmp_dir"' EXIT
ln -s "$tmp_dir" "$tmp_dir/parent-link"
ln -s "$repo_root/does-not-exist" "$tmp_dir/dangling-parent"
expect_failure 64 env \
  SUMMARY_PRODUCTION_COPY="$tmp_dir/parent-link/production-copy" \
  "$validator_script"
expect_failure 64 env \
  SUMMARY_PRODUCTION_COPY="$copy_dir" \
  TMPDIR="$tmp_dir/dangling-parent/nested" \
  "$validator_script"
expect_failure 64 env \
  SUMMARY_PRODUCTION_COPY="$copy_dir" \
  TMPDIR="$copy_dir" \
  "$validator_script"
expect_failure 64 env \
  SUMMARY_PRODUCTION_COPY="$copy_dir" \
  CARGO_HOME="$copy_dir/cargo-home" \
  CARGO_TARGET_DIR="$tmp_dir/target" \
  TMPDIR="$runtime_root" \
  "$validator_script"
[[ ! -e "$copy_dir/cargo-home" ]]
expect_failure_message 64 'temporary directory root must be separate from fixture and source workspaces' env \
  SUMMARY_PRODUCTION_COPY="$copy_dir" \
  TMPDIR="$repo_root" \
  "$validator_script"
expect_failure_message 64 'temporary directory root must be separate from fixture and source workspaces' env \
  SUMMARY_PRODUCTION_COPY="$copy_dir/" \
  TMPDIR="$repo_root" \
  "$validator_script"
expect_failure_message 64 'staged production copy must be separate from the source snapshot' env \
  SUMMARY_PRODUCTION_COPY="$repo_root" \
  TMPDIR="$runtime_root" \
  "$validator_script"
expect_failure_message 64 'external Cargo directories must be separate from fixture and runtime workspaces' env \
  SUMMARY_PRODUCTION_COPY="$copy_dir" \
  CARGO_HOME="$tmp_dir/external-cargo-home" \
  CARGO_TARGET_DIR="$repo_root/target" \
  TMPDIR="$runtime_root" \
  "$validator_script"
expect_failure 1 env -u SUMMARY_PRODUCTION_COPY \
  "$repo_root/scripts/validate-summary-production-fixture.sh"

fake_bin="$tmp_dir/fake-bin"
mkdir -p "$fake_bin"
cat >"$fake_bin/ssh" <<'EOF'
#!/usr/bin/env bash
exit 255
EOF
chmod +x "$fake_bin/ssh"
expect_failure_message 75 'source validation was unavailable' env \
  PATH="$fake_bin:/usr/bin:/bin" \
  CODEX_THREAD_ID=fixture-agent \
  SUMMARY_TESTBOX_PRODUCTION_COPY_SOURCE=/srv/codex/agents/fixture-agent/example \
  "$runner_script"

printf 'summary-production-copy-testbox-contract=passed\n'
