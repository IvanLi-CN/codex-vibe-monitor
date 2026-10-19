# SQLite contention boundaries and ordered Summary publication

Status: Accepted

## Context

The service keeps SQLite as the durable business-fact source, but several independent runtime paths were competing for the same small connection pool and single-writer admission. The managed-task dispatcher opened a write transaction even when no task was due or requested. Invocation timeline materialization held a source read transaction across all pages and account hydration. Summary terminal batches could commit a higher dashboard sequence before an earlier sequence, causing the in-memory proof guard to report a sticky gap even when the database was within capacity. Summary archive-snapshot promotion candidates were selected by scanning snapshot pages before checking the authoritative archive batch set.

These paths must be improved without weakening exact Summary semantics, changing public HTTP/SSE contracts, expanding the SQLite pool, or repeating the separate retention archive-progress repair. Invocation range demand priority is already defined by ADR 0043 and is consumed here as an integration slice rather than redesigned.

## Decision

The managed-task dispatcher performs a read-only due/requested probe before opening a write transaction. An idle cycle does not issue a no-op `managed_task_runs` update or commit a write transaction. When a candidate exists, the existing conditional claim and enablement-generation checks still run in a short write transaction, with the existing retry and ownership semantics preserved.

Invocation timeline requests capture one logical `asOf` watermark/fence before paging. Each bounded page may use its own short read transaction and must apply the same captured bounds; no source read transaction or connection is held across the full traversal. Account hydration is batched for the page, with the existing invalid-row fallback retained. The existing durable snapshot publication and HTTP response shape remain authoritative, and the publish transaction contains only the bounded projection exchange.

Summary terminal publication acknowledges only a continuous committed sequence prefix. A committed sequence above the next expected predecessor stays in a bounded in-memory reorder buffer. When the predecessor arrives, the publisher advances the prefix and acknowledges all newly contiguous entries. Duplicate, rollback, real gap, conflict, and capacity outcomes retain their existing idempotent or fail-closed behavior; a higher committed sequence is not itself proof of loss.

Summary archive-snapshot promotion candidates are driven from the existing `archive_batches` identity/order and use bounded `EXISTS` checks for snapshot, proof, and outcome state. The query must preserve manifest identity, format, proof, and cleanup-authority predicates. No new index or schema object is introduced; if the existing keys cannot demonstrate a bounded plan, the implementation returns a scoped design blocker instead of silently accepting an unbounded scan.

The current invocation-range demand-priority implementation and its tests remain part of the parent candidate. It retains the 64-sequence range contract, commit-before-issue publication, the 100 ms empty-range wait, and durable uniqueness. No per-invocation SQL is added.

## Consequences

Idle maintenance cycles stop consuming write admission, and long timeline reads release their source connection between pages. The same logical watermarks, proof rules, and public response fields continue to define correctness. Out-of-order terminal completion is temporarily retained in memory instead of being misclassified as a permanent gap; genuine missing or over-capacity conditions remain visible and fail closed. Candidate-query correctness is guarded by query-plan evidence, and a plan that cannot be proven bounded stops this repair rather than expanding scope.

The parent candidate must report no-op writes, page-read duration, reorder depth, candidate-query outcome, and invocation-range wait outcomes through fixed existing observability contracts. Production comparison remains read-only and is not replaced by local or Agent VM pressure testing.

## References

- [ADR 0040: Bounded maintenance contention](./0040-bounded-maintenance-contention.md)
- [ADR 0043: Bounded invocation range preparation and demand priority](./0043-invocation-range-preparation-priority.md)
- [Dashboard invocation timeline Spec](../specs/dashboard-invocation-timeline/SPEC.md)
- [Runtime read-model pressure recovery Spec](../specs/runtime-read-model-pressure-recovery/SPEC.md)
- `src/maintenance_store.rs`
- `src/api/slices/invocation_timeline.rs`
- `src/api/slices/subscriptions/state.rs`
- `src/maintenance/archive/cleanup/summary_snapshot.rs`
