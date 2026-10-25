# Invocation Query and Prompt-Cache Fixture Migration

## Scope

This source-quality extraction starts from baseline commit
`9c8c8aeac7a40728a36b4159a51c8416465b4a64` and removes the complete
`src/tests/stateful_sqlite/invocation_query_filters_and_schema_migrations.rs`
parent in one PR. The parent was 9,982 physical lines. The nine responsibility
modules below contain every existing test, helper, SQL fixture, failure trigger,
rollback assertion, idempotency assertion, lock lifetime, timeout, retry
threshold, and cleanup path.

The requested ranges describe responsibility ownership. At boundaries where the
next test's attributes preceded its function by one or two lines, those
attributes stay with the function so every Rust item remains syntactically
complete. The normalized baseline ranges therefore cover lines 1 through 9,982
exactly once. The removed parent is not replaced by a facade.

| Module                                          | Requested parent range | Normalized source range | Final lines | Top-level functions | Nested helpers | Tests |
| ----------------------------------------------- | ---------------------: | ----------------------: | ----------: | ------------------: | -------------: | ----: |
| `schema_retirement_and_historical_fixtures.rs`  |                  1-853 |                   1-853 |         841 |                  10 |              0 |     6 |
| `prompt_cache_materialization_and_allocator.rs` |               854-2149 |                854-2147 |       1,294 |                  17 |              5 |    17 |
| `proxy_capture_and_invocation_queries.rs`       |              2150-2643 |               2148-2642 |         495 |                   5 |              0 |     5 |
| `prompt_cache_schema_migration_contracts.rs`    |              2644-3141 |               2643-3140 |         498 |                   6 |              0 |     6 |
| `invocation_list_queries_and_filters.rs`        |              3142-4383 |               3141-4382 |       1,242 |                  15 |              0 |    15 |
| `invocation_summary_suggestions_and_stats.rs`   |              4384-6072 |               4383-6071 |       1,689 |                  20 |              0 |    20 |
| `prompt_cache_conversation_reads.rs`            |              6073-6750 |               6072-6749 |         678 |                  11 |              1 |    11 |
| `prompt_cache_binding_and_affinity.rs`          |              6751-8802 |               6750-8801 |       2,052 |                  17 |              0 |    17 |
| `prompt_cache_sticky_runtime.rs`                |              8803-9982 |               8802-9982 |       1,182 |                  14 |              0 |    14 |

The final modules total 9,971 physical lines after rustfmt. The eleven-line
difference from the original is the removal of redundant monolith-only import
statements; all names required by the migrated items remain available through
the existing parent test-module imports. The two migration row types and their
fields that cross the proxy/schema boundary are the only new crate-visible
surface, and they are re-exported explicitly from
`src/tests/stateful_sqlite/mod.rs`.

## Function and Test Manifest

The following is the before/after name manifest. Every name appears once in the
baseline parent and once in its designated child module. There are 115
top-level function definitions, six nested helper functions, and 111 test
functions, for 121 function definitions total. The top-level non-test helpers
and nested helpers are listed separately; every remaining name in each module
is a test function.

### Schema retirement and historical fixtures

Top-level non-test helpers:

```text
seed_websocket_retirement_state
fetch_prompt_cache_conversations
historical_codex_invocations_schema_sql
create_historical_retention_fixture
```

Tests:

```text
retire_websocket_proxy_migration_is_idempotent_and_preserves_unrelated_state
retire_websocket_proxy_migration_rolls_back_and_reenters_after_failure
ensure_schema_repairs_v271_and_v2802_retention_fixtures_without_historical_backfill
ensure_schema_adds_nullable_reported_cache_write_tokens_to_legacy_invocations
ensure_schema_enables_legacy_startup_backfill_rows
ensure_schema_defers_prompt_cache_conversation_materialization_to_startup_backfill
```

### Prompt-cache materialization and allocator

Nested helpers:

```text
insert_invocation
setup_scale_database
run_scale_case
percentile_micros
median_duration
```

Tests:

