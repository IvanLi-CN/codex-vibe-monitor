#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
dockerfile="$repo_root/Dockerfile"
runner="$repo_root/.github/scripts/run-backend-tests.sh"
compose_file="$repo_root/compose.backend-test.yml"
ci_main_workflow="$repo_root/.github/workflows/ci-main.yml"
release_workflow="$repo_root/.github/workflows/release.yml"

grep -q '^FROM rust:1.96.0-bookworm AS backend-test$' "$dockerfile"
grep -q '^  backend-test:$' "$compose_file"
grep -q 'target: backend-test' "$compose_file"
grep -q 'CARGO_NEXTEST_VERSION=0.9.138' "$dockerfile"
grep -q 'CARGO_NEXTEST_SHA256_AMD64=3793bf0c27607b196f502c39b2108f571de89fcda7586ae6beefa11ee177b216' "$dockerfile"
grep -q 'install -m 0755 /tmp/cargo-nextest /usr/local/cargo/bin/cargo-nextest' "$dockerfile"
grep -q '^COPY scripts/search-raw ./scripts/search-raw$' "$dockerfile"
grep -q 'ENTRYPOINT \["bash", ".github/scripts/run-backend-tests.sh"\]' "$dockerfile"
grep -q '^    entrypoint: \[\]$' "$compose_file"
grep -q '^    command: \["sleep", "infinity"\]$' "$compose_file"

if ! grep -Fq 'id: backend-test-image-name' "$ci_main_workflow"; then
  echo 'expected CI Main to normalize the backend-test GHCR image name' >&2
  exit 1
fi

if ! grep -Fq 'image_name=${GITHUB_REPOSITORY,,}' "$ci_main_workflow"; then
  echo 'expected CI Main backend-test image name to use Bash lowercase normalization' >&2
  exit 1
fi

if ! grep -Fq 'tags: ${{ env.REGISTRY }}/${{ steps.backend-test-image-name.outputs.image_name }}:backend-test-${{ github.sha }}' "$ci_main_workflow"; then
  echo 'expected CI Main backend-test image tag to use the normalized image name' >&2
  exit 1
fi

if grep -Fq 'tags: ${{ env.REGISTRY }}/${{ github.repository }}:backend-test-${{ github.sha }}' "$ci_main_workflow"; then
  echo 'CI Main must not publish the backend-test image with an unnormalized repository name' >&2
  exit 1
fi

grep -q '^FROM production-runtime AS runtime$' "$dockerfile"

release_amd_smoke_build="$(sed -n '/^      - name: Build smoke image (linux\/amd64, load)$/,/^      - name: Smoke test image (linux\/amd64)$/p' "$release_workflow")"
if ! grep -Fq 'target: runtime' <<<"$release_amd_smoke_build"; then
  echo 'release amd64 smoke build must use the runtime image target' >&2
  exit 1
fi

if ! grep -Fq -- '--target "runtime"' "$repo_root/.github/scripts/build-smoke-image-with-retry.sh"; then
  echo 'release arm64 smoke build helper must use the runtime image target' >&2
  exit 1
fi

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
