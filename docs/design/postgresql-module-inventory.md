# PostgreSQL Module Inventory

This is the pg01 storage contract for the existing SQLite database. It records the
ownership closure that must move together when a module changes authority. It does
not implement copying, change capture, verification, cutover, or production
deployment.

## Inventory source

`src/storage/core/catalog.rs` is the executable table inventory. It classifies every
non-internal table returned by `sqlite_master`, records its transaction group, and
checks every index and trigger against a classified table. Trigger bodies are
inspected for write targets, so a transaction closure includes trigger-maintained
tables rather than only the statement's explicit tables. It also reads every
SQLite foreign key and rejects a cross-module edge. A transaction boundary must
pass all tables through `ModuleCatalog::validate_transaction`; unknown ownership,
mixed module ownership, and cross-module trigger closure are errors.

The main application SQLite database and the separate maintenance SQLite database
are inspected as independent catalogs. Their table namespaces are not merged, so
maintenance ownership cannot be hidden by a similarly named main-database table.

The catalog deliberately fails closed. Adding a production table requires adding
its owner and transaction group before a migration can inspect the schema.

## Module ownership

### Traffic and archive

Transaction groups are `traffic_fact`, `archive_publication`, and
`traffic_recovery_obligation`.

Traffic facts and live state:

`codex_invocations`, `invocation_in_progress_live`, `prompt_cache_working_set_live`,
`pool_upstream_request_attempts`, `forward_proxy_attempts`,
`forward_proxy_metadata_history`, `proxy_raw_payload_blobs`,
`proxy_raw_payload_blob_links`, `proxy_raw_payload_blob_link_migrations`,
`codex_invocation_raw_codec_migrations`, and `codex_quota_snapshots`.

Archive publication and repair:

`archive_batches`, `retention_prepared_archives`, `retention_recovery_cursors`,
`retention_raw_reconciliation`, `archive_batch_upstream_activity`,
`hourly_rollup_materialized_buckets`, `hourly_rollup_archive_replay`,
`pool_upstream_node_health_archive`, and
`pool_upstream_node_health_hourly_archive`.

Source-change, coverage, and recovery obligations:

`summary_source_change_journal`, `summary_source_change_cursor`,
`summary_source_change_compaction_proof`, `summary_coverage_obligation`,
`summary_coverage_revision`, `summary_account_coverage_revision`,
`summary_all_time_coverage_checkpoint`, `summary_live_tail_reconciliation_checkpoint`,
`summary_archive_snapshot`, `summary_archive_snapshot_v2_proof`,
`summary_archive_snapshot_backfill_checkpoint`,
`summary_archive_snapshot_backfill_outcome`, `hourly_rollup_archive_progress`,
`hourly_rollup_archive_repair_progress`, `hourly_rollup_archive_repair_deferred`,
`hourly_rollup_live_progress`, `hourly_rollup_repair_seen_invocation_ids`,
`hourly_rollup_repair_cleared_buckets`, and
`account_activity_v2_bucket_repair_watermarks`.

### Analytics

Analytics owns rebuildable projections and their consumer checkpoints:

`invocation_rollup_daily`, `invocation_rollup_hourly`,
`invocation_failure_rollup_hourly`, `proxy_perf_stage_hourly`,
`prompt_cache_rollup_hourly`, `prompt_cache_upstream_account_hourly`,
`parallel_work_minute_key_rollup`,
`parallel_work_upstream_account_minute_key_rollup`,
`parallel_work_hourly_rollup`, `parallel_work_upstream_account_hourly_rollup`,
`parallel_work_hourly_coverage`, `parallel_work_rollup_coverage_state`,
`parallel_work_rollup_maintenance_state`, `timeseries_minute_projection_records`,
`timeseries_minute_projection_v2`, `timeseries_minute_projection_v2_state`,
`timeseries_minute_projection_v2_recovery`, `upstream_account_usage_hourly`,
`upstream_account_usage_breakdown_hourly`, `upstream_account_stats_hourly`,
`upstream_account_stats_minute`, `upstream_host_network_minute`,
`upstream_socket_network_minute`, `upstream_sticky_key_hourly`,
`forward_proxy_attempt_hourly`, `forward_proxy_weight_hourly`,
`dashboard_activity_excluded_invocation_ids`, `hourly_invoke_prefixes`,
`prompt_cache_conversations`, `prompt_cache_conversation_stats_refresh_queue`,
`prompt_cache_conversation_stats_generation_clock`,
`prompt_cache_conversation_stats_refresh_staging`,
`prompt_cache_conversation_migration_progress`,
`prompt_cache_conversation_materialization_runs`,
`prompt_cache_conversation_orphan_cleanup_state`,
`invocation_timeline_snapshot_rows`, `long_term_projection_archive_compatibility`,
`long_term_projection_bucket_state`, `long_term_projection_daily_backup_claims`,
`long_term_projection_daily_backups`, `long_term_projection_date_publications`,
`long_term_projection_dirty_buckets`, `long_term_projection_interval_state`,
`long_term_projection_interval_suppressions`, `long_term_projection_intervals`,
`long_term_projection_rebuild_members`, `long_term_projection_state`,
`long_term_stats_repair_queue`, `long_term_stats_state`, `long_term_usage_daily`,
`long_term_usage_hourly`, `retention_backlog_hourly_observations`,
`summary_all_time_projection_checkpoint`,
`summary_all_time_projection_account_checkpoint`,
`summary_all_time_projection_usage_checkpoint`,
`summary_compacted_source_recovery_checkpoint`,
`summary_compacted_source_recovery_aggregate`,
`summary_compacted_source_recovery_usage`, and
`summary_compacted_source_recovery_record`.

### Account and routing controls

