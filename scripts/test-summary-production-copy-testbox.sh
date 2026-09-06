#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
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
expect_failure 1 env -u SUMMARY_PRODUCTION_COPY \
  "$repo_root/scripts/validate-summary-production-fixture.sh"

printf 'summary-production-copy-testbox-contract=passed\n'
