#!/usr/bin/env bash
set -euo pipefail

# This script is intentionally data-blind: it reads only an isolated production-data copy and
# retains normalized response hashes, never raw records or response bodies, in its receipt.
fixture_dir="${SUMMARY_PRODUCTION_FIXTURE_DIR:-/workspace/production-fixture}"
database_path="$fixture_dir/codex_vibe_monitor.db"
archive_dir="$fixture_dir/archives"
runtime_workspace="${SUMMARY_PRODUCTION_FIXTURE_WORKSPACE:-/tmp/codex-vibe-monitor-summary-fixture-workspace}"
port="${SUMMARY_PRODUCTION_FIXTURE_PORT:-18080}"
bootstrap_timeout_seconds="${SUMMARY_PRODUCTION_FIXTURE_BOOTSTRAP_TIMEOUT_SECS:-30}"
timeout_seconds="${SUMMARY_PRODUCTION_FIXTURE_TIMEOUT_SECS:-1800}"
fixture_wait_seconds="${SUMMARY_PRODUCTION_FIXTURE_WAIT_SECS:-4800}"
cargo_target_dir="${SUMMARY_PRODUCTION_FIXTURE_CARGO_TARGET_DIR:-$runtime_workspace/target}"
receipt_path="${SUMMARY_PRODUCTION_RECEIPT_PATH:-$runtime_workspace/production-copy-receipt.json}"
target_sha="${TARGET_SHA:-}"
backend_test_image_digest="${BACKEND_TEST_IMAGE_DIGEST:-}"
fixture_contract_version="${FIXTURE_CONTRACT_VERSION:-summary-representative-scale-v1}"
oracle_version="${ORACLE_VERSION:-summary-oracle-v1}"
expected_oracle_sha256="${SUMMARY_PRODUCTION_FIXTURE_ORACLE_SHA256:-}"
app_log="$runtime_workspace/.summary-projection-validation.log"
summary_response_dir="$runtime_workspace/.summary-responses"
app_pid=""

