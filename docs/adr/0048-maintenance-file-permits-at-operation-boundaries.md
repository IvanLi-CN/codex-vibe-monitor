# Maintenance File Permits at Operation Boundaries

Status: Accepted

Holding a single heavy-file permit for an entire maintenance owner run serializes database work, pressure cooldowns, and final progress publication with unrelated file work. Heavy file work is mutually exclusive only during actual file-operation phases while each task retains its execution lease and selected logical work unit for its full lifetime; this permits unrelated owners to progress without allowing concurrent heavy file operations.

Release is allowed only after native file work has drained. Before waiting to acquire another file permit, the task must release database connections, write permits, and file fences. Reacquisition must revalidate the artifact and source identities required by the existing proof protocol; a timer cannot release a permit while native work remains active. Pressure retries stay within the admitted run, and this decision does not introduce cross-run staging, a fixed total task timeout, larger write transactions, or weaker foreground priority.

Main-database source loading must finish before queuing for a file phase. Archive copying uses the existing bounded selection, and archive proof connections must close or drain before returning to main-database work. Publication retains its established rename, file/directory sync, and manifest-commit durability order, including the existing fence through commit or compensation; the heavy-file permit need not cover the main-database commit. A publication that already owns transaction resources cannot wait for a new file permit: it must acquire without waiting or retain recovery evidence and drain/release resources before retrying. Nested helpers share a real owned permit, and a native worker retains that ownership until it exits even if its async caller is cancelled.

Whole-run serialization is simpler but lets database or retry delays block unrelated file owners. Phase-scoped admission requires explicit lifetime and cancellation boundaries and may leave multiple bounded tasks alive concurrently; the dispatcher must retain its worker/finalizer limits, single-instance task leases, and authoritative waiting observations.

This decision refines the resource-ownership boundary in [ADR 0034](0034-retention-maintenance-task-ownership.md) while retaining the selected work-unit and operation-budget rules in [ADR 0045](0045-retention-work-unit-operation-budgets.md). Scheduler waiting before task admission remains separate from task execution; file-resource waits after execution begins belong to the executing task.
