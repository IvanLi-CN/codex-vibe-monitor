#!/usr/bin/env bash
set -euo pipefail

build_platform="${BUILD_PLATFORM:-}"
smoke_tag="${SMOKE_TAG:-}"
candidate_tag="${CANDIDATE_TAG:-}"
app_effective_version="${APP_EFFECTIVE_VERSION:-}"
cache_ref="${CACHE_REF:-}"
build_context="${BUILD_CONTEXT:-.}"
dockerfile_path="${DOCKERFILE_PATH:-./Dockerfile}"
retry_attempts="${BUILD_RETRY_ATTEMPTS:-5}"
retry_base_delay_secs="${BUILD_RETRY_BASE_DELAY_SECS:-3}"

if [[ -z "${build_platform}" || -z "${smoke_tag}" || -z "${candidate_tag}" || -z "${app_effective_version}" || -z "${cache_ref}" ]]; then
  echo "missing required build env: BUILD_PLATFORM, SMOKE_TAG, CANDIDATE_TAG, APP_EFFECTIVE_VERSION, CACHE_REF" >&2
  exit 2
fi

is_transient_failure() {
  local err_file="$1"
  local patterns=(
    "context deadline exceeded"
    "DeadlineExceeded"
    "TLS handshake timeout"
    "Error in the HTTP2 framing layer"
    "curl failed"
    "i/o timeout"
    "connection reset by peer"
    "unexpected EOF"
    "temporary failure in name resolution"
    "no route to host"
    "429 Too Many Requests"
    "toomanyrequests"
    "request canceled while waiting for connection"
  )

  for pattern in "${patterns[@]}"; do
    if grep -Fqi "${pattern}" "${err_file}"; then
      return 0
    fi
  done

  return 1
}

for attempt in $(seq 1 "${retry_attempts}"); do
  err_file="$(mktemp)"

  if docker buildx build \
    --progress plain \
    --file "${dockerfile_path}" \
    --platform "${build_platform}" \
    --load \
    --tag "${smoke_tag}" \
    --tag "${candidate_tag}" \
    --build-arg "APP_EFFECTIVE_VERSION=${app_effective_version}" \
    --cache-from "type=registry,ref=${cache_ref}" \
    --cache-to "type=registry,ref=${cache_ref},mode=max" \
    "${build_context}" 2>"${err_file}"; then
    rm -f "${err_file}"
    exit 0
  fi

  if ! is_transient_failure "${err_file}"; then
    echo "[buildx] non-retryable failure for ${build_platform} on attempt ${attempt}/${retry_attempts}" >&2
    cat "${err_file}" >&2
    rm -f "${err_file}"
    exit 1
  fi

  if [[ "${attempt}" == "${retry_attempts}" ]]; then
    echo "[buildx] transient failure exhausted retries for ${build_platform} after ${retry_attempts} attempts" >&2
    cat "${err_file}" >&2
    rm -f "${err_file}"
    exit 1
  fi

  sleep_secs=$((attempt * retry_base_delay_secs))
  echo "[buildx] transient failure for ${build_platform}; retry in ${sleep_secs}s (${attempt}/${retry_attempts})" >&2
  tail -n 40 "${err_file}" >&2 || cat "${err_file}" >&2
  rm -f "${err_file}"
  sleep "${sleep_secs}"
done
