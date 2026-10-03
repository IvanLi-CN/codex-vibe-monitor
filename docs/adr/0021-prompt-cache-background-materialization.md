# Prompt-Cache Historical Materialization Runs in the Background

## Status

Accepted

The combined identity/statistics transaction boundary below is succeeded by
[ADR 0027](0027-prompt-cache-materialization-step-boundaries.md). Ordered phases and read
completeness remain active.

## Context

The prompt-cache conversation master is derived from retained proxy invocations. Installing its
tables and triggers is a fast structural operation, but scanning historical prompt-cache keys and
rebuilding aggregate statistics can scale with the SQLite database and compete with foreground
writes. Making that work part of the HTTP readiness path can therefore make a healthy process look
unavailable for an unbounded period.

The derived rows also need a durable completeness boundary. During a long migration, an aggregate
read must not silently mix materialized history with missing history or substitute zero values for
data that has not been scanned yet.

## Decision

`ensure_prompt_cache_conversations_schema()` installs only additive structure: the conversation
master and indexes, the durable refresh queue, invocation mutation triggers, and the
`prompt_cache_conversation_migration_progress` state table. HTTP readiness may proceed after this
structure is installed.

Historical materialization is an ordered startup backfill task named
`prompt_cache_conversations_materialization_v1`. Its durable phases are:

1. `identity_backfill` snapshots `MAX(codex_invocations.id)` and paginates distinct prompt-cache
   keys in key order. The snapshot bounds the initial scan, and `cursor_key` advances in the same
   transaction as each committed identity/statistics batch.
2. `identity_reconciliation` performs an uncursored, bounded search for missing identities so
   keys created or changed while the snapshot scan was running are included.
3. `stats_rebuild` paginates existing conversation keys and refreshes their aggregates.
4. `queue_drain` refreshes and clears durable mutation-queue entries. When the queue is empty and
   identity coverage is complete, the task records the `complete` phase and the statistics marker.

The identity marker, statistics freshness marker, complete phase, and empty refresh queue are a
single read-completeness contract. Until all four conditions hold, prompt-cache aggregate reads
and subscription baselines return the existing `ApiError::Unavailable` error. They do not expose
partial statistics or zero-valued placeholders. New invocation writes remain available; terminal
writes and prompt-cache key backfills only invalidate freshness and enqueue affected keys. The
startup materialization task owns the historical aggregate refresh so foreground P1/P2 write paths
never synchronously scan retained invocation payloads.

The startup backfill scheduler owns pressure admission, P2 write coordination, cancellation,
failure backoff, and task wake-up. Each micro-batch commits its data and progress cursor together,
so process exit or transaction failure repeats only an idempotent micro-batch. Prompt-cache
materialization may refine a page into smaller committed transactions and yields only between those transactions;
the shared cancellation behavior for other startup backfills is unchanged. A later run can
forward-repair missing identities, stale markers, or pending queue entries without rewriting
existing conversation IDs. WebSocket behavior is outside this decision.

## Consequences

- HTTP readiness is independent of historical prompt-cache row count and aggregate rebuild time.
- Progress is observable through the durable migration phase/cursor and existing startup-backfill
  scanned/updated/status records and structured task logs.
- A newly started process can serve normal writes while aggregate prompt-cache reads fail closed
  until materialization converges; queued aggregate refreshes do not extend foreground write
  transactions or retry in the online writer.
- Deployment rollback does not reverse the additive schema or derived rows; a newer program
  version performs forward repair.

## References

- `docs/specs/proxy-invocation-identity/IMPLEMENTATION.md`
- `docs/specs/proxy-invocation-identity/assets/persistent-state-migration-record.json`
- `docs/adr/0022-prompt-cache-adaptive-materialization.md`
