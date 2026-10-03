# Proxy Invocation Identity Implementation

> The current effective contract remains in `./SPEC.md`; this file records implementation coverage and verification facts.

## Current Status

- Implementation: Implemented for prompt-cache materialization and long-wait proxy isolation
- Lifecycle: active
- Catalog note: Durable backend identity, allocation, persistence, and retention contract.

## Implementation Coverage

- `REQ-PII-001`: `src/prompt_cache_conversations.rs` owns the six-character conversation prefix, four-character base-31 sequence, hourly unbound prefix, overflow handling, and cache recovery; HTTP capture and WebSocket preparation use the allocator.
- `REQ-PII-002`: `prompt_cache_conversations` stores identity and delayed aggregate statistics; `src/schema.rs` installs only additive structure, while the ordered `prompt_cache_conversations_materialization_v1` startup task performs historical materialization in the background. Each 400-key logical page uses an in-memory adaptive 64..400 micro-batch controller with a 32-key floor. The additive `idx_pool_attempts_account_model_success` partial covering index supports the model-health latest-success check without a temporary sort and is safe to create repeatedly during startup.
- `REQ-PII-003`: `AppState` cache state carries conversation identities; normalized prompt-cache keys use independent `Arc`/`Weak` allocation locks whose idle registry entries are reclaimed. Identity recovery and sequence reservation run outside the global cache mutex, then update cached state briefly after the interactive SQLite permit is released. Active references are registered before waiting and a drop guard releases them on cancellation; cache capacity remains 4096 and a full cache remains a durable-cache miss rather than a request rejection. Unbound hourly prefixes use a process-local namespace lock only during initialization and exclude issued prefixes, conversation masters, and invocation prefixes.
- `REQ-PII-004`: `src/sqlite_batch_writer.rs` and prompt-cache key backfill enqueue touched keys without synchronously scanning retained invocations; the startup materialization task drains the durable refresh queue. Identity discovery commits adaptive identity batches and their cursor together; statistics use bounded staging pages and publish only complete, generation-consistent aggregates. The task yields at committed boundaries and pauses for a 10 ms cooperative scheduler window between adaptive commits, while other startup backfills retain their existing cancellation behavior. `src/maintenance/retention.rs` deletes archived invocations before refreshing affected aggregates, releases orphan masters, and clears released identities from memory.
- `REQ-PII-005`: Allocation, migration, recovery, statistics, retention, collision, and exhaustion paths emit structured diagnostic events using prompt-key fingerprints.
- Long-wait acceptance coverage: `scripts/shared-testbox-performance-acceptance --scenario long-wait --duration 600 --rounds 3 --rate 4` drives eight 179-second/180-second-boundary proxy calls, two fast workers, same-key/different-key/unbound allocation, and cancellation while recording allocation, request, terminal confirmation, status, and incomplete-work samples. Fast-call terminal confirmation is observed immediately after each response so the measurement timestamp does not include the sustained workload window; cancellation requires a persisted downstream-closed/client-abort terminal row as well as a bounded client-close duration.
- Operational follow-up: `GET/PATCH /api/system/prompt-cache/materialization` and managed-task control use the same committed `managed_tasks.enabled` value in the maintenance database. The matching `startup_backfill_progress` checkpoint is updated in that maintenance transaction. Both PATCH handlers use the existing 503 Unavailable response when the control transaction fails. A one-time origin marker preserves an existing maintenance control; only a newly created managed row may be seeded from the old business-database enable bit. The business bit is not read by the run path or written by this task. An in-memory generation gate lets an already registered short SQL step finish while preventing later steps from starting under a stale generation. The existing status response still exposes durable progress, estimated remaining time, recent bounded run history, and the pause/resume control.

## Verification

- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets --all-features`
- `cargo clippy --locked --all-targets --all-features -- -D warnings`
- `cargo test prompt_cache_conversation -- --nocapture`
- `cargo test prompt_cache_conversation_allocator_releases_lease_after_cancellation -- --nocapture`
- `cargo test ensure_schema_adds_idempotent_success_attempt_route_lookup_index -- --nocapture`
- `cargo test prompt_cache_materialization_yields_only_after_a_committed_micro_batch -- --nocapture`
- `cargo test prompt_cache_materialization_large_key_is_paged_once_across_phases -- --nocapture`
- `cargo test prompt_cache_materialization_pages_resume_across_generation_change_and_control_restart -- --nocapture`
- `cargo test prompt_cache_materialization_uses_maintenance_control_when_legacy_business_flag_disagrees -- --nocapture`
- `cargo test prompt_cache_materialization_wake_preserves_operator_disable -- --nocapture`
- `cargo test prompt_cache_materialization_failure_history_reports_per_run_work -- --nocapture`
- `cargo test existing_maintenance_prompt_cache_control_and_checkpoint_win_over_legacy_state -- --nocapture`
- `cargo test legacy_only_prompt_cache_control_is_imported_once_into_maintenance -- --nocapture`
- `cargo test prompt_cache_control -- --nocapture`
- `cargo test prompt_cache_materialization_fixed_400_vs_adaptive_representative_scale -- --ignored --nocapture --test-threads=1`
- `bash .github/scripts/run-backend-tests.sh --profile stateful-sqlite`
- Focused stateful SQLite tests in `src/tests/stateful_sqlite/proxy_long_wait_allocation_and_index.rs` and `src/tests/stateful_sqlite/prompt_cache_materialization_control.rs`.
- The long-wait workload is a required Linux shared-testbox acceptance run; its candidate SHA, run directory, and per-round gate results are recorded with the delivery evidence.
- The `prompt-cache-control` service-process fixture records every online call, persistent identity and staging checkpoints, pause/resume generations, maintenance file-lock responses, and restart completion. Its 30-second progress clock runs only while the task is enabled and eligible: a future continuation deadline or an unresolved `coordinator_priority` notification wait is excluded. Expiring the priority fallback time alone does not prove that P2 admission is available. The independent 180-second post-input completion bound still applies to the entire observation window.
- Baseline `prompt-cache-control` replay at `b68295595d875aead33ef31514eaf602279ece94`: the isolated service accepted a managed run with maintenance control enabled and the legacy business bit disabled, but after 60 seconds retained 400 queued keys with zero materialized identities, staging rows, or materialization-run records. Evidence: `/srv/codex/agents/01a0f565-fc82-7563-a079-af80ead822b6/runs/20261002_143605_performance_b68295595d87_75209/logs/loadgen.jsonl` and `run-metadata.json`.

## Coverage / rollout summary

- `ensure_schema()` creates the conversation master, indexes, mutation triggers, durable refresh queue, and `prompt_cache_conversation_migration_progress`; it does not scan historical invocations, so HTTP readiness is not held by historical data volume.
- The `prompt_cache_conversations_materialization_v1` startup task runs in ordered `identity_backfill`, `identity_reconciliation`, `stats_rebuild`, and `queue_drain` phases. Identity backfill snapshots the maximum invocation row ID, paginates prompt-cache keys, and finishes with an uncursored missing-identity reconciliation. Identity phases commit identities, `cursor_key`, and the identity-key counter together, without scanning aggregate statistics. Statistics phases own the bounded source-page traversal; reruns are idempotent. Completed logical pages and empty phase transitions continue only within the original scan and elapsed-time budgets.
- The `prompt_cache_conversations_v1` identity marker, `prompt_cache_conversations_stats_v2` freshness marker, complete migration phase, and empty durable refresh queue are required together. Aggregate prompt-cache reads and subscription baselines return the existing `ApiError::Unavailable` contract until all conditions hold; partial or zero-valued complete results are not published.
- Invocation triggers continue to enqueue affected keys and remove the freshness marker. New requests remain writable during migration, and failures, SQLite pressure, or process restarts leave durable checkpoints for later retry. Each identity batch or statistics page has a committed checkpoint. A partial statistics page preserves staging progress; only the final generation-consistent page publishes the aggregate and clears its queue generation. The outer statistics cursor advances after the required keys are complete; a priority waiter defers the next boundary after the current step commits.
- The maintenance `managed_tasks.enabled` row is the prompt-cache task's only enablement authority. Its scheduler checkpoint is committed in the same maintenance-database transaction, then the in-memory generation is published before the control PATCH returns. A one-time maintenance metadata marker records whether this managed row existed before task seeding, so a pre-existing maintenance choice wins and a legacy business enable bit seeds only a genuinely new control. The business bit is not queried by the prompt-cache run path or synchronized. A short in-memory permit is acquired before each identity batch, statistics page, cursor update, and phase transition; it is released before/after the bounded business SQL step and never waits on the maintenance database.
- Maintenance checkpoint start/finalization and business-triggered wakes share an asynchronous control-update fence. The generation is rechecked after acquiring the fence, so an older result cannot overwrite a newer pause/resume checkpoint. An already admitted checkpoint may commit before a queued control update, which then takes authority. This fence never spans business materialization SQL; online write admission is released before maintenance-database reads, control-fence waits, and final checkpoint writes. In-memory scheduler mutations also recheck the published generation, preserving a newer resume wake.
- Statistics-page results distinguish complete, pending, source-generation change, budget exhaustion, actual disablement, priority yield, and unavailable maintenance control. Committed staging cursors remain resumable; an incomplete statistics page does not advance the outer key cursor or publish a freshness marker. Page/generation/budget continuations use the existing 15-second follow-up, while coordinator and database pressure retain their event/deadline behavior. Actual disable clears queued, due, and pressure-deferred scheduler state; a new enable generation supplies one wake. Repeated same-value control updates and ordinary business events preserve an unexpired retry deadline.
- Proxy allocation follows the short lock order: briefly access the cache to obtain a per-key lock, release it, enter `InteractiveProxy` only for identity/sequence SQL and consistency reads, release admission, then briefly update cached state. No network wait is inside either the global cache mutex or SQLite write admission; P1 terminal writers retain priority. The route lookup index is additive and older readers continue to use existing rows if they do not recognize it.
- The adaptive controller starts at 64 keys, doubles after two successful <=50 ms batches, halves at >=200 ms or pressure/failure, and stays within 32..400. Its state is process-local and is reset to 64 after restart. Statistics staging is bounded to 256 invocation rows per source page and a final publish transaction; identity discovery does not repeat those scans. After each committed adaptive batch with more work remaining, a 10 ms scheduler window gives foreground readers a bounded chance to run. Startup logs include `batch_count`, `last_batch_size`, `max_batch_size`, `batch_elapsed_ms`, and `defer_reason`.
- The migration progress row stores the fixed source total and committed identity-key counter after the identity snapshot; stats cursor commits do not double-count those keys. The status surface caps incomplete phases below 100%, reports zero ETA only for the complete phase, and reads only durable counters, bounded queue sampling, and the latest 10 records. Schema setup repairs counters for older complete rows, while failure history preserves the durable phase checkpoint.
- Earlier adaptive-batch controlled representative SQLite acceptance passed with 3 trials over 4,000 keys and 40,000 invocations: fixed median 129812 ms versus adaptive median 103232 ms; foreground p95 251 us versus 266 us and p99 622 us versus 613 us. The ignored fixture is an explicit empirical acceptance command rather than the unrelated summary-scale CI selector.
- Existing invocation IDs remain readable as historical rows; only new proxy capture and WebSocket IDs use the compact contract.
- PR2 public response fields and frontend consumers are intentionally deferred.

## Remaining Gaps

- Three-round Linux, non-test service-process `prompt-cache-control` acceptance is a required delivery gate. Delivery evidence carries the exact candidate SHA, scenario digest, persistent checkpoint samples, and completion results; the older adaptive benchmark does not replace this gate.
- The representative runtime fixture is intentionally ignored by default and must be rerun in a controlled environment when SQLite size or lock-contention characteristics change.

## Related Changes

- `docs/adr/0021-prompt-cache-background-materialization.md`
- `docs/adr/0022-prompt-cache-adaptive-materialization.md`
- `docs/adr/0027-prompt-cache-materialization-step-boundaries.md`
- `docs/specs/proxy-invocation-identity/assets/version-impact-record.json`

## References

- `./SPEC.md`
- `./HISTORY.md`
