# SQLite contention boundaries and ordered Summary publication

Status: Accepted

## Context

The service keeps SQLite as the durable business-fact source, but several independent runtime paths were competing for the same small connection pool and single-writer admission. The managed-task dispatcher opened a write transaction even when no task was due or requested. Startup metadata backfill evaluated JSON predicates over the full invocation table, and the events history query repeatedly searched work rows without a managed-run index. Invocation timeline requests performed snapshot DDL and a count query on the request path, while snapshot publication used one write transaction for every page. Summary terminal batches could commit a higher dashboard sequence before an earlier sequence, causing the in-memory proof guard to report a sticky gap even when the database was within capacity. Summary archive-snapshot promotion and backfill candidates also evaluated proof, obligation, and outcome predicates before establishing a bounded archive identity window.

These paths must be improved without weakening exact Summary semantics, changing public HTTP/SSE contracts, expanding the SQLite pool, or repeating the separate retention archive-progress repair. Invocation range demand priority is already defined by ADR 0043 and is consumed here as an integration slice rather than redesigned.

## Decision

The managed-task dispatcher performs a read-only due/requested probe before opening a write transaction. An idle cycle, including a due task with an active requested/running instance, does not issue a no-op `managed_task_runs` update or commit a write transaction. When a candidate exists, the existing conditional claim and enablement-generation checks still run in a short write transaction, with the existing retry and ownership semantics preserved.

Startup metadata backfill first reads fixed primary-key windows and parses the payload in Rust. Only rows whose requested field is still missing reach the existing conditional update, so JSON/TRIM predicates no longer determine the full-table candidate set. Pool-attempt identity repair uses the same window-and-conditional-update rule, preserving source predicates and the existing cursor/ retry behavior.

The events history path has indexes for the managed-run anti-join and its `COALESCE(actual_started_at, started_at), id` ordering. The query still returns at most 200 rows with the same status filter and fields; the indexes are initialized idempotently with the maintenance schema.

Invocation timeline requests capture one logical `asOf` watermark/fence before paging. Each bounded page may use its own short read transaction and must apply the same captured bounds; no source read transaction or connection is held across the full traversal. Account hydration is batched for the page, with the existing invalid-row fallback retained. Snapshot storage DDL is initialized with the application schema, not on the request path. The in-memory snapshot row count is reused for response totals, and durable publication commits existing-size row batches independently. The token is published only after every batch succeeds; the existing cleanup guard removes partial rows on failure.

Summary terminal publication acknowledges only a continuous committed sequence prefix. A committed sequence above the next expected predecessor stays in a bounded in-memory reorder buffer. When the predecessor arrives, the publisher advances the prefix and acknowledges all newly contiguous entries. Duplicate, rollback, real gap, conflict, and capacity outcomes retain their existing idempotent or fail-closed behavior; a higher committed sequence is not itself proof of loss.

Summary archive-snapshot promotion and backfill candidates use two phases. Phase one reads a fixed identity window from the existing `archive_batches` seek index. Phase two joins those identities through a bounded `VALUES` relation and evaluates snapshot, proof, obligation, outcome, format, and cleanup-authority predicates in one query. Manifest identity and continuous-prefix confirmation remain unchanged. No new Summary index or schema object is introduced; if the existing keys cannot demonstrate a bounded plan, the implementation returns a scoped design blocker instead of silently accepting an unbounded scan.

The current invocation-range demand-priority implementation and its tests remain part of the parent candidate. It retains the 64-sequence range contract, commit-before-issue publication, the 100 ms empty-range wait, and durable uniqueness. No per-invocation SQL is added.

## Consequences

Idle maintenance cycles stop consuming write admission, startup discovery and events history become bounded by primary-key/index windows, and timeline reads release their source connection between pages without request-time DDL or count scans. The same logical watermarks, proof rules, and public response fields continue to define correctness. Out-of-order terminal completion is temporarily retained in memory instead of being misclassified as a permanent gap; genuine missing or over-capacity conditions remain visible and fail closed. Candidate-query correctness is guarded by query-plan evidence, and a plan that cannot be proven bounded stops this repair rather than expanding scope. Retention archive preparation, deletion proof, archive format, and archive-codec cleanup remain outside this decision.

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
