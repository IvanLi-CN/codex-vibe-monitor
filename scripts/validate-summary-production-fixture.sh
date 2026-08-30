#!/usr/bin/env bash
set -euo pipefail

# This script is intentionally data-blind: it verifies only bounded startup
# timing and HTTP status codes against an isolated production-data copy.
fixture_dir="${SUMMARY_PRODUCTION_FIXTURE_DIR:-/workspace/production-fixture}"
database_path="$fixture_dir/codex_vibe_monitor.db"
archive_dir="$fixture_dir/archives"
port="${SUMMARY_PRODUCTION_FIXTURE_PORT:-18080}"
timeout_seconds="${SUMMARY_PRODUCTION_FIXTURE_TIMEOUT_SECS:-1800}"
app_log="$fixture_dir/summary-projection-validation.log"
app_pid=""

if [[ ! -f "$database_path" || ! -d "$archive_dir" ]]; then
  printf 'summary_fixture_input_missing\n' >&2
  exit 64
fi
if [[ ! "$port" =~ ^[0-9]{2,5}$ || ! "$timeout_seconds" =~ ^[0-9]+$ ]]; then
  printf 'summary_fixture_invalid_configuration\n' >&2
  exit 64
fi

cleanup() {
  if [[ -n "$app_pid" ]] && kill -0 "$app_pid" 2>/dev/null; then
    kill "$app_pid" 2>/dev/null || true
    wait "$app_pid" 2>/dev/null || true
  fi
  rm -f "$app_log"
}
trap cleanup EXIT HUP INT TERM

http_status() {
  local path="$1"
  local status_line=""

  if ! { exec 3<>"/dev/tcp/127.0.0.1/$port"; } 2>/dev/null; then
    printf '000'
    return
  fi
  printf 'GET %s HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n' "$path" >&3
  IFS=$'\r' read -r status_line <&3 || true
  exec 3>&-
  exec 3<&-
  if [[ "$status_line" =~ ^HTTP/[0-9.]+\ ([0-9]{3}) ]]; then
    printf '%s' "${BASH_REMATCH[1]}"
  else
    printf '000'
  fi
}

(
  cd /workspace
  exec env \
    DATABASE_PATH="$database_path" \
    ARCHIVE_DIR="$archive_dir" \
    HTTP_BIND="127.0.0.1:$port" \
    OPENAI_UPSTREAM_BASE_URL="http://127.0.0.1:9/" \
    PROXY_USAGE_BACKFILL_ON_STARTUP=false \
    RETENTION_ENABLED=false \
    MAX_PARALLEL_POLLS=1 \
    SHARED_CONNECTION_PARALLELISM=1 \
    cargo run --locked -- --http-bind "127.0.0.1:$port"
) >"$app_log" 2>&1 &
app_pid="$!"

summary_paths=(
  'current?limit=50&timeZone=Asia%2FShanghai'
  '1d?limit=50&timeZone=Asia%2FShanghai'
  '7d?limit=50&timeZone=Asia%2FShanghai'
  '30d?limit=50&timeZone=Asia%2FShanghai'
  'today?limit=50&timeZone=Asia%2FShanghai'
  'all?limit=50&timeZone=Asia%2FShanghai'
)
summary_labels=(current one_day seven_days thirty_days today all_time)
started_at="$(date +%s)"

while :; do
  now="$(date +%s)"
  elapsed_seconds="$((now - started_at))"
  if ! kill -0 "$app_pid" 2>/dev/null; then
    printf 'summary_fixture_process_exited elapsed_s=%s\n' "$elapsed_seconds" >&2
    exit 1
  fi

  health_status="$(http_status '/health')"
  all_ready=true
  statuses=()
  for index in "${!summary_paths[@]}"; do
    status="$(http_status "/api/stats/summary?window=${summary_paths[$index]}")"
    statuses+=("${summary_labels[$index]}=$status")
    [[ "$status" == '200' ]] || all_ready=false
  done

  if [[ "$health_status" == '200' && "$all_ready" == true ]]; then
    printf 'summary_fixture_ready elapsed_s=%s health=%s %s\n' \
      "$elapsed_seconds" "$health_status" "${statuses[*]}"
    exit 0
  fi
  if (( elapsed_seconds % 30 == 0 )); then
    printf 'summary_fixture_wait elapsed_s=%s health=%s %s\n' \
      "$elapsed_seconds" "$health_status" "${statuses[*]}"
  fi
  if (( elapsed_seconds >= timeout_seconds )); then
    printf 'summary_fixture_timeout elapsed_s=%s health=%s %s\n' \
      "$elapsed_seconds" "$health_status" "${statuses[*]}" >&2
    exit 1
  fi
  sleep 1
done
