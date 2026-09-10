#!/usr/bin/env bash
set -euo pipefail

copy_path="${SUMMARY_PRODUCTION_COPY:?SUMMARY_PRODUCTION_COPY is required}"
[[ -e "$copy_path" && ! -L "$copy_path" ]] || {
  printf 'project-reason: staged production copy is missing or symbolic\n' >&2
  exit 64
}
[[ "$(readlink -f -- "$copy_path")" == "$copy_path" ]] || {
  printf 'project-reason: staged production copy is not canonical\n' >&2
  exit 64
}
if find -P "$copy_path" \( -type l -o -type b -o -type c -o -type p -o -type s \) -print -quit | grep -q .; then
  printf 'project-reason: staged production copy contains unsupported filesystem entries\n' >&2
  exit 64
fi

copy_bytes="$(du -sb -- "$copy_path" | awk '{print $1}')"
[[ "$copy_bytes" =~ ^[0-9]+$ ]] || {
  printf 'project-reason: staged production copy size is unavailable\n' >&2
  exit 64
}
printf 'production-copy-bytes=%s\n' "$copy_bytes"

target_dir="${CARGO_TARGET_DIR:-/codex-scratch/target}"
  runtime_dir="/codex-scratch/summary-production-runtime"
  # The shared runner mounts the canonical staged copy only beneath
  # /codex-scratch. Never rely on an image-specific /srv/app layout.
  database_path="$copy_path/codex_vibe_monitor.db"
  archive_dir="$copy_path/archives"
  binary_path="$target_dir/debug/codex-vibe-monitor"
  log_path="$runtime_dir/service.log"
  service_pid=""

  cleanup_service() {
    if [[ -n "$service_pid" ]] && kill -0 "$service_pid" 2>/dev/null; then
      kill -TERM "$service_pid" 2>/dev/null || true
      wait "$service_pid" 2>/dev/null || true
    fi
  }
  trap cleanup_service EXIT

  summary_error_class() {
    local response_path="$1"
    if [[ ! -s "$response_path" ]]; then
      printf 'empty_response'
    elif grep -Fq 'summary projection has not completed hydration' "$response_path"; then
      printf 'projection_unhydrated'
    elif grep -Fq 'summary projection archive source is unavailable for the requested range' "$response_path"; then
      printf 'archive_range_unavailable'
    elif grep -Fq 'summary projection account archive source is unavailable for the requested range' "$response_path"; then
      printf 'account_archive_range_unavailable'
    elif grep -Fq 'summary projection persisted-live source is unavailable for the requested range' "$response_path"; then
      printf 'persisted_live_range_unavailable'
    elif grep -Fq 'summary projection persisted-live account source is unavailable for the requested range' "$response_path"; then
      printf 'persisted_live_account_range_unavailable'
    elif grep -Fq 'summary projection current source is unavailable' "$response_path"; then
      printf 'current_source_unavailable'
    elif grep -Fq 'summary delta journal has an unproven change' "$response_path"; then
      printf 'delta_gap_unavailable'
    elif grep -Fq 'summary projection last-good snapshot exceeded the freshness budget' "$response_path"; then
      printf 'stale_projection_unavailable'
    else
      printf 'unclassified_response'
    fi
  }

  summary_bootstrap_stage() {
    local stage
    local selected="none"
    for stage in \
      rollup_load \
      live_exact_admission \
      current_index_admission \
      boundary_manifest_admission \
      archive_account_discovery \
      boundary_manifest_page_planning \
      historical_live_coverage \
      boundary_archive_hydration \
      paged_boundary_archive_hydration \
      current_archive_admission \
      runtime_overlay \
      projection_materialization; do
      if grep -Fq "stage=\"${stage}\"" "$log_path" \
        || grep -Fq "stage=${stage}" "$log_path"; then
        selected="$stage"
      fi
    done
    printf '%s' "$selected"
  }

  summary_bootstrap_failure_class() {
    if grep -Fq 'summary projection build exceeded' "$log_path"; then
      printf 'deadline_exceeded'
    elif grep -Fq 'summary projection startup hydration deferred because a refresh is already in flight' "$log_path"; then
      printf 'refresh_coalesced'
    elif grep -Fq 'summary projection startup hydration failed' "$log_path"; then
      printf 'build_failed'
    else
      printf 'no_failure_log'
    fi
  }

  summary_startup_phase_diagnostics() {
    python3 - "$log_path" <<'PY'
import re
import sys

phase_pattern = re.compile(r'phase="?([a-z0-9_]+)"?')
elapsed_pattern = re.compile(r'elapsed_ms=(\d+)')
phases = []
try:
    with open(sys.argv[1], encoding="utf-8", errors="replace") as log:
        for line in log:
            if "startup phase finished" not in line:
                continue
            phase = phase_pattern.search(line)
            elapsed = elapsed_pattern.search(line)
            if phase is not None and elapsed is not None:
                phases.append(f"{phase.group(1)}:{elapsed.group(1)}")
except OSError:
    pass
print("summary-production-startup-phases=" + ",".join(phases or ["none"]))
PY
  }

  summary_validate_exact_response() {
    local response_path="$1"
    local window="$2"
    local now_epoch="$3"
    python3 /workspace/scripts/summary-production-exact-oracle.py \
      --database "$database_path" \
      --archives "$archive_dir" \
      --response "$response_path" \
      --window "$window" \
      --now "$now_epoch"
  }

  [[ -f "$database_path" && ! -L "$database_path" ]] || {
    printf 'project-reason: staged production copy does not contain a canonical SQLite file\n' >&2
    exit 64
  }
  [[ -d "$archive_dir" && ! -L "$archive_dir" ]] || {
    printf 'project-reason: staged production copy does not contain a canonical archive directory\n' >&2
    exit 64
  }

  # Archive manifests store production-absolute paths. The runner deliberately exposes only
  # the staged copy, so relocate the Summary manifests inside the disposable SQLite copy before
  # starting the service. Every target must resolve beneath the canonical staged archive root.
  python3 - "$database_path" "$archive_dir" <<'PY'
