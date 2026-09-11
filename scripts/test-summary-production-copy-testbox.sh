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

expect_failure 64 env \
  SUMMARY_TESTBOX_PRODUCTION_COPY_SOURCE=/tmp/not-authorized \
  "$repo_root/scripts/run-summary-production-copy-testbox.sh"
expect_failure 64 env \
  SUMMARY_TESTBOX_PRODUCTION_COPY_SOURCE=/srv/codex/example \
  SUMMARY_TESTBOX_RUNNER=/definitely/missing/runner \
  "$repo_root/scripts/run-summary-production-copy-testbox.sh"
if grep -Fq 'SUMMARY_PRODUCTION_VALIDATION_COMMAND' "$runner_script" \
  || grep -Fq 'SUMMARY_PRODUCTION_VALIDATION_COMMAND' "$validator_script"; then
  printf 'validation command overrides are not allowed in the production-copy gate\n' >&2
  exit 1
fi
grep -Eq 'summary-production-(sqlite|health|startup-phases|window|bootstrap|exactness|overlay|recovery(-telemetry)?|validation)=' "$runner_script"
grep -Fq 'while [[ ! -f /codex-scratch/READY ]]' "$runner_script"
grep -Fq 'export SUMMARY_PRODUCTION_COPY=/codex-scratch/production-copy' "$runner_script"
if grep -Fq 'CARGO_TARGET_DIR=/codex-scratch/target' "$runner_script"; then
  printf 'shared-testbox adapter must not force a Cargo target path\n' >&2
  exit 1
fi
grep -Fq 'SUMMARY_PRODUCTION_RECENT_READY_DEADLINE_SECS' "$runner_script"
grep -Fq 'SUMMARY_PRODUCTION_HISTORICAL_READY_DEADLINE_SECS' "$runner_script"
grep -Fq 'SUMMARY_PRODUCTION_RECOVERY_DIAGNOSTICS' "$runner_script"
grep -Fq '  CARGO_HOME' "$runner_script"
grep -Fq '  CARGO_TARGET_DIR' "$runner_script"
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
mkdir -p "$copy_dir"
trap 'rm -rf "$tmp_dir"' EXIT
ln -s "$tmp_dir" "$tmp_dir/parent-link"
expect_failure 64 env \
  SUMMARY_PRODUCTION_COPY="$tmp_dir/parent-link/production-copy" \
  "$validator_script"
expect_failure 64 env \
  SUMMARY_PRODUCTION_COPY="$copy_dir" \
  TMPDIR="$copy_dir" \
  "$validator_script"
expect_failure 64 env \
  SUMMARY_PRODUCTION_COPY="$copy_dir" \
  CARGO_HOME="$copy_dir/cargo-home" \
  CARGO_TARGET_DIR="$tmp_dir/target" \
  "$validator_script"
[[ ! -e "$copy_dir/cargo-home" ]]
expect_failure 1 env -u SUMMARY_PRODUCTION_COPY \
  "$repo_root/scripts/validate-summary-production-fixture.sh"

printf 'summary-production-copy-testbox-contract=passed\n'
