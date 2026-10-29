#!/usr/bin/env bash
set -euo pipefail

# These tests exercise functional and crash-recovery boundaries without a load generator.
filter='test(=tests::lightweight::postgres_migration_engine::optional_postgres_migration_engine_uses_real_source_and_target_fixture) | test(=tests::lightweight::postgres_migration_engine::optional_postgres_migration_target_uses_real_pool_transactions) | test(=tests::lightweight::postgres_migration_engine::optional_postgres_migration_engine_recovers_after_process_sigkill) | test(=tests::lightweight::postgres_migration_engine::migration_source_ack_uncertainty_is_idempotent_after_commit)'
bash .github/scripts/run-backend-tests.sh --profile lightweight --test-filter "$filter"
