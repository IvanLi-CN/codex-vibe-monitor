#!/usr/bin/env bash
set -euo pipefail

# This script is intentionally data-blind: it verifies only bounded startup
# timing and HTTP status codes against an isolated production-data copy.
fixture_dir="${SUMMARY_PRODUCTION_FIXTURE_DIR:-/workspace/production-fixture}"
database_path="$fixture_dir/codex_vibe_monitor.db"
archive_dir="$fixture_dir/archives"
runtime_workspace="${SUMMARY_PRODUCTION_FIXTURE_WORKSPACE:-/tmp/codex-vibe-monitor-summary-fixture-workspace}"
port="${SUMMARY_PRODUCTION_FIXTURE_PORT:-18080}"
timeout_seconds="${SUMMARY_PRODUCTION_FIXTURE_TIMEOUT_SECS:-1800}"
fixture_wait_seconds="${SUMMARY_PRODUCTION_FIXTURE_WAIT_SECS:-4800}"
cargo_target_dir="${SUMMARY_PRODUCTION_FIXTURE_CARGO_TARGET_DIR:-$runtime_workspace/target}"
app_log="$fixture_dir/summary-projection-validation.log"
app_pid=""

if [[ ! "$port" =~ ^[0-9]{2,5}$ || ! "$timeout_seconds" =~ ^[0-9]+$ ||
  ! "$fixture_wait_seconds" =~ ^[0-9]+$ || "$runtime_workspace" != /tmp/* ||
  "$runtime_workspace" == *..* || "$cargo_target_dir" != "$runtime_workspace"/* ||
  "$cargo_target_dir" == *..* ]]; then
  printf 'summary_fixture_invalid_configuration\n' >&2
  exit 64
fi
fixture_wait_started="$(date +%s)"
while [[ ! -f "$fixture_dir/READY" ]]; do
  if (( $(date +%s) - fixture_wait_started >= fixture_wait_seconds )); then
    printf 'summary_fixture_input_timeout\n' >&2
    exit 64
  fi
  sleep 1
done
if [[ ! -f "$database_path" || ! -d "$archive_dir" ]]; then
  printf 'summary_fixture_input_missing\n' >&2
  exit 64
fi
if ! mkdir -p "$runtime_workspace" "$cargo_target_dir" ||
  ! tar --exclude='./production-fixture' --exclude='./production-fixture/*' \
    --exclude='./.codex-*' --exclude='./target' --exclude='./target/*' \
    -C /workspace -cf - . | \
    tar --no-same-owner --no-same-permissions -C "$runtime_workspace" -xf - ||
  [[ -e "$runtime_workspace/production-fixture" ]]; then
  printf 'summary_fixture_workspace_copy_failure\n' >&2
  exit 1
fi

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
  elif grep -Eiq 'failed to create proxy raw payload directory' "$log_path"; then
    printf 'proxy_raw_directory_failure'
  elif grep -Eiq 'failed to create.*terminal.*directory|terminal.*journal.*permission denied' "$log_path"; then
    printf 'terminal_journal_directory_failure'
  elif grep -Eiq 'archive.*permission denied|permission denied.*archive' "$log_path"; then
    printf 'archive_fixture_permission_failure'
  elif grep -Eiq 'failed to create database directory' "$log_path"; then
    printf 'database_directory_failure'
  elif grep -Eiq 'failed to create.*migration temp table|failed to create.*trigger' "$log_path"; then
    printf 'schema_initialization_failure'
  elif grep -Eiq 'failed to create.*target' "$log_path"; then
    printf 'cargo_target_directory_failure'
  elif grep -Eiq 'failed to create.*workspace' "$log_path"; then
    printf 'cargo_workspace_directory_failure'
  elif grep -Eiq 'permission denied' "$log_path"; then
    printf 'filesystem_permission_failure'
  elif grep -Eiq 'database disk image is malformed|SQLITE_CORRUPT|file is not a database' "$log_path"; then
    printf 'sqlite_corruption'
  elif grep -Eiq 'failed to open sqlite database|unable to open database|cannot open.*database|SQLITE_CANTOPEN' "$log_path"; then
    printf 'sqlite_open_failure'
  elif grep -Eiq 'database is locked|SQLITE_BUSY' "$log_path"; then
    printf 'sqlite_busy'
  elif grep -Eiq 'attempt to write a readonly database|readonly database|SQLITE_READONLY' "$log_path"; then
    printf 'sqlite_write_failure'
  elif grep -Eiq 'no such table|no such column|SQLITE_ERROR|error returned from database' "$log_path"; then
    printf 'sqlite_query_failure'
  elif grep -Eiq 'error: could not compile|could not compile|rustc.*error' "$log_path"; then
    printf 'source_build_failure'
  elif grep -Eiq 'configuration|invalid .*environment|is not supported' "$log_path"; then
    printf 'runtime_configuration_failure'
  elif grep -Eiq 'migration|migrat' "$log_path"; then
    printf 'database_migration_failure'
  elif grep -Eiq 'archive.*missing|failed to .*archive|archive.*failed' "$log_path"; then
    printf 'archive_fixture_failure'
  elif grep -Eiq 'summary projection.*hydration|summary.*projection.*failed' "$log_path"; then
    printf 'summary_projection_failure'
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
  local status=""

  status="$(curl --noproxy '*' --silent --show-error --output /dev/null --write-out '%{http_code}' \
    --connect-timeout 1 --max-time 3 "http://127.0.0.1:$port$path" 2>/dev/null || true)"
  if [[ "$status" =~ ^[0-9]{3}$ ]]; then
    printf '%s' "$status"
  else
    printf '000'
  fi
}

(
  cd "$runtime_workspace"
  exec env \
    DATABASE_PATH="$database_path" \
    ARCHIVE_DIR="$archive_dir" \
    HTTP_BIND="127.0.0.1:$port" \
    OPENAI_UPSTREAM_BASE_URL="http://127.0.0.1:9/" \
    PROXY_USAGE_BACKFILL_ON_STARTUP=false \
    PROXY_RAW_DIR=/tmp/codex-vibe-monitor-summary-fixture-raw \
    RETENTION_ENABLED=false \
    XRAY_RUNTIME_DIR=/tmp/codex-vibe-monitor-summary-fixture-xray \
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

(
  cd "$runtime_workspace"
  cargo fmt --all -- --check
  cargo check --locked --all-features
  cargo clippy --locked --all-features -- -D warnings
)
printf 'summary_fixture_quality_gates_passed\n'
