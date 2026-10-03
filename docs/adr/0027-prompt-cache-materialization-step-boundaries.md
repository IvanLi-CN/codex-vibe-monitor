# Prompt-Cache Materialization Uses Committed Identity and Statistics Steps

Status: Accepted

This succeeds the combined identity/statistics transaction and set-based writeback portions of
ADRs 0021 and 0022. Their ordered phases, adaptive identity batch sizes, priority boundaries,
and fail-closed aggregate read contract remain applicable.

The maintenance database owns the task's committed control. Before each business SQL step, a
short memory gate registers the current control generation; an already registered step may finish
after pause, but a later step cannot start under the old generation. Business transactions never
read or wait for the maintenance database.

Identity discovery commits identities, its key cursor, and identity-key count together. It does
not compute statistics. Statistics rebuild and queue drain own bounded 256-invocation pages:
partial aggregates and their source-generation fence stay in existing staging rows. Only a
generation-consistent final page publishes an aggregate and clears that generation's queue item;
the outer statistics-key cursor advances only after all required keys are complete. A crash between
those commits repeats idempotent work without skipping identities or publishing partial statistics.
This trades one combined transaction for resumable short steps and avoids scanning large keys twice.

Completed logical pages and empty phase transitions may continue within the same existing scan
and elapsed-time budgets. An unfinished statistics page, changed source generation, or exhausted
budget still uses the existing 15-second follow-up. Priority and database pressure retain their
eligibility/deadline path; actual disablement waits for a new enable generation. No table, column,
HTTP field, or read-completeness condition changes.

Maintenance checkpoint start/finalization and wake writes use a separate asynchronous fence
shared with control commits. Each writer rechecks its generation after acquiring the fence.
The fence does not span business SQL, and online write admission is released before waiting
for maintenance state. Scheduler changes check the same published generation so a late disable
result cannot erase a later resume wake.

## References

- [Historical materialization](0021-prompt-cache-background-materialization.md)
- [Adaptive materialization](0022-prompt-cache-adaptive-materialization.md)
- [Task state outside the main database](0023-task-operations-state-outside-main-database.md)
- [Invocation identity requirements](../specs/proxy-invocation-identity/SPEC.md)
