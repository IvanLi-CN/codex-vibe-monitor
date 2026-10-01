# Retention Core and Conversation Derived Maintenance Have Separate Completion Boundaries

Status: Accepted

Invocation retention commits Verified Archives and source-row transitions without synchronously rebuilding prompt-cache conversation aggregates or scanning for orphan conversations. Affected conversation keys are durably invalidated and queued in the same source transaction; the existing materialization owner refreshes them, while orphan cleanup advances an independently recoverable, exact-reference cursor. This prevents a derived-data query from holding the archive publication write transaction for minutes while preserving archive proof, statistics correctness, and active-conversation safety.

A retention run reports core completion and per-stage completion separately. Its overall completion is partial whenever work in its captured scope remains pending or a recoverable stage fails, even if archive publication has completed; a later background refresh does not rewrite that historical result. Each run has a 60-second admission/work budget and reaches a safe committed boundary when that budget expires. Prompt-cache query pages have a 2-second execution budget with verified SQLite cancellation and rollback, rather than relying on a row LIMIT or merely abandoning an asynchronous future.

Correctness cursors and refresh invalidation remain authoritative in the main database; task progress and run summaries are asynchronously mirrored to the maintenance database under ADR 0023. Moving the aggregate refresh preserves the existing statistics-read contract in ADR 0021; it does not authorize stale aggregate reads. Durable partial-work semantics require forward repair after a stopped release and do not promise that an older binary will preserve the new continuation state.

## References

- [Background prompt-cache materialization](0021-prompt-cache-background-materialization.md)
- [Adaptive prompt-cache materialization](0022-prompt-cache-adaptive-materialization.md)
- [Task operations outside the main database](0023-task-operations-state-outside-main-database.md)
