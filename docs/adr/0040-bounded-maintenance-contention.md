# Bounded maintenance contention

Status: Accepted

## Context

File verification and cleanup can occupy the main SQLite writer admission even when no database transaction needs them. Repeated backfill wake notifications can independently contend on the maintenance database. A faster response is not evidence of improvement if work has stopped or exact read-model coverage has been weakened.

## Decision

Retention prepares and fully verifies the archive artifact before taking the write admission. The verified file identity remains fenced through the source-proof and delete transaction. Waiting for a background slot holds neither a pool connection nor a file fence: after such a wait the artifact is fenced and verified again before admission. Readiness waiting preserves the original fairness deadline without reserving a writer or consuming a fairness token; the final non-blocking claim rechecks foreground priority and the existing fairness rate limit. A pool-readiness race revokes an unused fairness admission before retrying. Native SQLite work is completed or rolled back and drained before admission is released; post-commit file cleanup follows release. Source validation, publication proof, rollup and dirty markers, and deletion remain atomic.

Monthly replacement keeps the existing recovery journal and atomic rename/sync/manifest sequence. This retains the critical rename and durability sync in the short publication transaction while moving complete hashing and later file cleanup outside admission. An ambiguous commit or failed file compensation preserves recovery references. The native cleanup owner survives cancellation of its waiter; confirmed commit counters are recorded before cleanup can yield. Proof-page staging does not gain deletion authority or require one complete file hash per page.

Backfill notifications become bounded in-memory obligations keyed by fixed task and control generation. The existing dirty/progress records remain the durable source of recovery work; no new persistent notification queue is introduced. A successful terminal commit does not wait for the maintenance database. Failed wake confirmation retains its obligation and original age, with independent capped backoff and rate-limited warnings. Duplicate notifications do not make a failed task eligible early. Control changes fence obsolete work before and after confirmation.

Only two optional performance indexes are prepared by an explicit ownership-aware maintenance command. Index preparation is separate from application startup and from historical materialization. Missing indexes retain correct existing queries. A cancelled or expired operation is interrupted, rolled back and drained before its ownership proof is released; an earlier completed index is retained for idempotent re-entry. Application rollback does not remove indexes or reverse committed recovery progress.

Invocation-range readiness uses bounded speculative preparation under the existing durable reservation protocol. Real missing-range demand takes priority, and speculative memory may be evicted without making committed IDs reusable. Reservation settlement retains native connection ownership through caller cancellation. Preparation does not change the request wait, namespace encoding, committed-before-issuance rule, or recovery obligation fences; a new owner with no committed range still fails within the existing bounded request wait when persistence is unavailable. The runtime recovery Spec records its startup, hourly and refill boundaries separately from index preparation.

## Consequences

The service continues to recover from durable business and progress state after losing memory. The implementation must prove that each coalesced notification is either covered by committed progress or retained until confirmation; queue bounds cannot silently drop an obligation. Index installation may need a separately approved maintenance window at production scale. Existing exact Summary publication and live-row MAX semantics remain authoritative. Collection, completed work and backlog are evaluated together using the unchanged contention metric contract.

See [maintenance boundaries](../design/maintenance-contention-boundaries.md) for interfaces and budgets. This decision refines [coordinated admission](0015-coordinated-runtime-sqlite-write-admission.md), [task-state ownership](0023-task-operations-state-outside-main-database.md), and [monthly archive batches](0032-retention-task-local-batches-and-monthly-archive-targets.md); it does not replace their durable fact ownership.

The [persistent-state record](assets/bounded-maintenance-contention/persistent-state-migration-record.json) and [version-impact record](assets/bounded-maintenance-contention/version-impact-record.json) track separate compatibility contracts. Pending candidate evidence is not release verification.
