#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
runner_script="$repo_root/scripts/run-summary-production-copy-testbox.sh"
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
grep -Fq 'validation_command="${SUMMARY_PRODUCTION_VALIDATION_COMMAND:-}"' "$runner_script"
grep -Fq "printf -v validation_command_export '%q' \"\$validation_command\"" "$runner_script"
grep -Fq 'export SUMMARY_PRODUCTION_VALIDATION_COMMAND=${validation_command_export};' "$runner_script"
grep -Fq 'summary-production-(sqlite|health|window|bootstrap|overlay|recovery(-telemetry)?|validation)' "$runner_script"
grep -Fq 'SUMMARY_PRODUCTION_RECENT_READY_DEADLINE_SECS' "$runner_script"
grep -Fq 'SUMMARY_PRODUCTION_HISTORICAL_READY_DEADLINE_SECS' "$runner_script"
grep -Fq 'SUMMARY_PRODUCTION_RECOVERY_DIAGNOSTICS' "$runner_script"
grep -Fq 'recent_ready_deadline_secs="${SUMMARY_PRODUCTION_RECENT_READY_DEADLINE_SECS:-30}"' \
  "$repo_root/scripts/validate-summary-production-fixture.sh"
grep -Fq 'bash -c "$SUMMARY_PRODUCTION_VALIDATION_COMMAND"' \
  "$repo_root/scripts/validate-summary-production-fixture.sh"
grep -Fq 'database_path="$copy_path/codex_vibe_monitor.db"' \
  "$repo_root/scripts/validate-summary-production-fixture.sh"
grep -Fq 'archive_dir="$copy_path/archives"' \
  "$repo_root/scripts/validate-summary-production-fixture.sh"
grep -Fq 'summary-production-archive-remap=verified=' \
  "$repo_root/scripts/validate-summary-production-fixture.sh"
grep -Fq 'summary-production-exact-oracle.py' \
  "$repo_root/scripts/validate-summary-production-fixture.sh"
grep -Fq 'summary_validate_exact_response' \
  "$repo_root/scripts/validate-summary-production-fixture.sh"
grep -Fq 'summary-production-exactness=' \
  "$repo_root/scripts/summary-production-exact-oracle.py"
grep -Fq 'UPDATE archive_batches SET file_path = ?1 WHERE id = ?2' \
  "$repo_root/scripts/validate-summary-production-fixture.sh"
grep -Fq 'summary-production-recovery-telemetry=' \
  "$repo_root/scripts/validate-summary-production-fixture.sh"
grep -Fq 'ansi_pattern = re.compile' \
  "$repo_root/scripts/validate-summary-production-fixture.sh"
if grep -Fq 'bash -lc "$SUMMARY_PRODUCTION_VALIDATION_COMMAND"' \
  "$repo_root/scripts/validate-summary-production-fixture.sh"; then
  printf 'custom validation must preserve the test image toolchain PATH\n' >&2
  exit 1
fi
expect_failure 1 env -u SUMMARY_PRODUCTION_COPY \
  "$repo_root/scripts/validate-summary-production-fixture.sh"

printf 'summary-production-copy-testbox-contract=passed\n'
