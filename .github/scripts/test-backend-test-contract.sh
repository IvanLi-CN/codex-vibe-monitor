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
grep -q 'ENTRYPOINT \["bash", ".github/scripts/run-backend-tests.sh"\]' "$dockerfile"

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

if ! grep -Fq 'image="${REGISTRY}/${GITHUB_REPOSITORY,,}:backend-test-${TARGET_SHA}"' "$release_workflow"; then
  echo 'expected manual release digest lookup to use the normalized backend-test image name' >&2
  exit 1
fi

if grep -Fq 'image="${REGISTRY}/${GITHUB_REPOSITORY}:backend-test-${TARGET_SHA}"' "$release_workflow"; then
  echo 'manual release digest lookup must not use an unnormalized repository name' >&2
  exit 1
fi

manual_backfill_step="$(sed -n '/^      - name: Ensure immutable release snapshot for manual backfill$/,/^      - name: Select pending release target$/p' "$release_workflow")"
if ! grep -Fq -- '--backend-test-image-digest "${{ steps.manual-backend-test-image.outputs.digest }}"' <<<"$manual_backfill_step"; then
  echo 'manual snapshot backfill must bind the resolved backend-test image digest' >&2
  exit 1
fi

pending_recovery_steps="$(sed -n '/^      - name: Set up Docker Buildx for pending release digest lookup$/,/^      - name: Load immutable release snapshot$/p' "$release_workflow")"
for required in \
  "if: github.event_name != 'workflow_dispatch' && steps.pending-target.outputs.target_sha != ''" \
  'image="${REGISTRY}/${GITHUB_REPOSITORY,,}:backend-test-${TARGET_SHA}"' \
  'python3 .github/scripts/release_snapshot.py ensure' \
  '--backend-test-image-digest "${BACKEND_TEST_IMAGE_DIGEST}"' \
  '--skip-publish'; do
  if ! grep -Fq -- "$required" <<<"$pending_recovery_steps"; then
    echo "automatic pending-release snapshot recovery is missing: $required" >&2
    exit 1
  fi
done

github_repository_fixture='IvanLi-CN/Codex-Vibe-Monitor'
target_sha_fixture='deadbeef'
manual_image="ghcr.io/$(printf '%s' "$github_repository_fixture" | tr '[:upper:]' '[:lower:]'):backend-test-${target_sha_fixture}"
[[ "$manual_image" == 'ghcr.io/ivanli-cn/codex-vibe-monitor:backend-test-deadbeef' ]]

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