Account and routing controls remain SQLite-authoritative until a later module
decision. Their inventory is:

`external_api_keys`, `managed_model_catalog_migrations`,
`managed_model_suppressions`, `managed_models`,
`models_dev_sync_discovered_models`, `models_dev_sync_metadata`,
`models_dev_sync_model_selections`, `models_dev_sync_provider_selections`,
`models_dev_sync_quote_provider_choices`, `oauth_bridge_settings`,
`pool_oauth_login_sessions`, `pool_oauth_mailbox_sessions`,
`pool_routing_settings`, `pool_sticky_model_route_generations`,
`pool_sticky_model_routes`, `pool_sticky_route_generations`,
`pool_sticky_routes`, `pool_tags`, `pool_upstream_account_egress_throttle`,
`pool_upstream_account_events`, `pool_upstream_account_group_notes`,
`pool_upstream_account_limit_samples`, `pool_upstream_account_model_catalogs`,
`pool_upstream_account_model_routes`, `pool_upstream_account_tags`,
`pool_upstream_accounts`, `prompt_cache_conversation_bindings`,
`prompt_cache_conversation_operation_events`,
`prompt_cache_encrypted_session_owners`, `proxy_model_settings`,
`pricing_settings_meta`, `pricing_settings_models`, `pricing_model_quotes`,
`forward_proxy_settings`, and `forward_proxy_runtime`.

### Task operations and performance

Task operations own `schema_refresh_migrations`, `startup_backfill_progress`,
`system_task_runs`, `maintenance_metadata`, `maintenance_owner_retry_state`,
`managed_task_progress`, `managed_task_runs`, `managed_task_work_runs`,
`managed_tasks`, `task_timeline_coverage`, and `task_timeline_segments`.

Performance owns `system_raw_payload_inventory_paths` and
`system_raw_payload_metrics`. These tables are not part of the initial traffic
authority switch.

## Trigger and writer closure

The catalog assigns every SQLite trigger to the owner of its source table and
records each table written by its body. The current trigger families that must be
included in a traffic migration are:

- `schema/runtime_system.rs`: invocation, attempt, and Raw-link maintenance;
- `schema/invocation_live.rs`: live invocation identity and in-progress state;
- `schema/prompt_cache_working_set_triggers.rs`: prompt-cache working-set state;
- `schema/summary_coverage.rs`: summary proof invalidation and coverage revisions;
- `schema/rollups_statistics.rs`: projection replacement and terminal-row guards;
- `long_term_stats/schema.rs`: long-term projection correction and archive publication.

The current schema also exposes cross-module trigger obligations that remain
explicit migration blockers until a later adapter refactors them into durable
module-owned obligations: invocation terminal updates write
`timeseries_minute_projection_v2_recovery`, and archive/account changes write
`long_term_projection_dirty_buckets`. The executable catalog reports these edges
and rejects their mixed transaction closure; it does not silently assign one
shared target table to multiple authorities.

The main transaction writers are the SQLite batch writer and terminal materializer
(`src/sqlite_batch_writer/flush.rs`), proxy attempt persistence, archive and
retention batches (`src/maintenance/retention/task_batches.rs`), archive writers,
hourly-rollup repair/materialization, summary coverage repair, prompt-cache
materialization, and account/routing control writers. Shutdown and journal
recovery writers are included in the same generation boundary; they cannot bypass
the module lease because a scheduler was paused.

The inventory is intentionally based on runtime schema inspection rather than
only on source string search. Tests create the current schema and verify that
tables, indexes, triggers, and foreign keys all resolve to a closed catalog.

## Recovery and authority contract

`ModuleGenerationGate` is the in-process admission fence. A lease captures one
module, backend, and generation. `StoragePools::begin_typed` binds a transaction
to a module marker, so a transaction cannot silently switch backends between
statements. Mixed module, backend, or generation leases are rejected.

`AuthorityManifestStore` is the durable publication boundary. Publication compares
the migration, installation, module, source, target, and source watermark identity
before replacing the matching no-cutover marker. A committed, conflicting, or
indeterminate result has distinct handling. An indeterminate result keeps the
module closed until `recover_authority` resolves the durable record.

Migration status, table checkpoints, outbox items, and capacity reservations each
have validation methods. Status transitions are monotonic and follow the
background protocol; a paused or blocked status must carry a durable reason.
`MigrationTaskPolicy` is an overlay over operator admission and returns the
stable `migration_hold` reason without changing the operator's persisted schedule
or enablement state. Module-aware decisions hold tasks assigned to the active
module, while explicit task keys cover independent obligations. This task-policy
contract does not wire runtime load shedding or scheduler changes into the service;
that behavior remains out of scope here.

## Configuration and test foundation

`StorageConfig` keeps PostgreSQL separate from SQLite paths and archives:

- `POSTGRES_DATABASE_URL` is optional; absence preserves SQLite-only operation;
- `POSTGRES_MAX_CONNECTIONS` defaults to `5` and must be non-zero;
- `MIGRATION_SWITCH_BUDGET_MS` defaults to `3000` and must be non-zero.

The connection string is redacted from `Debug` and omitted from serialized
configuration. `sqlx` enables both SQLite and PostgreSQL with rustls. The
PostgreSQL contract tests do not require a live server: they exercise the
PostgreSQL-capable pool/router type and run all authority, lease, recovery, and
catalog checks against SQLite fixtures. A future adapter may add a live database
profile with `POSTGRES_TEST_DATABASE_URL` without changing the contract tests.

## Explicit non-scope

This inventory does not add a migration worker, change capture, table copy,
PostgreSQL DDL, automatic startup migration, UI, runtime load shedding, deployment
configuration, legacy-table deletion, or failback behavior. Those belong to the
subsequent migration-engine and module-adapter work.
