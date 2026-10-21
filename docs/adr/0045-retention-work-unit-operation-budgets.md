# Retention Work Units and Operation Budgets

Status: Accepted

## Context

Retention previously treated a fixed total task budget as a production boundary. That boundary could expire during monthly file preparation, Summary proof, or source conversion, producing repeated re-preparation and one-row adaptive batches even when each individual operation had a safe boundary. The same task also had two paths that could persist the same Summary pages: the monthly writer and the later proof preparation.

## Decision

- Production retention has no fixed total work duration, default, or replacement upper bound. A dispatch selects a bounded live-row candidate window and owns it as one complete logical work unit. The existing candidate target and byte limits remain unchanged; a later dispatch reselects remaining live rows rather than receiving hidden follow-up work from the current task.
- Operation-level boundaries remain authoritative: short SQLite transactions, the 200 ms write target and 250 ms warning, the 64-row/1 MiB adaptive limits, the 2-second SQLite maintenance query budget, configured raw catch-up budgets, pressure/admission deferral, shutdown cancellation, and native SQLite worker drain before releasing resources.
- The monthly writer scans and persists each V2 Summary page exactly once, using one short main-database transaction per page. Summary proof preparation stages and commits only the manifest, then validates the already durable page set, archive identity, and source identity before persisting the final proof.
- Existing published-artifact and proof reuse remains available only as crash/restart recovery evidence after all identity checks. It is not a normal scheduling continuation mechanism and never resumes source candidate IDs, cursors, temporary files, or deletion transactions.
- Adaptive recovery may lower a stale bytes-per-row estimate after a successful normal commit following an oversized-row breach. The real byte and row limits still apply to every micro-batch; this does not enlarge batches or relax lock priority.
- HTTP, SSE, JSON field names, archive layout/format, schema, journal, and call ID contracts are unchanged. The existing optional `budgetMs` and `timeoutCount` fields remain present and are null for a normal non-test run.

## Consequences

A selected 797-row monthly work unit can finish in one dispatch even when file preparation, two Summary page commits, proof, and source deletion take longer than the former calibration. Cancellation or an operation-level stop still leaves live rows and durable recovery evidence under the existing cleanup contract. Phase duration, lock wait, commit counts, and stop reasons remain observable without presenting a synthetic total budget.

The previous fixed-budget tests remain useful as test-only deadline and cancellation seams for recovery and native cleanup. They do not define production scheduling behavior. The archive-file-io regression must verify source, manifest, Summary page, proof, and deletion counts against the same real SQLite/file work unit, while performance and contention acceptance remain CI-owned.

## References

- [Bounded retention runs](../specs/bounded-retention-runs/SPEC.md)
- [Retention archive](../specs/9aucy-db-retention-archive/SPEC.md)
- [Monthly Summary page checkpoints](0042-monthly-summary-page-checkpoints.md)
- [Reuse proven monthly archive after budget stop](0043-reuse-proven-monthly-archive-after-budget-stop.md)
- `src/maintenance/retention/maintenance_loop.rs`
- `src/maintenance/retention/task_batches.rs`
- `src/maintenance/retention/batch_plan.rs`
- `src/maintenance/archive/writers.rs`
