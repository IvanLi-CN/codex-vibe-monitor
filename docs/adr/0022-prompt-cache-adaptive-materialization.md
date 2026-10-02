# Prompt-Cache Adaptive Materialization Batches

## Status

Accepted

## Context

The background materialization contract from ADR 0021 removed historical scanning from HTTP
readiness and moved aggregate refreshes out of online writes. A fixed 400-key transaction still
holds SQLite's write lock for too long on large databases, while a permanently small batch wastes
throughput when the database is quiet. Interactive writes also must not cancel a transaction after
it has started: repeated mid-batch preemption can leave progress permanently starved.

## Decision

Keep 400 keys as the logical page and split its work into in-memory adaptive micro-batches. A new
process starts at 64 keys, may range from 32 through 400, and observes each committed transaction:

- two consecutive successful transactions at or below 50 ms double the next batch;
- a transaction at or above 200 ms, a SQLite busy/locked failure, or a priority waiter observed at
  the boundary halves the next batch;
- all values remain clamped to 32..400 and the controller is reset on process restart.

Each micro-batch uses one SQLite transaction for identity ensure, aggregate refresh, queue clear,
the phase cursor checkpoint, and commit. A process exit or transaction failure therefore repeats
the latest micro-batch idempotently instead of skipping it.

Aggregate rows discovered by the grouped refresh are written back with one set-based update per
micro-batch rather than one SQLite update statement per conversation key. This keeps the smaller
transaction boundaries from multiplying per-key write overhead while preserving the same atomic
identity, statistics, queue, and cursor commit.

Prompt-cache materialization checks the existing P1/interactive waiter signal before starting a
micro-batch and after its commit. The task's `managed_tasks.enabled` row in the maintenance
database is its sole operator-control authority; its `startup_backfill_progress` scheduler
checkpoint is kept consistent in the same maintenance transaction. A short in-memory generation
gate is checked before each business-database SQL step. A step registered before a committed
control change may finish, while subsequent steps from the old generation stop. The business
transaction never reads or waits for the maintenance database. If a waiter or operator disable
arrives, the completed batch remains committed and the next batch is deferred. When another
adaptive micro-batch remains, the committed boundary also includes a 10 ms cooperative scheduler
window so foreground readers can acquire SQLite access between commits; this wait is outside the
transaction and cannot interrupt the active batch. Other startup backfills retain the existing P2
cancellation behavior.

Statistics page outcomes are distinct from operator disablement. A committed partial page keeps
its staging cursor and does not advance the outer key cursor or publish aggregate freshness. Pending
pages, source-generation changes, and bounded work-budget exhaustion use a 15-second follow-up;
priority and SQLite pressure preserve their existing eligibility/defer path. An actual disable
clears queued and deferred scheduler state, and a new enable generation supplies one wake. Ordinary
business events and repeated same-value control requests do not erase an unexpired retry deadline.

The existing migration progress table, unavailable read contract, marker names, queue, and aggregate
responses do not change. A dedicated operator status surface adds GET/PATCH
/api/system/prompt-cache/materialization. Its request and response fields remain unchanged; both
the dedicated PATCH and managed-task PATCH update the same maintenance-database control. The legacy
business-database `startup_backfill_progress.enabled` value is retained for compatibility and may
seed a newly created managed control once; the run path never consults it, and the task never
mirrors it.

The prompt-cache migration progress row stores a fixed source total and a committed identity-key
counter once the identity snapshot is captured. Statistics cursor commits do not add those keys
again. A bounded prompt_cache_conversation_materialization_runs table retains the latest 100 task
calls, including phase, duration, processed/updated counts, batch metrics, and defer or failure
reason. The UI exposes the latest 10 records; incomplete phases stay below 100% and zero ETA is
reported only in the complete phase. Status reads use durable counters and bounded queue sampling,
never a historical table scan. Schema setup repairs counters for older complete rows.

## Consequences

- Foreground writes can preempt between prompt-cache transactions without rolling back a batch
  that already began.
- Foreground aggregate readers get a bounded scheduling opportunity between adaptive commits,
  keeping the representative read p95/p99 within the fixed-400 baseline while retaining shorter
  write transactions.
- Statistics writeback remains proportional to the number of micro-batches, not the number of
  conversation keys, so adaptive batches do not pay an avoidable per-key statement overhead.
- Quiet SQLite databases can recover close to the 400-key throughput ceiling, while high-latency
  transactions reduce lock duration on the next batch.
- Adaptive size is intentionally process-local; durable correctness remains in phase, cursor,
  markers, and the refresh queue.
- Aggregate reads remain fail-closed until identity coverage, statistics, complete phase, and an
  empty queue are simultaneously true.
- The representative 4,000-key/40,000-invocation fixture is an empirical acceptance aid, not a
  production timing guarantee; operators should use the structured batch and defer fields when
  assessing a rollout.

## References

- `docs/adr/0021-prompt-cache-background-materialization.md`
- `docs/specs/proxy-invocation-identity/IMPLEMENTATION.md`
- `src/prompt_cache_conversations.rs`
- `src/maintenance/startup_backfill.rs`
