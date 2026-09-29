# Proxy Invocation Identity Implementation

> The current effective contract remains in `./SPEC.md`; this file records implementation coverage and verification facts.

## Current Status

- Implementation: Implemented for the adaptive prompt-cache materialization follow-up
- Lifecycle: active
- Catalog note: Durable backend identity, allocation, persistence, and retention contract.

## Implementation Coverage

- `REQ-PII-001`: `src/prompt_cache_conversations.rs` owns the six-character conversation prefix, four-character base-31 sequence, hourly unbound prefix, overflow handling, and cache recovery; HTTP capture and WebSocket preparation use the allocator.
- `REQ-PII-002`: `prompt_cache_conversations` stores identity and delayed aggregate statistics; `src/schema.rs` installs only additive structure, while the ordered `prompt_cache_conversations_materialization_v1` startup task performs historical materialization in the background. Each 400-key logical page uses an in-memory adaptive 64..400 micro-batch controller with a 32-key floor.
- `REQ-PII-003`: `AppState` cache state carries conversation identities; allocator recovery scans retained invocation IDs after cache misses and handles concurrent unique-key races.
- `REQ-PII-004`: `src/sqlite_batch_writer.rs` and prompt-cache key backfill enqueue touched keys without synchronously scanning retained invocations; the startup materialization task drains the durable refresh queue. Prompt-cache micro-batches use one set-based aggregate writeback per transaction, yield only at transaction boundaries, and pause for a 10 ms cooperative scheduler window between adaptive commits, while other startup backfills retain their existing cancellation behavior. `src/maintenance/retention.rs` deletes archived invocations before refreshing affected aggregates, releases orphan masters, and clears released identities from memory.
- `REQ-PII-005`: Allocation, migration, recovery, statistics, retention, collision, and exhaustion paths emit structured diagnostic events using prompt-key fingerprints.
- Operational follow-up: `GET/PATCH /api/system/prompt-cache/materialization` and the System > Cache materialization page expose durable progress, estimated remaining time, recent bounded run history, and a pause/resume control. The control takes effect at the next committed micro-batch boundary and resumes from the durable cursor.

## Verification

- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets --all-features`
- `cargo clippy --locked --all-targets --all-features -- -D warnings`
- `cargo test prompt_cache_conversation -- --nocapture`
- `cargo test prompt_cache_materialization_yields_only_after_a_committed_micro_batch -- --nocapture`
- `cargo test prompt_cache_materialization_fixed_400_vs_adaptive_representative_scale -- --ignored --nocapture --test-threads=1`
- `bash .github/scripts/run-backend-tests.sh --profile stateful-sqlite`
- Focused stateful SQLite tests in `src/tests/stateful_sqlite/invocation_query_filters_and_schema_migrations.rs`.

## Coverage / rollout summary

- `ensure_schema()` creates the conversation master, indexes, mutation triggers, durable refresh queue, and `prompt_cache_conversation_migration_progress`; it does not scan historical invocations, so HTTP readiness is not held by historical data volume.
- The `prompt_cache_conversations_materialization_v1` startup task runs in ordered `identity_backfill`, `identity_reconciliation`, `stats_rebuild`, and `queue_drain` phases. Identity backfill snapshots the maximum invocation row ID, paginates prompt-cache keys, and finishes with an uncursored missing-identity reconciliation. Each page commits its data before advancing `cursor_key`; reruns are idempotent.
- The `prompt_cache_conversations_v1` identity marker, `prompt_cache_conversations_stats_v2` freshness marker, complete migration phase, and empty durable refresh queue are required together. Aggregate prompt-cache reads and subscription baselines return the existing `ApiError::Unavailable` contract until all conditions hold; partial or zero-valued complete results are not published.
- Invocation triggers continue to enqueue affected keys and remove the freshness marker. New requests remain writable during migration, and failures, SQLite pressure, or process restarts leave durable checkpoints for later retry. A micro-batch commits identity ensure, aggregate refresh, queue clear, and its cursor checkpoint in one transaction; a priority waiter defers only the next boundary after the current batch commits.
- The adaptive controller starts at 64 keys, doubles after two successful <=50 ms batches, halves at >=200 ms or pressure/failure, and stays within 32..400. Its state is process-local and is reset to 64 after restart. Aggregate rows are written back with one set-based update per micro-batch. After each committed adaptive batch with more work remaining, a 10 ms scheduler window gives foreground readers a bounded chance to run. Startup logs include `batch_count`, `last_batch_size`, `max_batch_size`, `batch_elapsed_ms`, and `defer_reason`.
- The migration progress row stores the fixed source total and committed-key counter after the identity snapshot, while `prompt_cache_conversation_materialization_runs` retains the latest 100 task calls. The status surface reads only durable counters, bounded queue sampling, and the latest 10 records; it does not rescan historical invocations or conversation rows.
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