if [[ ! "$port" =~ ^[0-9]{2,5}$ || ! "$bootstrap_timeout_seconds" =~ ^[0-9]+$ ||
  ! "$timeout_seconds" =~ ^[0-9]+$ ||
  ! "$fixture_wait_seconds" =~ ^[0-9]+$ || "$runtime_workspace" != /tmp/* ||
  "$runtime_workspace" == *..* || "$cargo_target_dir" != "$runtime_workspace"/* ||
  "$cargo_target_dir" == *..* || "$receipt_path" == *..* ||
  ! "$target_sha" =~ ^[0-9a-f]{40}$ ||
  ! "$backend_test_image_digest" =~ ^sha256:[0-9a-f]{64}$ ||
  ! "$expected_oracle_sha256" =~ ^[0-9a-f]{64}$ ||
  -z "$fixture_contract_version" || -z "$oracle_version" ]]; then
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
if ! mkdir -p "$runtime_workspace" "$cargo_target_dir" "$summary_response_dir" ||
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
  rm -rf "$summary_response_dir"
}
trap cleanup EXIT HUP INT TERM

write_receipt() {
  local result="$1"
  local bootstrap_elapsed="$2"
  local all_time_elapsed="$3"
  local oracle_sha256="$4"
  python3 - "$receipt_path" "$result" "$bootstrap_elapsed" "$all_time_elapsed" "$oracle_sha256" \
    "$target_sha" "$backend_test_image_digest" "$fixture_contract_version" "$oracle_version" <<'PY'
from __future__ import annotations

import json
import sys
from pathlib import Path

path, result, bootstrap, all_time, oracle, target, image, fixture, oracle_version = sys.argv[1:]
payload = {
    "schema_version": 1,
    "source": "production-copy",
    "target_sha": target,
    "backend_test_image_digest": image,
    "fixture_contract_version": fixture,
    "oracle_version": oracle_version,
    "bootstrap_deadline_seconds": 30,
    "all_time_deadline_seconds": 1800,
    "bootstrap_elapsed_seconds": int(bootstrap) if bootstrap else None,
    "all_time_elapsed_seconds": int(all_time) if all_time else None,
    "oracle_sha256": oracle or None,
    "result": result,
}
path_obj = Path(path)
path_obj.parent.mkdir(parents=True, exist_ok=True)
path_obj.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

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
  elif grep -Eiq 'no such table' "$log_path"; then
    printf 'sqlite_missing_table'
  elif grep -Eiq 'no such column' "$log_path"; then
    printf 'sqlite_missing_column'
  elif grep -Eiq 'constraint failed' "$log_path"; then
    printf 'sqlite_constraint_failure'
  elif grep -Eiq 'SQLITE_ERROR|error returned from database' "$log_path"; then
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
  local signature_word=""
  local normalized=""
  local sqlite_code=""
  local -a signature=()

  [[ -f "$log_path" ]] || {
    printf 'missing_log'
    return
  }
  line="$(grep -Eia 'error returned from database: \(code: [0-9]+\)' "$log_path" | tail -n1 || true)"
  if [[ "$line" =~ \(code:[[:space:]]([0-9]+)\) ]]; then
    sqlite_code="${BASH_REMATCH[1]}"
    if [[ "$line" =~ trigger[^A-Za-z_]*([A-Za-z_][A-Za-z0-9_]*) ]]; then
      case "${BASH_REMATCH[1]}" in
        failed|failure|error)
          ;;
        *)
          printf 'sqlite_error_code=%s trigger=%s' "$sqlite_code" "${BASH_REMATCH[1]}"
          return
          ;;
      esac
    fi
    if [[ "$line" =~ near[[:space:]]\"([A-Za-z_][A-Za-z0-9_]*)\" ]]; then
      printf 'sqlite_error_code=%s near=%s' "$sqlite_code" "${BASH_REMATCH[1]}"
      return
    fi
    case "$line" in
      *'cannot start a transaction within a transaction'*)
        printf 'sqlite_error_code=%s nested_transaction' "$sqlite_code"
        return
        ;;
      *'cannot commit - no transaction is active'*)
        printf 'sqlite_error_code=%s missing_transaction' "$sqlite_code"
        return
        ;;
      *'unsafe use of'*)
        printf 'sqlite_error_code=%s unsafe_sqlite_feature' "$sqlite_code"
        return
        ;;
      *'database schema is locked'*)
        printf 'sqlite_error_code=%s schema_locked' "$sqlite_code"
        return
        ;;
    esac
    for signature_word in $line; do
      normalized="${signature_word,,}"
      normalized="${normalized//[^a-z]/}"
      case "$normalized" in
        error|returned|from|database|code|sqlite|sql|syntax|near|table|column|index|trigger|view|function|schema|malformed|missing|ambiguous|qualified|identifier|expression|query|select|insert|update|delete|create|drop|alter|pragma|transaction|begin|commit|rollback|constraint|foreign|key|unique|not|null|check|unsafe|use|virtual|json|jsonb|locked|busy|readonly|open|unable|cannot|attempt|file|disk|image|corrupt|permission|denied|row|rows|failed)
          signature+=("$normalized")
          ;;
      esac
      (( ${#signature[@]} >= 16 )) && break
    done
    if (( ${#signature[@]} > 0 )); then
      printf 'sqlite_error_code=%s phrase=%s' "$sqlite_code" "${signature[*]}"
    else
      printf 'sqlite_error_code=%s' "$sqlite_code"
    fi
    return
  fi
  line="$(grep -Eim1 'no such table:[[:space:]]+[A-Za-z_][A-Za-z0-9_]*' "$log_path" || true)"
  if [[ "$line" =~ no[[:space:]]such[[:space:]]table:[[:space:]]([A-Za-z_][A-Za-z0-9_]*) ]]; then
    printf 'sqlite_missing_table=%s' "${BASH_REMATCH[1]}"
    return
  fi
  line="$(grep -Eim1 'no such column:[[:space:]]+[A-Za-z_][A-Za-z0-9_]*' "$log_path" || true)"
  if [[ "$line" =~ no[[:space:]]such[[:space:]]column:[[:space:]]([A-Za-z_][A-Za-z0-9_]*) ]]; then
    printf 'sqlite_missing_column=%s' "${BASH_REMATCH[1]}"
    return
  fi
  if grep -Eiq 'constraint failed' "$log_path"; then
    printf 'sqlite_constraint_failure'
    return
  fi
  line="$(grep -Eim1 '(^error:|^Error:|failed|permission denied|cannot|could not|panic)' "$log_path" || true)"
  [[ -n "$line" ]] || {
    printf 'no_classified_error'
    return
  }
  for signature_word in $line; do
    normalized="${signature_word,,}"
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

last_startup_phase() {
  local log_path="$1"
  local phase="none"
  local candidate=""

  [[ -f "$log_path" ]] || {
    printf 'none'
    return
  }
  for candidate in db_connect schema runtime_init http_ready; do
    if grep -Eiq "phase=[\"']?${candidate}[\"']?" "$log_path"; then
      phase="$candidate"
    fi
  done
  printf '%s' "$phase"
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

canonical_response_hash() {
  python3 - "$summary_response_dir" <<'PY'
from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

root = Path(sys.argv[1])
hasher = hashlib.sha256()
for path in sorted(root.glob("*.json")):
    payload = json.loads(path.read_text(encoding="utf-8"))
    canonical = json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
    hasher.update(path.stem.encode("utf-8"))
    hasher.update(b"\0")
    hasher.update(canonical.encode("utf-8"))
    hasher.update(b"\n")
print(hasher.hexdigest())
PY
}

capture_summary_response() {
  local label="$1"
  local path="$2"
  local output="$summary_response_dir/${label}.json"
  local status=""
  status="$(curl --noproxy '*' --silent --show-error --output "$output" --write-out '%{http_code}' \
    --connect-timeout 1 --max-time 10 "http://127.0.0.1:$port$path" 2>/dev/null || true)"
  [[ "$status" == '200' && -s "$output" ]]
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
  'current&limit=50&timeZone=Asia%2FShanghai'
  '1d&limit=50&timeZone=Asia%2FShanghai'
  '7d&limit=50&timeZone=Asia%2FShanghai'
  '30d&limit=50&timeZone=Asia%2FShanghai'
  'today&limit=50&timeZone=Asia%2FShanghai'
  'all&limit=50&timeZone=Asia%2FShanghai'
)
summary_labels=(current one_day seven_days thirty_days today all_time)
started_at="$(date +%s)"
bootstrap_elapsed_seconds=""
all_time_elapsed_seconds=""
bootstrap_ready=false
oracle_sha256=""

while :; do
  now="$(date +%s)"
  elapsed_seconds="$((now - started_at))"
  if ! kill -0 "$app_pid" 2>/dev/null; then
    reason="$(classify_app_exit "$app_log")"
    signature="$(safe_failure_signature "$app_log")"
    phase="$(last_startup_phase "$app_log")"
    printf 'summary_fixture_process_exited elapsed_s=%s phase=%s reason=%s signature=%s\n' \
      "$elapsed_seconds" "$phase" "$reason" "$signature" >&2
    write_receipt "environment_failure" "$bootstrap_elapsed_seconds" "$all_time_elapsed_seconds" ""
    exit 1
  fi

  health_status="$(http_status '/health')"
  rolling_ready=true
  all_ready=true
  statuses=()
  for index in "${!summary_paths[@]}"; do
    status="$(http_status "/api/stats/summary?window=${summary_paths[$index]}")"
    statuses+=("${summary_labels[$index]}=$status")
    [[ "$status" == '200' ]] || all_ready=false
    if (( index < 5 )) && [[ "$status" != '200' ]]; then
      rolling_ready=false
    fi
  done

  if [[ "$health_status" == '200' && "$rolling_ready" == true && "$bootstrap_ready" == false ]]; then
    bootstrap_ready=true
    bootstrap_elapsed_seconds="$elapsed_seconds"
    printf 'summary_fixture_bootstrap_ready elapsed_s=%s health=%s %s\n' \
      "$elapsed_seconds" "$health_status" "${statuses[*]}"
  fi
  if [[ "$health_status" == '200' && "$all_ready" == true ]]; then
    all_time_elapsed_seconds="$elapsed_seconds"
    for index in "${!summary_paths[@]}"; do
      capture_summary_response "${summary_labels[$index]}" \
        "/api/stats/summary?window=${summary_paths[$index]}" || all_ready=false
    done
    if [[ "$all_ready" == true ]]; then
      oracle_sha256="$(canonical_response_hash)"
      if [[ "$oracle_sha256" != "$expected_oracle_sha256" ]]; then
        printf 'summary_fixture_oracle_mismatch elapsed_s=%s\n' "$elapsed_seconds" >&2
        write_receipt "oracle_mismatch" "$bootstrap_elapsed_seconds" "$all_time_elapsed_seconds" "$oracle_sha256"
        exit 1
      fi
      printf 'summary_fixture_ready elapsed_s=%s health=%s %s\n' \
        "$elapsed_seconds" "$health_status" "${statuses[*]}"
      break
    fi
  fi
  if (( elapsed_seconds % 30 == 0 )); then
    printf 'summary_fixture_wait elapsed_s=%s health=%s %s\n' \
      "$elapsed_seconds" "$health_status" "${statuses[*]}"
  fi
  if [[ "$bootstrap_ready" == false && elapsed_seconds -ge bootstrap_timeout_seconds ]]; then
    printf 'summary_fixture_bootstrap_timeout elapsed_s=%s health=%s %s\n' \
      "$elapsed_seconds" "$health_status" "${statuses[*]}" >&2
    write_receipt "timeout" "" "" ""
    exit 1
  fi
  if (( elapsed_seconds >= timeout_seconds )); then
    printf 'summary_fixture_timeout elapsed_s=%s health=%s %s\n' \
      "$elapsed_seconds" "$health_status" "${statuses[*]}" >&2
    write_receipt "timeout" "$bootstrap_elapsed_seconds" "" ""
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
write_receipt "pass" "$bootstrap_elapsed_seconds" "$all_time_elapsed_seconds" "$oracle_sha256"
printf 'summary_fixture_quality_gates_passed\n'
