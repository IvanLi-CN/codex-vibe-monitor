# Proxy Invocation Identity Implementation

> The current effective contract remains in `./SPEC.md`; this file records implementation coverage and verification facts.

## Current Status

- Implementation: Implemented for the prompt-cache materialization follow-up
- Lifecycle: active
- Catalog note: Durable backend identity, allocation, persistence, and retention contract.

## Implementation Coverage

- `REQ-PII-001`: `src/prompt_cache_conversations.rs` owns the six-character conversation prefix, four-character base-31 sequence, hourly unbound prefix, overflow handling, and cache recovery; HTTP capture and WebSocket preparation use the allocator.
- `REQ-PII-002`: `prompt_cache_conversations` stores identity and delayed aggregate statistics; `src/schema.rs` installs only additive structure, while the ordered `prompt_cache_conversations_materialization_v1` startup task performs historical materialization in the background.
- `REQ-PII-003`: `AppState` cache state carries conversation identities; allocator recovery scans retained invocation IDs after cache misses and handles concurrent unique-key races.
- `REQ-PII-004`: `src/sqlite_batch_writer.rs` and prompt-cache key backfill enqueue touched keys without synchronously scanning retained invocations; the startup materialization task drains the durable refresh queue. `src/maintenance/retention.rs` deletes archived invocations before refreshing affected aggregates, releases orphan masters, and clears released identities from memory.
- `REQ-PII-005`: Allocation, migration, recovery, statistics, retention, collision, and exhaustion paths emit structured diagnostic events using prompt-key fingerprints.

## Verification

- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets --all-features`
- `cargo clippy --locked --all-targets --all-features -- -D warnings`
- `cargo test prompt_cache_conversation -- --nocapture`
- `bash .github/scripts/run-backend-tests.sh --profile stateful-sqlite`
- Focused stateful SQLite tests in `src/tests/stateful_sqlite/invocation_query_filters_and_schema_migrations.rs`.

## Coverage / rollout summary

- `ensure_schema()` creates the conversation master, indexes, mutation triggers, durable refresh queue, and `prompt_cache_conversation_migration_progress`; it does not scan historical invocations, so HTTP readiness is not held by historical data volume.
- The `prompt_cache_conversations_materialization_v1` startup task runs in ordered `identity_backfill`, `identity_reconciliation`, `stats_rebuild`, and `queue_drain` phases. Identity backfill snapshots the maximum invocation row ID, paginates prompt-cache keys, and finishes with an uncursored missing-identity reconciliation. Each page commits its data before advancing `cursor_key`; reruns are idempotent.
- The `prompt_cache_conversations_v1` identity marker, `prompt_cache_conversations_stats_v2` freshness marker, complete migration phase, and empty durable refresh queue are required together. Aggregate prompt-cache reads and subscription baselines return the existing `ApiError::Unavailable` contract until all conditions hold; partial or zero-valued complete results are not published.
- Invocation triggers continue to enqueue affected keys and remove the freshness marker. New requests remain writable during migration, and failures, cancellation, SQLite pressure, or process restarts leave durable checkpoints for later retry.
- Existing invocation IDs remain readable as historical rows; only new proxy capture and WebSocket IDs use the compact contract.
- PR2 public response fields and frontend consumers are intentionally deferred.

## Remaining Gaps

- No known gaps within the prompt-cache background materialization boundary. Representative large-database runtime evidence remains a delivery acceptance item because deterministic tests cannot reproduce every SQLite size and lock-contention profile.

## Related Changes

- None recorded until delivery creates the signed-off commit and pull request.

## References

- `./SPEC.md`
- `./HISTORY.md`
