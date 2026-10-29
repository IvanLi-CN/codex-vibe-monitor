#!/usr/bin/env bash
set -euo pipefail

: "${BASE_SHA:?BASE_SHA must be set by the acceptance runner}"
: "${CANDIDATE_SHA:?CANDIDATE_SHA must be set by the acceptance runner}"
: "${POSTGRES_ACCEPTANCE_EVIDENCE_DIR:?POSTGRES_ACCEPTANCE_EVIDENCE_DIR must be set by the acceptance runner}"

lightweight_filter='test(=tests::lightweight::postgres_migration_engine::optional_postgres_migration_engine_uses_real_source_and_target_fixture) | test(=tests::lightweight::postgres_migration_engine::optional_postgres_migration_target_uses_real_pool_transactions) | test(=tests::lightweight::postgres_migration_engine::optional_postgres_migration_engine_recovers_after_process_sigkill) | test(=tests::lightweight::postgres_migration_engine::optional_postgres_migration_switch_performance_is_within_budget) | test(=tests::lightweight::storage_runtime::optional_postgres_storage_runtime_startup_performance_is_within_budget) | test(=tests::lightweight::postgres_tests::archive_native_insert_path_uses_owner_target_columns_when_postgres_is_available) | test(=tests::lightweight::postgres_tests::optional_postgres_storage_transaction_smoke_test)'
archive_filter='test(=tests::archive_file_io::archive_postgres_adapter::postgres_archive_ownership_is_cross_batch_and_release_retry_monotonic) | test(=tests::archive_file_io::archive_postgres_adapter::postgres_archive_release_and_prune_wait_for_live_raw_link_transactions) | test(=tests::archive_file_io::archive_postgres_adapter::postgres_archive_prune_ledger_upgrade_backfills_only_recoverable_paths)'

bash .github/scripts/run-backend-tests.sh \
  --profile lightweight \
  --test-filter "$lightweight_filter"
bash .github/scripts/run-backend-tests.sh \
  --profile archive-file-io \
  --test-filter "$archive_filter"

for evidence_file in process-kill.json startup-performance.json switch-performance.json; do
  test -s "$POSTGRES_ACCEPTANCE_EVIDENCE_DIR/$evidence_file"
done
