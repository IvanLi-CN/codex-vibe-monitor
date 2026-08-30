#!/usr/bin/env bash
set -euo pipefail

# This script is intentionally data-blind: it verifies only bounded startup
# timing and HTTP status codes against an isolated production-data copy.
fixture_dir="${SUMMARY_PRODUCTION_FIXTURE_DIR:-/workspace/production-fixture}"
database_path="$fixture_dir/codex_vibe_monitor.db"
archive_dir="$fixture_dir/archives"
port="${SUMMARY_PRODUCTION_FIXTURE_PORT:-18080}"
timeout_seconds="${SUMMARY_PRODUCTION_FIXTURE_TIMEOUT_SECS:-1800}"
cargo_target_dir="${SUMMARY_PRODUCTION_FIXTURE_CARGO_TARGET_DIR:-/tmp/codex-vibe-monitor-summary-target}"
app_log="$fixture_dir/summary-projection-validation.log"
app_pid=""

if [[ ! -f "$database_path" || ! -d "$archive_dir" ]]; then
  printf 'summary_fixture_input_missing\n' >&2
  exit 64
fi
if [[ ! "$port" =~ ^[0-9]{2,5}$ || ! "$timeout_seconds" =~ ^[0-9]+$ ||
  "$cargo_target_dir" != /tmp/* || "$cargo_target_dir" == *..* ]]; then
  printf 'summary_fixture_invalid_configuration\n' >&2
  exit 64
fi
mkdir -p "$cargo_target_dir"

cleanup() {
  if [[ -n "$app_pid" ]] && kill -0 "$app_pid" 2>/dev/null; then
    kill "$app_pid" 2>/dev/null || true
    wait "$app_pid" 2>/dev/null || true
  fi
  rm -f "$app_log"
}
trap cleanup EXIT HUP INT TERM

classify_app_exit() {
  local log_path="$1"

  if [[ ! -f "$log_path" ]]; then
    printf 'missing_log'
  elif grep -Eiq 'permission denied|failed to create.*target|failed to create.*workspace' "$log_path"; then
    printf 'workspace_write_failure'
  elif grep -Eiq 'database disk image is malformed|SQLITE_CORRUPT|file is not a database' "$log_path"; then
    printf 'sqlite_corruption'
  elif grep -Eiq 'failed to open sqlite database|unable to open database|cannot open.*database|SQLITE_CANTOPEN' "$log_path"; then
    printf 'sqlite_open_failure'
  elif grep -Eiq 'database is locked|SQLITE_BUSY' "$log_path"; then
    printf 'sqlite_busy'
  elif grep -Eiq 'error: could not compile|could not compile|rustc.*error' "$log_path"; then
    printf 'source_build_failure'
  elif grep -Eiq 'configuration|invalid .*environment|is not supported' "$log_path"; then
    printf 'runtime_configuration_failure'
  elif grep -Eiq 'migration|migrat' "$log_path"; then
    printf 'database_migration_failure'
  else
    printf 'startup_process_exit'
  fi
}

safe_failure_signature() {
  local log_path="$1"
  local line=""
  local token=""
  local normalized=""
  local -a signature=()

  [[ -f "$log_path" ]] || {
    printf 'missing_log'
    return
  }
  line="$(grep -Eim1 '(^error:|^Error:|failed|permission denied|cannot|could not|panic)' "$log_path" || true)"
  [[ -n "$line" ]] || {
    printf 'no_classified_error'
    return
  }
  for token in $line; do
    normalized="${token,,}"
    normalized="${normalized//[^a-z]/}"
    case "$normalized" in
      error|failed|to|open|sqlite|database|permission|denied|unable|could|not|find|cargo|toml|target|path|directory|schema|migration|connection|read|write|file|such|no|invalid|configuration|bind|address|already|in|use|returned|from|io|os|panic|create|workspace|lock|locked|corrupt|malformed)
        signature+=("$normalized")
        ;;
      *)
        signature+=('<redacted>')
        ;;
    esac
    (( ${#signature[@]} >= 24 )) && break
  done
  printf '%s' "${signature[*]}"
}

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
    CARGO_TARGET_DIR="$cargo_target_dir" \
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
    reason="$(classify_app_exit "$app_log")"
    signature="$(safe_failure_signature "$app_log")"
    printf 'summary_fixture_process_exited elapsed_s=%s reason=%s signature=%s\n' \
      "$elapsed_seconds" "$reason" "$signature" >&2
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
