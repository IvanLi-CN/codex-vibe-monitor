#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
script="$repo_root/.github/scripts/build-smoke-image-with-retry.sh"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "${tmp_dir}"' EXIT

cat >"$tmp_dir/docker-transient" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
attempt_file="${FAKE_DOCKER_ATTEMPT_FILE:?}"
count=0
if [[ -f "$attempt_file" ]]; then
  count="$(cat "$attempt_file")"
fi
count=$((count + 1))
printf '%s' "$count" >"$attempt_file"
if [[ "$count" -lt 3 ]]; then
  echo "failed to authorize: DeadlineExceeded: context deadline exceeded" >&2
  exit 1
fi
exit 0
EOF

cat >"$tmp_dir/docker-permanent" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo "manifest for docker.io/library/rust:1.96.0-bookworm not found" >&2
exit 1
EOF

chmod +x "$tmp_dir/docker-transient" "$tmp_dir/docker-permanent"

transient_path="$tmp_dir/transient-bin"
mkdir -p "$transient_path"
cp "$tmp_dir/docker-transient" "$transient_path/docker"

attempt_file="$tmp_dir/transient-attempts"
PATH="$transient_path:$PATH" \
FAKE_DOCKER_ATTEMPT_FILE="$attempt_file" \
BUILD_PLATFORM="linux/arm64" \
SMOKE_TAG="ghcr.io/example/smoke:arm64" \
CANDIDATE_TAG="ghcr.io/example/candidate:arm64" \
APP_EFFECTIVE_VERSION="test-version" \
CACHE_REF="ghcr.io/example/buildcache:arm64" \
BUILD_RETRY_ATTEMPTS="5" \
BUILD_RETRY_BASE_DELAY_SECS="0" \
bash "$script" >"$tmp_dir/transient.out" 2>"$tmp_dir/transient.err"

[[ "$(cat "$attempt_file")" == "3" ]]
grep -q "transient failure for linux/arm64; retry in 0s (1/5)" "$tmp_dir/transient.err"
grep -q "transient failure for linux/arm64; retry in 0s (2/5)" "$tmp_dir/transient.err"

permanent_path="$tmp_dir/permanent-bin"
mkdir -p "$permanent_path"
cp "$tmp_dir/docker-permanent" "$permanent_path/docker"

set +e
PATH="$permanent_path:$PATH" \
BUILD_PLATFORM="linux/arm64" \
SMOKE_TAG="ghcr.io/example/smoke:arm64" \
CANDIDATE_TAG="ghcr.io/example/candidate:arm64" \
APP_EFFECTIVE_VERSION="test-version" \
CACHE_REF="ghcr.io/example/buildcache:arm64" \
BUILD_RETRY_ATTEMPTS="5" \
BUILD_RETRY_BASE_DELAY_SECS="0" \
bash "$script" >"$tmp_dir/permanent.out" 2>"$tmp_dir/permanent.err"
rc=$?
set -e

[[ "$rc" == "1" ]]
grep -q "non-retryable failure for linux/arm64 on attempt 1/5" "$tmp_dir/permanent.err"
if grep -q "retry in" "$tmp_dir/permanent.err"; then
  echo "expected permanent failure path to stop without retries" >&2
  exit 1
fi

echo "test-build-smoke-image-with-retry: all checks passed"
