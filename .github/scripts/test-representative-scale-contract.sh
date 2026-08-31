#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
dockerfile="$repo_root/Dockerfile"
runner="$repo_root/.github/scripts/run-backend-tests.sh"
test_file="$repo_root/src/tests/stateful_sqlite/representative_scale_acceptance.rs"

grep -Fq 'AS backend-test' "$dockerfile"
grep -Fq 'CARGO_NEXTEST_VERSION=0.9.138' "$dockerfile"
grep -Fq 'CARGO_NEXTEST_SHA256_AMD64=3793bf0c27607b196f502c39b2108f571de89fcda7586ae6beefa11ee177b216' "$dockerfile"
grep -Fq 'ENTRYPOINT ["bash", ".github/scripts/run-backend-tests.sh"]' "$dockerfile"
grep -Fq 'FIXTURE_CONTRACT_VERSION: &str = "summary-representative-scale-v1"' "$test_file"
grep -Fq 'ORACLE_VERSION: &str = "summary-oracle-v1"' "$test_file"
grep -Fq 'summary_representative_scale_acceptance' "$test_file"
grep -Fq 'cargo nextest run' "$runner"

set +e
missing_nextest_output="$({
  cd "$repo_root"
  PATH=/usr/bin:/bin BACKEND_TEST_WORKSPACE=/tmp/codex-vibe-monitor-contract-test \
    bash .github/scripts/run-backend-tests.sh --profile stateful-sqlite
} 2>&1)"
missing_nextest_status=$?
set -e
test "$missing_nextest_status" -eq 1
grep -Fq 'cargo-nextest is not installed' <<<"$missing_nextest_output"

set +e
invalid_workspace_output="$({
  cd "$repo_root"
  BACKEND_TEST_WORKSPACE=/workspace bash .github/scripts/run-backend-tests.sh --profile lightweight
} 2>&1)"
invalid_workspace_status=$?
set -e
test "$invalid_workspace_status" -eq 64
grep -Fq 'BACKEND_TEST_WORKSPACE must be a path under /tmp' <<<"$invalid_workspace_output"

printf 'test-representative-scale-contract: all checks passed\n'
