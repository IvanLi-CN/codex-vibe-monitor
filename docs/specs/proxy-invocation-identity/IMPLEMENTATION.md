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
- `REQ-PII-004`: `src/sqlite_batch_writer.rs` and prompt-cache key backfill enqueue touched keys without synchronously scanning retained invocations; the startup materialization task drains the durable refresh queue. Prompt-cache micro-batches use one set-based aggregate writeback per transaction, yield only at transaction boundaries, and pause for a 10 ms cooperative scheduler window between adaptive commits, while other startup backfills retain their existing cancellation behavior. `src/maintenance/retention.rs` deletes archived invocations before refreshing affected aggregates, releases orphan masters, and clears released identities from memory.
- `REQ-PII-005`: Allocation, migration, recovery, statistics, retention, collision, and exhaustion paths emit structured diagnostic events using prompt-key fingerprints.
- Operational follow-up: `GET/PATCH /api/system/prompt-cache/materialization` and the System > Cache materialization page expose durable progress, estimated remaining time, recent bounded run history, and a pause/resume control. The control takes effect at the next committed micro-batch boundary and resumes from the durable cursor.

## Verification

- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets --all-features`
- `cargo clippy --locked --all-targets --all-features -- -D warnings`
- `cargo test prompt_cache_conversation -- --nocapture`
- `cargo test prompt_cache_conversation_allocator_releases_lease_after_cancellation -- --nocapture`
- `cargo test ensure_schema_adds_idempotent_success_attempt_route_lookup_index -- --nocapture`
- `cargo test prompt_cache_materialization_yields_only_after_a_committed_micro_batch -- --nocapture`
- `cargo test prompt_cache_materialization_fixed_400_vs_adaptive_representative_scale -- --ignored --nocapture --test-threads=1`
- `bash .github/scripts/run-backend-tests.sh --profile stateful-sqlite`
- Focused stateful SQLite tests in `src/tests/stateful_sqlite/invocation_query_filters_and_schema_migrations.rs`.

## Coverage / rollout summary

- `ensure_schema()` creates the conversation master, indexes, mutation triggers, durable refresh queue, and `prompt_cache_conversation_migration_progress`; it does not scan historical invocations, so HTTP readiness is not held by historical data volume.
- The `prompt_cache_conversations_materialization_v1` startup task runs in ordered `identity_backfill`, `identity_reconciliation`, `stats_rebuild`, and `queue_drain` phases. Identity backfill snapshots the maximum invocation row ID, paginates prompt-cache keys, and finishes with an uncursored missing-identity reconciliation. Each page commits its data before advancing `cursor_key`; reruns are idempotent.
- The `prompt_cache_conversations_v1` identity marker, `prompt_cache_conversations_stats_v2` freshness marker, complete migration phase, and empty durable refresh queue are required together. Aggregate prompt-cache reads and subscription baselines return the existing `ApiError::Unavailable` contract until all conditions hold; partial or zero-valued complete results are not published.
- Invocation triggers continue to enqueue affected keys and remove the freshness marker. New requests remain writable during migration, and failures, SQLite pressure, or process restarts leave durable checkpoints for later retry. A micro-batch commits identity ensure, aggregate refresh, queue clear, and its cursor checkpoint in one transaction; a priority waiter defers only the next boundary after the current batch commits.
- Proxy allocation follows the short lock order: briefly access the cache to obtain a per-key lock, release it, enter `InteractiveProxy` only for identity/sequence SQL and consistency reads, release admission, then briefly update cached state. No network wait is inside either the global cache mutex or SQLite write admission; P1 terminal writers retain priority. The route lookup index is additive and older readers continue to use existing rows if they do not recognize it.
- The adaptive controller starts at 64 keys, doubles after two successful <=50 ms batches, halves at >=200 ms or pressure/failure, and stays within 32..400. Its state is process-local and is reset to 64 after restart. Aggregate rows are written back with one set-based update per micro-batch. After each committed adaptive batch with more work remaining, a 10 ms scheduler window gives foreground readers a bounded chance to run. Startup logs include `batch_count`, `last_batch_size`, `max_batch_size`, `batch_elapsed_ms`, and `defer_reason`.
- The migration progress row stores the fixed source total and committed identity-key counter after the identity snapshot; stats cursor commits do not double-count those keys. The status surface caps incomplete phases below 100%, reports zero ETA only for the complete phase, and reads only durable counters, bounded queue sampling, and the latest 10 records. Schema setup repairs counters for older complete rows, while failure history preserves the durable phase checkpoint.
- Current-head controlled representative SQLite acceptance passed with 3 trials over 4,000 keys and 40,000 invocations: fixed median 129812 ms versus adaptive median 103232 ms; foreground p95 251 us versus 266 us and p99 622 us versus 613 us. The ignored fixture is an explicit empirical acceptance command rather than the unrelated summary-scale CI selector.
- Existing invocation IDs remain readable as historical rows; only new proxy capture and WebSocket IDs use the compact contract.
- PR2 public response fields and frontend consumers are intentionally deferred.

## Remaining Gaps

- No known gaps within the prompt-cache background materialization boundary. The representative runtime fixture is intentionally ignored by default and must be rerun in a controlled environment when SQLite size or lock-contention characteristics change.

## Related Changes

- `docs/adr/0021-prompt-cache-background-materialization.md`
- `docs/adr/0022-prompt-cache-adaptive-materialization.md`
- `docs/specs/proxy-invocation-identity/assets/version-impact-record.json`

## References

- `./SPEC.md`
- `./HISTORY.md`