```text
prompt_cache_conversation_materialization_checkpoints_and_resumes_with_new_keys
prompt_cache_stats_generation_clock_survives_queue_drain
prompt_cache_materialization_yields_only_after_a_committed_micro_batch
prompt_cache_materialization_fixed_400_vs_adaptive_representative_scale
prompt_cache_conversation_read_snapshot_keeps_completeness_with_aggregate_reads
ensure_schema_recovers_prompt_cache_stats_when_refresh_marker_is_missing
ensure_schema_recovers_prompt_cache_identity_after_invocation_payload_update
prompt_cache_conversation_allocator_keeps_sequences_in_memory
prompt_cache_conversation_retention_preserves_active_and_live_identities
prompt_cache_conversation_stats_refresh_is_deferred_after_terminal_batch
prompt_cache_conversation_allocator_recovers_same_key_database_race
prompt_cache_conversation_allocator_reinitializes_after_instance_cleanup
prompt_cache_conversation_allocator_rejects_sequence_exhaustion
prompt_cache_conversation_allocator_retries_colliding_candidates
prompt_cache_conversation_allocator_rejects_invocation_only_prefix_collision
prompt_cache_conversation_allocator_bounds_colliding_candidate_retries
dropped_pool_invocation_recovery_releases_prompt_cache_lease
```

### Proxy capture and invocation queries

Tests:

```text
proxy_capture_target_large_nonstream_json_error_preserves_prefixed_metadata
locate_invocation_returns_account_scoped_anchor_pages
invocation_queries_support_short_invoke_and_attempt_ids
proxy_capture_target_nonstream_usage_survives_response_raw_truncation
resolve_default_source_scope_always_all
```

### Prompt-cache schema migration contracts

Tests:

```text
ensure_schema_adds_codex_imagegen_column_without_losing_existing_binding_policies
ensure_schema_preserves_prompt_cache_binding_timeouts_when_adding_policy_columns
ensure_schema_migrates_pre_timeout_prompt_cache_binding_table
ensure_schema_creates_prompt_cache_conversation_operation_events_table
ensure_schema_creates_sticky_affinity_generation_and_routing_source_storage
ensure_schema_migrates_legacy_hourly_rollup_replay_identity_without_upgrading_unverified_markers
```

### Invocation list queries and filters

Tests:

```text
list_invocations_projects_payload_context_fields
list_invocations_filters_by_sticky_key_and_upstream_account_id
invocation_queries_filter_upstream_scope_and_treat_legacy_rows_as_external
list_invocations_filters_extended_route_and_diagnostics_dimensions
list_invocations_response_omits_raw_expires_at_field
list_invocations_tolerates_malformed_payload_json
list_invocations_ignores_non_numeric_proxy_weight_delta
list_invocations_preserves_historical_xy_records
list_invocations_legacy_limit_query_skips_snapshot_shape
list_invocations_keeps_snapshot_stable_across_pagination_and_sorting
list_invocations_failure_class_filter_matches_resolved_classification
list_invocations_status_failed_matches_http_failure_statuses
list_invocations_status_success_excludes_resolved_failures
list_invocations_warning_success_is_separate_from_success_and_failed
list_invocations_status_sort_uses_normalized_status_values
```

### Invocation summary, suggestions, and stats

Tests:

```text
fetch_invocation_summary_reports_new_records_count_for_applied_filters
fetch_invocation_summary_resolves_failure_class_for_legacy_rows
fetch_invocation_summary_normalizes_top_level_success_and_failure_counts
fetch_invocation_summary_keeps_zero_ms_ttft_and_excludes_negative_samples
list_invocations_keeps_negative_ttft_in_requesting_phase
persisted_nonfinite_timing_is_unavailable_to_phase_and_summary_queries
fetch_invocation_summary_returns_zero_values_for_empty_results
fetch_invocation_summary_p95_ignores_zero_response_duration_placeholders
fetch_invocation_summary_ignores_stale_retry_timing
fetch_invocation_new_records_count_requires_snapshot_id
fetch_invocation_new_records_count_uses_snapshot_boundary
list_invocations_total_tokens_range_filters_exclude_null_values
list_invocations_total_ms_range_filters_exclude_null_values
fetch_invocation_suggestions_orders_by_count_and_respects_time_bounds
fetch_invocation_suggestions_include_extended_buckets_and_account_labels
list_invocations_supports_model_target_multi_select_and_rerouted_filters
fetch_invocation_suggestions_filters_active_bucket_before_limit
fetch_invocation_suggestions_use_snapshot_and_keep_other_filters
stats_endpoints_preserve_historical_xy_records
yesterday_summary_and_timeseries_only_include_previous_local_day
```

### Prompt-cache conversation reads

Nested helpers:

```text
insert_row
```

Tests:

