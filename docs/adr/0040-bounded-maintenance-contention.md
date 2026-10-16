# Bounded maintenance contention

Status: Accepted

## Context

File verification and cleanup can occupy the main SQLite writer admission even when no database transaction needs them. Repeated backfill wake notifications can independently contend on the maintenance database. A faster response is not evidence of improvement if work has stopped or exact read-model coverage has been weakened.

## Decision

Retention prepares and fully verifies the archive artifact before taking the write admission. The verified file identity remains fenced through the source-proof and delete transaction. Waiting for a background slot holds neither a pool connection nor a file fence: after such a wait the artifact is fenced and verified again before admission. Native SQLite work is completed or rolled back and drained before admission is released; post-commit file cleanup follows release. Source validation, publication proof, rollup and dirty markers, and deletion remain atomic.

Backfill notifications become bounded in-memory obligations keyed by fixed task and control generation. The existing dirty/progress records remain the durable source of recovery work; no new persistent notification queue is introduced. A successful terminal commit does not wait for the maintenance database. Failed wake confirmation retains its obligation and original age, with independent capped backoff and rate-limited warnings. Duplicate notifications do not make a failed task eligible early. Control changes fence obsolete work before and after confirmation.

Only two optional performance indexes are prepared by an explicit ownership-aware maintenance command. Index preparation is separate from application startup and from historical materialization. Missing indexes retain correct existing queries. A cancelled or expired operation is interrupted, rolled back and drained before its ownership proof is released; an earlier completed index is retained for idempotent re-entry. Application rollback does not remove indexes or reverse committed recovery progress.

## Consequences

The service continues to recover from durable business and progress state after losing memory. The implementation must prove that each coalesced notification is either covered by committed progress or retained until confirmation; queue bounds cannot silently drop an obligation. Index installation may need a separately approved maintenance window at production scale. Existing exact Summary publication and live-row MAX semantics remain authoritative. Collection, completed work and backlog are evaluated together using the unchanged contention metric contract.

See [maintenance boundaries](../design/maintenance-contention-boundaries.md) for interfaces and budgets. This decision refines [coordinated admission](0015-coordinated-runtime-sqlite-write-admission.md), [task-state ownership](0023-task-operations-state-outside-main-database.md), and [monthly archive batches](0032-retention-task-local-batches-and-monthly-archive-targets.md); it does not replace their durable fact ownership.