import pathlib
import sqlite3
import sys

database_path = pathlib.Path(sys.argv[1])
archive_root = pathlib.Path(sys.argv[2]).resolve(strict=True)
connection = sqlite3.connect(database_path)
try:
    rows = connection.execute(
        "SELECT id, file_path FROM archive_batches "
        "WHERE dataset = 'codex_invocations' AND status = 'completed'"
    ).fetchall()
    relocated = []
    path_mapping = []
    for archive_batch_id, stored_path in rows:
        parts = pathlib.PurePath(stored_path).parts
        archive_indexes = [index for index, part in enumerate(parts) if part == "archives"]
        if not archive_indexes:
            raise RuntimeError("archive manifest path cannot be mapped into staged archives")
        relative_path = pathlib.PurePath(*parts[archive_indexes[-1] + 1 :])
        candidate = archive_root.joinpath(relative_path)
        resolved = candidate.resolve(strict=True)
        if archive_root not in (resolved, *resolved.parents) or not resolved.is_file():
            raise RuntimeError("archive manifest target is absent from staged archives")
        relocated.append((str(resolved), archive_batch_id))
        path_mapping.append((stored_path, str(resolved)))
    connection.execute("BEGIN IMMEDIATE")
    connection.executemany(
        "UPDATE archive_batches SET file_path = ?1 WHERE id = ?2", relocated
    )
    for table_name in ("hourly_rollup_archive_replay", "hourly_rollup_archive_progress"):
        for old_path, new_path in path_mapping:
            connection.execute(
                f"UPDATE {table_name} SET file_path = ?1 WHERE file_path = ?2",
                (new_path, old_path),
            )
    connection.commit()
