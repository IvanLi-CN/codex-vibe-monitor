# Prompt-Cache Queue Events Preempt Idle Deadlines

Status: Accepted

## Context

Prompt-cache terminal writes enqueue statistics refresh keys and signal the background materialization task. The task also stores ordinary continuation and idle deadlines in both the scheduler and the maintenance checkpoint. Treating every future deadline as an unbreakable retry boundary can leave a newly queued key waiting behind the normal six-hour idle interval.

The scheduler already distinguishes an active database-pressure defer from ordinary scheduled work. The materialization control also has a generation fence and a single pending wake set, so an event wake can be admitted without bypassing control or creating a second task signal.

## Decision

A prompt-cache statistics queue event preempts an unexpired ordinary idle or continuation deadline. The wake clears the durable `next_run_after` and schedules the task at the current time. An active database-pressure defer retains its future retry deadline and is not cleared by an event. A cooperative coordinator-priority yield uses an ordinary retry deadline, so a later queue event may preempt it.

The wake remains subject to the maintenance `managed_tasks` enablement check, the current control generation, and the database-pressure gate when the task is dispatched. Concurrent or repeated events coalesce behind the scheduler's existing per-task pending wake entry; only the first event advances the checkpoint wake generation and schedules work.

Final checkpoint writes compare the durable wake generation captured when the run started. If a queue event advanced that generation while the run was processing, finalization keeps `next_run_after` cleared under the same control lock, so a late run result cannot restore an ordinary delay over the pending event.

## Consequences

- Newly enqueued statistics work is eligible immediately even when the task was idle-scheduled for a later time.
- An event that overlaps an in-flight materialization remains immediately eligible after that run's checkpoint is finalized.
- Database-pressure backoff remains authoritative while the process retains the pressure-defer state; dispatch still rechecks pressure before opening task work. Coordinator-priority retry state remains event-preemptible.
- No new durable column or event-priority state is required. A process restart may recover a future checkpoint deadline without its in-memory pressure marker; the normal pressure gate remains the final admission boundary.
- Existing disablement, generation fencing, bounded continuation, and complete-marker publication semantics remain unchanged.

## Considered Options

- Add a durable force-wake marker alongside `next_run_after`: rejected because it duplicates the existing scheduler wake state and introduces another crash-recovery state to reconcile.
- Clear every future deadline on queue events: rejected because it would erase an active database-pressure retry boundary and could turn sustained database pressure into repeated immediate admissions. Cooperative coordinator-priority retries are deliberately outside that boundary.
