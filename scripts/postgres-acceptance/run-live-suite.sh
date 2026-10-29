#!/usr/bin/env bash
set -euo pipefail

: "${BASE_SHA:?BASE_SHA must be set by the acceptance runner}"
: "${CANDIDATE_SHA:?CANDIDATE_SHA must be set by the acceptance runner}"
: "${POSTGRES_ACCEPTANCE_EVIDENCE_DIR:?POSTGRES_ACCEPTANCE_EVIDENCE_DIR must be set by the acceptance runner}"

lightweight_filter='test(=tests::lightweight::postgres_migration_engine::optional_postgres_migration_engine_uses_real_source_and_target_fixture) | test(=tests::lightweight::postgres_migration_engine::optional_postgres_migration_target_uses_real_pool_transactions) | test(=tests::lightweight::postgres_migration_engine::optional_postgres_migration_engine_recovers_after_process_sigkill) | test(=tests::lightweight::postgres_migration_engine::optional_postgres_migration_switch_performance_is_within_budget) | test(=tests::lightweight::postgres_migration_engine::optional_postgres_migration_engine_handles_continuous_load_and_non_empty_switch_tail) | test(=tests::lightweight::postgres_migration_engine::migration_engine_restarts_after_apply_ack_failure_without_loss_or_resurrection) | test(=tests::lightweight::postgres_migration_engine::migration_source_ack_uncertainty_is_idempotent_after_commit) | test(=tests::lightweight::postgres_migration_engine::migration_engine_copies_catches_up_verifies_and_publishes_authority) | test(=tests::lightweight::postgres_migration_engine::switch_catches_up_source_writes_during_large_pre_fence_verification) | test(=tests::lightweight::postgres_migration_engine::migration_switch_keeps_source_fenced_after_indeterminate_publication) | test(=tests::lightweight::postgres_migration_engine::migration_switch_retries_after_recovery_with_next_authority_generation) | test(=tests::lightweight::postgres_migration_engine::migration_switch_abort_does_not_leave_source_fenced) | test(=tests::lightweight::postgres_migration_engine::migration_switch_retries_after_tail_failure_without_generation_drift) | test(=tests::lightweight::postgres_migration_engine::migration_switch_fails_closed_for_pre_fenced_source_authority) | test(=tests::lightweight::postgres_migration_engine::migration_switch_fails_closed_when_source_fence_rollback_fails) | test(=tests::lightweight::storage_migration_status::postgres_cutover_publishes_postgres_authority_and_completed_module) | test(=tests::lightweight::storage_runtime::optional_postgres_storage_runtime_startup_performance_is_within_budget) | test(=tests::lightweight::postgres_tests::archive_native_insert_path_uses_owner_target_columns_when_postgres_is_available) | test(=tests::lightweight::postgres_tests::optional_postgres_storage_transaction_smoke_test)'
archive_filter='test(=tests::archive_file_io::archive_postgres_adapter::postgres_archive_ownership_is_cross_batch_and_release_retry_monotonic) | test(=tests::archive_file_io::archive_postgres_adapter::postgres_archive_release_and_prune_wait_for_live_raw_link_transactions) | test(=tests::archive_file_io::archive_postgres_adapter::postgres_archive_prune_ledger_upgrade_backfills_only_recoverable_paths) | test(=tests::archive_file_io::proxy_runtime_pressure::proxy_runtime_attempt_lifecycle_avoids_sqlite_admission_under_write_pressure) | test(=tests::archive_file_io::proxy_runtime_pressure::proxy_runtime_terminal_commits_survive_retention_and_backfill_contention) | test(=tests::archive_file_io::terminal_journal_recovery::valid_attempt_ack_does_not_create_a_false_interruption_after_isolation) | test(=tests::archive_file_io::shutdown_terminal_recovery::shutdown_recovery_ack_failure_keeps_obligation_without_poisoning_main_journal)'

bash .github/scripts/run-backend-tests.sh \
  --profile lightweight \
  --test-filter "$lightweight_filter"
bash .github/scripts/run-backend-tests.sh \
  --profile archive-file-io \
  --test-filter "$archive_filter"

python3 - "$POSTGRES_ACCEPTANCE_EVIDENCE_DIR/fault-boundaries.json" <<'PY'
import json
import os
import sys
from pathlib import Path

