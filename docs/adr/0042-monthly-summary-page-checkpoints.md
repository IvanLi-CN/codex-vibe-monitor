# Monthly Summary Page Checkpoints for Task-Local Archives

Status: Accepted

## Decision

The monthly invocation archive keeps the existing canonical gzip SQLite file, `archive_batches` manifest, `summary_archive_snapshot` V2 pages, and final proof contract. No new schema object, archive layout, page format, or retention budget is introduced.

This decision narrows the continuation prohibition in ADR 0032: unfinished task files, source candidate IDs, cursors, and deletion transactions remain non-resumable, while already committed V2 Summary pages are explicitly progress evidence that may be reused by manifest identity.

After the prepared gzip digest is known, a task-local invocation archive enters the existing `materializing` manifest state. For an existing final file, the staging transaction preserves the predecessor SHA and row count until the atomic replacement transaction commits; a stopped run before rename therefore remains retryable. A stale retry may never demote an already `completed` manifest, and finalization is idempotent for that state. The replacement continues to use the existing archive fence, rollback link, rename, sync, and recovery journal.

Once the final artifact is published, the writer scans the task SQLite copy in deterministic `(julianday(occurred_at), id)` order and commits each V2 Summary page in its own short main-database transaction. A page is progress evidence only. Cleanup authority and source deletion remain blocked until the existing complete semantic proof validates every page, the manifest identity, and the source identity.

On retry, the task reselects live source rows and rebuilds the task file. It may reuse only a contiguous, valid V2 page prefix whose `archive_batch_id` and manifest SHA match the rebuilt artifact, whose IDs and coverage exactly match the archive's deterministic keyset pages, and whose payload re-encodes byte-for-byte from the current archive rows. Temporary SQLite files, source candidate IDs, and source-deletion transactions are never resumed. Missing, gapped, stale-format, or corrupt pages are deleted for that exact manifest identity only if their observed page fingerprints are unchanged; a competing repair causes the current attempt to defer and retry.

## Invariants

- A `materializing` manifest is never an authoritative Summary source, regardless of how many pages are present.
- Page transactions verify the batch file path and SHA before writing and have no deletion authority.
- A file publication failure leaves live source rows and the predecessor artifact recoverable; a page or proof failure leaves the published artifact non-authoritative and retryable.
- The existing 120-second retention work budget remains unchanged; page commits create safe progress within that budget rather than extending it.

## Consequences

The managed-task page can observe durable manifest and Summary page progress even when a monthly archive scan crosses a run boundary. A restarted run still repeats source selection and file preparation, but it does not repeat already committed Summary pages for the same artifact identity. Existing completed archives and V2 readers remain compatible because the persisted tables and final proof semantics are unchanged.

## References

- [Bounded retention runs](../specs/bounded-retention-runs/SPEC.md)
- [Task-local persistent-state record](../specs/bounded-retention-runs/assets/task-local-persistent-state-record.json)
- `src/maintenance/archive/writers.rs`
- `src/maintenance/retention/batch_plan.rs`
- `src/maintenance/retention/task_batches.rs`