```text
normalize_prompt_cache_conversation_limit_accepts_whitelist_values_only
normalize_prompt_cache_conversation_activity_hours_accepts_whitelist_values_only
normalize_prompt_cache_conversation_activity_minutes_accepts_precise_five_minutes_only
resolve_prompt_cache_conversation_selection_rejects_mutually_exclusive_params
resolve_prompt_cache_conversation_selection_rejects_activity_hours_and_minutes_combo
prompt_cache_conversations_groups_recent_keys_and_uses_history_totals
prompt_cache_last24h_requests_keep_null_status_rows_neutral
prompt_cache_last24h_requests_treat_running_rows_with_failure_class_as_failures
prompt_cache_last24h_requests_treat_running_rows_with_error_text_as_failures
prompt_cache_last24h_requests_treat_pending_rows_with_failure_kind_as_failures
prompt_cache_last24h_requests_keep_status_only_http_failures_marked_as_failures
```

### Prompt-cache binding and affinity

Tests:

```text
prompt_cache_conversation_binding_patch_is_mutually_exclusive_and_clearable
prompt_cache_conversation_proxy_override_bypasses_node_shunt_group_slots
prompt_cache_conversation_binding_route_accepts_encoded_key
prompt_cache_group_binding_timeout_response_ignores_account_overrides
prompt_cache_conversation_binding_reports_encrypted_owner_and_clear_keeps_owner_lock
prompt_cache_conversation_binding_hides_encrypted_owner_when_routing_disabled
prompt_cache_group_binding_promotes_to_account_after_encrypted_owner_lock
bulk_prompt_cache_conversation_bindings_bind_to_upstream_account_across_keys
bulk_prompt_cache_conversation_bindings_bind_none_clears_only_manual_binding
bulk_prompt_cache_conversation_bindings_clear_and_reset_affinity_removes_all_affinity_rows
prompt_cache_clear_and_reset_affinity_fences_stale_sticky_revival_and_emits_only_fresh_runtime_sticky_event
prompt_cache_sticky_routes_are_isolated_by_normalized_model_key
prompt_cache_manual_binding_is_idempotent_for_unchanged_model_routes
prompt_cache_manual_binding_rolls_back_when_sticky_update_fails
prompt_cache_routing_events_filter_by_all_or_normalized_model_scope
fresh_assignment_persists_selection_audit_for_attempt_and_sticky_event
model_scoped_sticky_clear_cause_does_not_cross_models
```

### Prompt-cache sticky runtime

Tests:

```text
runtime_sticky_first_concurrent_success_locks_target_and_audits_late_completion
runtime_sticky_route_change_broadcasts_previous_and_current_history_scopes
runtime_sticky_upsert_rechecks_generation_after_waiting_for_write_lock
generic_sticky_route_success_does_not_emit_prompt_cache_conversation_event
bulk_prompt_cache_conversation_bindings_set_fast_mode_rewrite_mode_preserves_binding_kind
prompt_cache_conversation_operation_events_list_filters_by_info_type
bulk_prompt_cache_conversation_bindings_reject_invalid_target_without_partial_writes
prompt_cache_same_account_binding_newer_than_owner_keeps_owner_guard_active
prompt_cache_same_group_binding_overrides_owner_guard
prompt_cache_pre_owner_group_binding_keeps_owner_guard_active
prompt_cache_manual_override_wins_when_binding_and_owner_share_same_second
prompt_cache_manual_override_wins_even_after_owner_reconfirmation
prompt_cache_group_promotion_ignores_stale_group_after_operator_rebind
prompt_cache_stale_success_cannot_reclaim_owner_after_override_migration
```

## Completeness and inventory evidence

The baseline manifest reports 121 function definitions and 111 test
attributes. The post-migration manifest reports the same counts and the same
function-name multiset across the nine modules. The parent path is absent, all
nine policy paths exist, every policy line budget matches the formatted file,
and every new module is below the 3,000-line test-helper target.

The policy removes only the obsolete parent entry, records nine replacement
entries with their final physical line counts, changes the explicit inventory
from 35 to 43 entries, and retains the immutable candidate counts of 32
production and 23 test/helper. The suppression inventory remains 126 entries.

The migration keeps `src/api/slices/invocations_and_summary.rs`,
`src/schema.rs`, production code, SQL, transaction boundaries, database pool
settings, runtime timing, concurrency capacity, assertions, and resource
classification unchanged.