path = Path(sys.argv[1])
payload = {
    "schema_version": 1,
    "acceptance": "postgres-migration-deterministic-fault-boundaries",
    "fixture_version": os.environ.get("POSTGRES_ACCEPTANCE_FIXTURE_VERSION", "4.13.11"),
    "base_sha": os.environ["BASE_SHA"],
    "candidate_sha": os.environ["CANDIDATE_SHA"],
    "observed_status": "passed",
    "boundary_results": {
        "target_transaction_commit_and_local_ack": "migration_engine_restarts_after_apply_ack_failure_without_loss_or_resurrection",
        "source_acknowledgement_after_commit": "migration_source_ack_uncertainty_is_idempotent_after_commit",
        "source_fence": [
            "migration_switch_keeps_source_fenced_after_indeterminate_publication",
            "migration_switch_abort_does_not_leave_source_fenced",
            "migration_switch_fails_closed_for_pre_fenced_source_authority",
            "migration_switch_fails_closed_when_source_fence_rollback_fails",
        ],
        "authority_publication": [
            "migration_switch_keeps_source_fenced_after_indeterminate_publication",
            "migration_switch_retries_after_recovery_with_next_authority_generation",
        ],
        "router_generation": "postgres_cutover_publishes_postgres_authority_and_completed_module",
        "journal_acknowledgement": [
            "valid_attempt_ack_does_not_create_a_false_interruption_after_isolation",
            "shutdown_recovery_ack_failure_keeps_obligation_without_poisoning_main_journal",
        ],
        "normal_timeout_and_recovery": "migration_switch_retries_after_tail_failure_without_generation_drift",
        "real_postgres_process_termination": "optional_postgres_migration_engine_recovers_after_process_sigkill",
        "real_postgres_non_empty_tail": "optional_postgres_migration_engine_handles_continuous_load_and_non_empty_switch_tail",
    },
    "not_covered_boundaries": [
        "filesystem or WAL crash durability beyond the committed PostgreSQL transaction",
        "backup and restore drill",
        "production natural-traffic baseline",
    ],
    "raw_result_paths": [
        str(path.parent / "stdout.txt"),
        str(path.parent / "stderr.txt"),
    ],
    "commands": {
        "lightweight_filter": os.environ.get("PG_ACCEPTANCE_LIGHTWEIGHT_FILTER", "embedded in run-live-suite.sh"),
        "archive_filter": os.environ.get("PG_ACCEPTANCE_ARCHIVE_FILTER", "embedded in run-live-suite.sh"),
    },
}
path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

python3 - "$POSTGRES_ACCEPTANCE_EVIDENCE_DIR/system-headroom.json" <<'PY'
import json
import os
import shutil
import sys
from pathlib import Path

path = Path(sys.argv[1])
continuous = json.loads((path.parent / "continuous-load.json").read_text(encoding="utf-8"))
capture = continuous["capture"]
wal_bytes = float(continuous["postgres_wal_bytes"])
measured_growth_bytes = int(wal_bytes + float(capture["peak_bytes"]))
usage = shutil.disk_usage(path.parent)
required_headroom_bytes = measured_growth_bytes * 2
if usage.free < required_headroom_bytes:
    raise SystemExit(
        f"disk headroom below twice measured target-plus-capture growth: free={usage.free} required={required_headroom_bytes}"
    )
payload = {
    "schema_version": 1,
    "acceptance": "postgres-migration-ci-resource-headroom",
    "fixture_version": os.environ.get("POSTGRES_ACCEPTANCE_FIXTURE_VERSION", "4.13.11"),
    "base_sha": os.environ["BASE_SHA"],
    "candidate_sha": os.environ["CANDIDATE_SHA"],
    "free_bytes_after_run": usage.free,
    "measured_target_plus_capture_growth_bytes": measured_growth_bytes,
    "required_free_bytes": required_headroom_bytes,
    "headroom_threshold_multiplier": 2,
    "headroom_passed": True,
    "cpu_count": os.cpu_count(),
    "load_average": os.getloadavg() if hasattr(os, "getloadavg") else None,
    "raw_result_paths": [str(path.parent / "stdout.txt"), str(path.parent / "stderr.txt")],
}
path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

for evidence_file in process-kill.json startup-performance.json switch-performance.json continuous-load.json runtime-pressure.json runtime-maintenance-pressure.json fault-boundaries.json system-headroom.json; do
  test -s "$POSTGRES_ACCEPTANCE_EVIDENCE_DIR/$evidence_file"
done
