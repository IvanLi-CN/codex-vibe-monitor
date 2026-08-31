#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
dockerfile="$repo_root/Dockerfile"
runner="$repo_root/.github/scripts/run-backend-tests.sh"

grep -q '^FROM rust:1.96.0-bookworm AS backend-test$' "$dockerfile"
grep -q 'CARGO_NEXTEST_VERSION=0.9.138' "$dockerfile"
grep -q 'CARGO_NEXTEST_SHA256_AMD64=3793bf0c27607b196f502c39b2108f571de89fcda7586ae6beefa11ee177b216' "$dockerfile"
grep -q 'ENTRYPOINT \["bash", ".github/scripts/run-backend-tests.sh"\]' "$dockerfile"

set +e
missing_nextest_output="$(PATH=/usr/bin:/bin bash "$runner" --profile stateful-sqlite 2>&1)"
missing_nextest_rc=$?
set -e
[[ "$missing_nextest_rc" == 1 ]]
grep -q 'cargo-nextest is not installed' <<<"$missing_nextest_output"

set +e
invalid_workspace_output="$(BACKEND_TEST_WORKSPACE=/workspace bash "$runner" --profile lightweight 2>&1)"
invalid_workspace_rc=$?
set -e
[[ "$invalid_workspace_rc" == 64 ]]
grep -q 'BACKEND_TEST_WORKSPACE must be a path under /tmp' <<<"$invalid_workspace_output"

printf '%s\n' 'test-backend-test-contract: all checks passed'
