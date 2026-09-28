# Proxy Invocation Identity Implementation

> The current effective contract remains in `./SPEC.md`; this file records implementation coverage and verification facts.

## Current Status

- Implementation: Implemented for backend PR1
- Lifecycle: active
- Catalog note: Durable backend identity, allocation, persistence, and retention contract.

## Implementation Coverage

- `REQ-PII-001`: `src/prompt_cache_conversations.rs` owns the six-character conversation prefix, four-character base-31 sequence, hourly unbound prefix, overflow handling, and cache recovery; HTTP capture and WebSocket preparation use the allocator.
- `REQ-PII-002`: `prompt_cache_conversations` stores identity and delayed aggregate statistics; `src/schema.rs` invokes its idempotent schema/backfill path.
- `REQ-PII-003`: `AppState` cache state carries conversation identities; allocator recovery scans retained invocation IDs after cache misses and handles concurrent unique-key races.
- `REQ-PII-004`: `src/sqlite_batch_writer.rs` refreshes touched keys after terminal/derived batches and drains a durable refresh queue; `src/maintenance/retention.rs` deletes archived invocations before refreshing affected aggregates, releases orphan masters, and clears released identities from memory.
- `REQ-PII-005`: Allocation, migration, recovery, statistics, retention, collision, and exhaustion paths emit structured diagnostic events using prompt-key fingerprints.

## Verification

- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets --all-features`
- `cargo test prompt_cache_conversation -- --nocapture`
- Focused stateful SQLite tests in `src/tests/stateful_sqlite/invocation_query_filters_and_schema_migrations.rs`.

## Coverage / rollout summary

- The backfill creates the master table and records `prompt_cache_conversations_v1` only after identity creation and aggregate refresh complete. Invocation triggers enqueue affected keys in the durable refresh queue and remove the `prompt_cache_conversations_stats_v2` marker; the marker is restored only when the queue is empty after aggregate refresh. A missing marker or pending queue triggers paged startup recovery, including identity creation for newly discovered keys.
- Existing invocation IDs remain readable as historical rows; only new proxy capture and WebSocket IDs use the compact contract.
- PR2 public response fields and frontend consumers are intentionally deferred.

## Remaining Gaps

- None within the PR1 backend boundary.

## Related Changes

- None recorded until delivery creates the signed-off commit and pull request.

## References

- `./SPEC.md`
- `./HISTORY.md`