except Exception:
    connection.rollback()
    raise
finally:
    connection.close()

print(f"summary-production-archive-remap=verified={len(rows)}")
PY

  mkdir -p "$runtime_dir/proxy-raw" "$runtime_dir/xray"
  export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-1.96.0-x86_64-unknown-linux-gnu}"
  cargo build --locked --manifest-path /workspace/Cargo.toml --target-dir "$target_dir"

  DATABASE_PATH="$database_path" \
  ARCHIVE_DIR="$archive_dir" \
  PROXY_RAW_DIR="$runtime_dir/proxy-raw" \
  XRAY_RUNTIME_DIR="$runtime_dir/xray" \
  XRAY_BINARY=/bin/false \
  HTTP_BIND=127.0.0.1:18080 \
  RETENTION_ENABLED=false \
  POLL_INTERVAL_SECS=86400 \
  UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS=86400 \
  OPENAI_UPSTREAM_BASE_URL=http://127.0.0.1:9 \
  UPSTREAM_ACCOUNTS_OAUTH_ISSUER=http://127.0.0.1:9 \
  UPSTREAM_ACCOUNTS_USAGE_BASE_URL=http://127.0.0.1:9 \
  RUST_LOG=warn,codex_vibe_monitor::runtime=info,codex_vibe_monitor::api::slices::invocations_and_summary=info \
  "$binary_path" >"$log_path" 2>&1 &
  service_pid="$!"

  started_at="$SECONDS"
  health_status=000
  health_acceptance_deadline_secs=30
  health_observation_deadline_secs=45
  while (( SECONDS - started_at < health_observation_deadline_secs )); do
    health_status="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 2 http://127.0.0.1:18080/health || true)"
    [[ "$health_status" == 200 ]] && break
    sleep 1
  done
  health_elapsed_secs="$((SECONDS - started_at))"
  if [[ "$health_status" != 200 || "$health_elapsed_secs" -gt "$health_acceptance_deadline_secs" ]]; then
    process_state=running
    if ! kill -0 "$service_pid" 2>/dev/null; then
      process_state=exited
    fi
    printf 'summary-production-health=status=%s elapsed_secs=%s acceptance_deadline_secs=%s process_state=%s\n' \
      "$health_status" \
      "$health_elapsed_secs" \
      "$health_acceptance_deadline_secs" \
      "$process_state" >&2
    summary_startup_phase_diagnostics >&2
    exit 1
  fi

  recent_windows=(current 1d 7d today)
  recent_statuses=()
  recent_oracle_nows=()
  recent_ready_deadline_secs="${SUMMARY_PRODUCTION_RECENT_READY_DEADLINE_SECS:-30}"
  [[ "$recent_ready_deadline_secs" =~ ^[1-9][0-9]*$ ]] || {
    printf 'project-reason: recent Summary readiness deadline must be a positive integer\n' >&2
    exit 64
  }
  recent_deadline=$((SECONDS + recent_ready_deadline_secs))
  while :; do
    recent_ready=true
    recent_statuses=()
    recent_error_classes=()
    recent_oracle_nows=()
    for window in "${recent_windows[@]}"; do
      response_path="$runtime_dir/summary-${window}.response"
      oracle_now="$(date -u +%s)"
      status="$(curl -sS -o "$response_path" -w '%{http_code}' --max-time 2 \
        "http://127.0.0.1:18080/api/stats/summary?window=${window}&limit=50&timeZone=Asia%2FShanghai" || true)"
      recent_statuses+=("$status")
      recent_oracle_nows+=("$oracle_now")
      if [[ "$status" == 200 ]]; then
        recent_error_classes+=("ok")
      else
        recent_error_classes+=("$(summary_error_class "$response_path")")
        recent_ready=false
      fi
    done
    "$recent_ready" && break
    (( SECONDS >= recent_deadline )) && break
    sleep 1
  done
  for index in "${!recent_windows[@]}"; do
    printf 'summary-production-window=%s status=%s reason=%s elapsed_secs=%s\n' \
      "${recent_windows[$index]}" "${recent_statuses[$index]}" \
      "${recent_error_classes[$index]}" "$((SECONDS - started_at))"
    if [[ "${recent_statuses[$index]}" == 200 ]]; then
      summary_validate_exact_response \
        "$runtime_dir/summary-${recent_windows[$index]}.response" \
        "${recent_windows[$index]}" \
        "${recent_oracle_nows[$index]}"
    fi
  done
  if [[ "${recent_error_classes[*]}" == *projection_unhydrated* ]]; then
    printf 'summary-production-bootstrap=reason=%s stage=%s\n' \
      "$(summary_bootstrap_failure_class)" \
      "$(summary_bootstrap_stage)"
  fi
  historical_status=000
  historical_error_class=projection_unhydrated
  historical_oracle_now=0
  historical_ready_deadline_secs="${SUMMARY_PRODUCTION_HISTORICAL_READY_DEADLINE_SECS:-1800}"
  [[ "$historical_ready_deadline_secs" =~ ^[1-9][0-9]*$ ]] || {
    printf 'project-reason: historical Summary readiness deadline must be a positive integer\n' >&2
    exit 64
  }
  historical_deadline=$((SECONDS + historical_ready_deadline_secs))
  while :; do
    historical_response_path="$runtime_dir/summary-30d.response"
    historical_oracle_now="$(date -u +%s)"
    historical_status="$(curl -sS -o "$historical_response_path" -w '%{http_code}' --max-time 2 \
      'http://127.0.0.1:18080/api/stats/summary?window=30d&limit=50&timeZone=Asia%2FShanghai' || true)"
    if [[ "$historical_status" == 200 ]]; then
      historical_error_class=ok
      break
    fi
    historical_error_class="$(summary_error_class "$historical_response_path")"
    (( SECONDS >= historical_deadline )) && break
    sleep 1
  done
  printf 'summary-production-window=30d status=%s reason=%s elapsed_secs=%s\n' \
    "$historical_status" "$historical_error_class" "$((SECONDS - started_at))"
  [[ "$historical_status" == 200 ]] && summary_validate_exact_response \
    "$historical_response_path" 30d "$historical_oracle_now"
  recovery_diagnostics() {
    python3 - "$log_path" "$database_path" <<'PY'
import re
import sqlite3
import sys

stages = {
    "historical_coverage_snapshot_backfill": [],
    "historical_coverage_overlay_publication": [],
    "coverage_overlay_publication": [],
    "historical_coverage_overlay_deferred": [],
}
ansi_pattern = re.compile(r'\x1b\[[0-?]*[ -/]*[@-~]')
stage_pattern = re.compile(r'stage[= ]+"?([a-z0-9_]+)"?')
elapsed_pattern = re.compile(r'elapsed_ms=(\d+)')
recent_candidates_pattern = re.compile(r'recent_candidate_count=(\d+)')
recent_candidates = []
overlay_counts = None
overlay_count_pattern = re.compile(
    r'global_bucket_count=(\d+).*account_bucket_count=(\d+).*'
    r'boundary_record_count=(\d+).*unavailable_unmaterialized_range_count=(\d+).*'
    r'unavailable_boundary_range_count=(\d+)'
)
try:
    lines = open(sys.argv[1], encoding="utf-8", errors="replace")
    for line in lines:
        line = ansi_pattern.sub("", line)
        stage = stage_pattern.search(line)
        if stage is None or stage.group(1) not in stages:
            continue
        elapsed = elapsed_pattern.search(line)
        if elapsed is not None:
            stages[stage.group(1)].append(int(elapsed.group(1)))
        recent_candidates_match = recent_candidates_pattern.search(line)
        if stage.group(1) == "historical_coverage_snapshot_backfill" and recent_candidates_match:
            recent_candidates.append(int(recent_candidates_match.group(1)))
        if stage.group(1) in {
            "coverage_overlay_publication",
            "historical_coverage_overlay_publication",
        }:
            counts = overlay_count_pattern.search(line)
            if counts is not None:
                overlay_counts = tuple(int(value) for value in counts.groups())
except OSError:
    pass

def report(name):
    values = stages[name]
    return (len(values), sum(values), max(values, default=0))

backfill = report("historical_coverage_snapshot_backfill")
published = report("historical_coverage_overlay_publication")
published_internal = report("coverage_overlay_publication")
deferred = report("historical_coverage_overlay_deferred")
print(
    "summary-production-recovery-telemetry="
    f"backfill_runs={backfill[0]} backfill_total_ms={backfill[1]} backfill_max_ms={backfill[2]} "
    f"backfill_recent_candidates_max={max(recent_candidates, default=0)} "
    f"overlay_published_runs={published[0] + published_internal[0]} "
    f"overlay_published_total_ms={published[1] + published_internal[1]} "
    f"overlay_deferred_runs={deferred[0]} overlay_deferred_total_ms={deferred[1]}"
)
if overlay_counts is not None:
    print(
        "summary-production-overlay="
        f"global_buckets={overlay_counts[0]} account_buckets={overlay_counts[1]} "
        f"boundary_records={overlay_counts[2]} "
        f"unavailable_unmaterialized_ranges={overlay_counts[3]} "
        f"unavailable_boundary_ranges={overlay_counts[4]}"
    )
connection = sqlite3.connect(sys.argv[2])
tables = {
    row[0]
    for row in connection.execute("SELECT name FROM sqlite_master WHERE type = 'table'")
}
proofs = 0
outcomes = 0
obligations = 0
if "summary_archive_snapshot_v2_proof" in tables:
    proofs = connection.execute(
        "SELECT COUNT(*) FROM summary_archive_snapshot_v2_proof"
    ).fetchone()[0]
if "summary_archive_snapshot_backfill_outcome" in tables:
    outcomes = connection.execute(
        "SELECT COUNT(*) FROM summary_archive_snapshot_backfill_outcome"
    ).fetchone()[0]
if "summary_coverage_obligation" in tables:
    obligations = connection.execute(
        "SELECT COUNT(*) FROM summary_coverage_obligation WHERE state <> 'resolved'"
    ).fetchone()[0]
print(
    "summary-production-recovery="
    f"v2_proofs={proofs} outcomes={outcomes} unresolved_obligations={obligations}"
)
PY
  }
  "$recent_ready" || exit 1
  [[ "$historical_status" == 200 ]] || {
    printf 'project-reason: historical Summary coverage did not become exact-ready\n' >&2
    exit 1
  }

  all_status=000
  all_response_path="$runtime_dir/summary-all.response"
  all_oracle_now=0
  all_deadline=$((SECONDS + 1800))
  while :; do
    all_oracle_now="$(date -u +%s)"
    all_status="$(curl -sS -o "$all_response_path" -w '%{http_code}' --max-time 2 \
      'http://127.0.0.1:18080/api/stats/summary?window=all&limit=50&timeZone=Asia%2FShanghai' || true)"
    [[ "$all_status" == 200 ]] && break
    (( SECONDS >= all_deadline )) && break
    sleep 1
  done
  printf 'summary-production-window=all status=%s elapsed_secs=%s\n' \
    "$all_status" "$((SECONDS - started_at))"
  [[ "$all_status" == 200 ]] && summary_validate_exact_response \
    "$all_response_path" all "$all_oracle_now"
  if [[ "${SUMMARY_PRODUCTION_RECOVERY_DIAGNOSTICS:-}" == "1" ]]; then
    recovery_diagnostics
  fi
  [[ "$all_status" == 200 ]]
printf 'summary-production-validation=passed\n'
