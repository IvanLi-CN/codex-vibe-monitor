# Retention Maintenance Has Four Explicit Task Owners

Status: Accepted

Retention currently combines archive work and orphan-conversation cleanup in one execution, observes independently owned Prompt-cache materialization as a retention stage, and attributes a separate raw orphan worker to the retention timeline. The owner confirmed four independently managed responsibilities so completion, failure, and resource consumption can be attributed to the work that actually owns them. Automatic triggering is controlled per task; explicit manual execution remains available when automatic triggers are paused. The confirmed requirement set is recorded in the [ownership Spec](../specs/retention-maintenance-task-ownership/SPEC.md).

## Agreed Responsibility Split

| Responsibility                     | Task ownership                                                                        | Completion scope                                                                                                                           |
| ---------------------------------- | ------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| Data retention and archival        | Keep `retention_archive` and its existing task route.                                 | Archive publication, source transitions, detail pruning, cold raw compression, archive expiry, and the existing retention-policy janitors. |
| Prompt-cache materialization       | Reuse `prompt_cache_materialization`; do not introduce another materialization owner. | Its existing identity, aggregate, and projection work, including refreshes durably queued by retention.                                    |
| Orphan invocation identity cleanup | Extract the existing cleanup from the retention execution into a managed task.        | Reference- and occupancy-safe release of eligible conversation identities and ended, unreferenced hourly invocation prefixes.              |
| Orphan Raw Payload File Cleanup    | Give the existing independent raw orphan worker a separate managed identity.          | Ownership-safe file removal and the corresponding physical inventory reconciliation.                                                       |

Archive-associated raw-file release remains part of the source transition that permits it. Independent raw orphan cleanup handles files without a remaining owner; it does not take over that atomic archive transition. The existing raw-payload metrics inventory remains a separate responsibility outside this task split.

## Agreed Operator Control

The four task-page controls are independent and are the sole steady-state authority for automatic triggering. The existing enabled/disabled field represents whether automatic triggers are active; it does not grant or revoke permission for an explicit manual execution. Pausing `retention_archive` does not pause either cleanup task or Prompt-cache materialization. Resuming automatic archival does not change the other tasks' control values.

Remove `RETENTION_ENABLED` and its legacy `XY_RETENTION_ENABLED` alias from the supported runtime configuration. Neither is read, validated, rejected, or imported during startup or control initialization. Environment configuration documentation, examples, and the effective-policy display must reflect the managed controls. A retained old variable has no effect, including when its old value was false.

Do not import the previously effective environmental enablement. Existing managed-task control values and administrator schedule overrides remain intact; missing task rows use the new task defaults. Consequently, an archive task whose stored control is enabled may begin automatic work after upgrading even if the retired variable previously suppressed it. The owner explicitly selected this behavior instead of a one-time compatibility import.

Both newly managed cleanup tasks default to enabled on a new installation and when their controls are first introduced into an existing maintenance store. New installations retain the enabled defaults of the archive and materialization tasks. An existing row's disabled value is an operator decision and must not be reset to the enabled default on later starts or upgrades.

Orphan identity cleanup includes both orphan Prompt-cache Conversations and the existing cleanup of ended, unreferenced hourly invocation prefixes. They remain distinct measured populations within the same identity-cleanup responsibility; the split must not leave hourly prefix cleanup behind in the archive execution.

## Agreed Cleanup Scheduling

Both cleanup tasks automatically continue bounded batches while the current scan still has candidates to inspect or committed recovery work to settle. Between progressing batches they yield; once the scan is exhausted they return to a five-minute idle inspection interval. A scan's inspected candidate count is not a measurement of all currently deletable objects, and a protected candidate must not create a busy retry loop.

Pressure, resource admission refusal, and actual operation errors use reason-specific backoff rather than an unconditional continuation. Existing raw quarantine and release-pending recovery facts and failure deadlines survive the task split. Each task exposes the next inspection separately from the next eligible continuation or retry, without promising an actual start time.

Pausing automatic triggers prevents new automatically admitted runs from startup, inspection, event wake, catch-up, or retry. It does not cancel or truncate an already admitted bounded run, roll back completed batches, erase scan/recovery state, or toggle another task. The active run retains its normal budget, scope, pressure rules, and shutdown handling; after it ends, further automatic continuation remains paused. Missed periodic inspections are not queued for burst execution.

Task-page run-now and CLI one-shot requests remain allowed while automatic triggers are paused. A manual request accepted before a pause remains a manual request and is not discarded because the control changed. Manual execution does not resume automatic triggering, and any resulting remaining work waits for a later explicit request or for automatic triggering to resume. Startup, inspection, continuation, and explicit requests share that task's single-instance execution boundary, deletion protections, and resource admission.

## Agreed One-Shot Scope

`--retention-run-once` executes only the archival responsibility and its retained policy janitors. It no longer performs orphan conversation identity cleanup or a raw orphan scan. Its dry-run scope is the same archival responsibility.

The two cleanup owners each provide their own bounded one-shot and dry-run entry point. A task-specific invocation cannot execute, resume automatic triggers for, or consume the budget of another responsibility. Identity cleanup includes its conversation and hourly-prefix populations; raw cleanup uses its existing ownership and recovery protocol.

Dry-run evaluates the selected task's eligible work and reports the inspected scope and incomplete coverage explicitly. It cannot delete source rows, release identities, unlink files, publish quarantine/release changes, or advance the live cleanup correctness cursor; operational observation or necessary structural initialization is distinct from performing the cleanup. A bounded preview is not a promise of a full-store total.

## Presentation and Audit Contract

The task catalog and actual-execution area show four distinct responsibility owners. Each has its own detail route, effective scheduling, operator control, run history, work measurements, and resource-deferral attribution. Related-task links can connect the archive detail page to materialization and cleanup without producing another combined execution or a mixed-unit completion percentage.

New archive progress and history describe the archive owner's work only. They do not present independently owned materialization or identity cleanup as archive stages, and the raw orphan worker's new execution and pressure events belong to the raw cleanup identity. Shared database pressure remains a shared condition, while the observed affected work retains its real task identity.

Old retention records remain under their original task identity and retain their original combined completion, core completion, and detail values. New records explicitly identify their responsibility scope so the UI can explain old combined outcomes without reinterpreting them as the new archive-only outcome. Missing scope evidence stays legacy or unknown; no synthetic cleanup history, zero metrics, or revised historical completion is created. The old archive URL continues to resolve to the archive owner and can display its original history.

## Required Boundaries

- Each responsibility has its own execution observation, run history, work counters, completion outcome, failure reason, and retry state. A different task's pending work or failure cannot change its completed run result.
- Orphan conversation cleanup has its own work budget and admission boundary; it cannot consume the archive run's remaining budget or terminate that run with a cleanup failure. The existing raw worker retains one execution owner when it becomes managed; adding a task entry must not create a second worker.
- Invocation mutations, statistics invalidation, and durable refresh enqueue remain in the same business transaction. The existing materialization owner consumes committed refresh work under its own controls and exact-read contract.
- Conversation deletion continues to check both key references and Conversation ID references, active occupancy, and existing retention protections. Raw deletion continues to require its existing ownership proof and file-lock rules. Task separation does not authorize earlier or less thoroughly proven deletion.
- Archive publication proofs, source identity verification, monthly archive targets, task-local batch closure, retention periods, and foreground-write priority remain governed by their existing contracts.
- Progress retains task-specific units. Invocation rows, conversation identities, files, and bytes cannot be combined into one total or completion percentage. Unknown population or missing observation remains unknown.
- Business correctness state remains authoritative in the main store; operational configuration, progress, and history retain the existing maintenance-store ownership. Observation failure cannot add synchronous fallback writes to the business store.

## Verification Expectations

- Disabling archival while both cleanup controls remain enabled leaves the cleanup owners eligible; disabling either cleanup does not change archival, materialization, or the other cleanup control.
- Both cleanup owners begin enabled when first introduced. Repeated startup and interrupted initialization preserve previously saved task controls and administrator overrides.
- Setting either retired environment variable to false, true, or an otherwise invalid value does not gate work, affect initialization, or cause a configuration parsing error.
- A progressing scan continues bounded work without waiting for the idle interval; an exhausted scan returns to five-minute inspection. Pressure and actual failures retain their respective backoff and recovery facts.
- A simultaneous automatic and explicit trigger cannot create two executions of the same owner. Pausing automatic triggers permits the active bounded run to finish its normal scope, prevents later automatic runs, and still admits an explicit manual run without changing the paused control.
- Completed archive work remains completed when materialization has pending refreshes or identity/file cleanup is deferred or fails. Each owner's committed work, failure, and timing remain attributable to that owner.
- A conversation or hourly prefix that gains a reference or active occupancy before deletion remains retained. A raw file with a valid owner or changed identity remains retained, including during archive publication or interrupted release recovery.
- Measurement and history preserve their own units and coverage; missing historical evidence is never converted into a zero or an invented independently measured cleanup run.

## Decision Review

The owner confirmed the assembled requirement set, including ownership, automatic-trigger controls, defaults, scheduling, entry-point scopes, presentation, and historical audit compatibility. Requirement confirmation does not authorize implementation or delivery work.

## References

- [Domain terms](../../CONTEXT.md#autonomous-retention-recovery)
- [Retention core and conversation-derived maintenance](0025-retention-core-and-conversation-derived-maintenance.md)
- [Task operations outside the main database](0023-task-operations-state-outside-main-database.md)
- [Task runtime observation and effective schedules](0024-task-runtime-observation-and-effective-schedules.md)
- [Retention task-local batches and monthly targets](0032-retention-task-local-batches-and-monthly-archive-targets.md)
- [Bounded retention runs](../specs/bounded-retention-runs/SPEC.md)
- [Planned public and persistent-state impact](assets/retention-maintenance-task-ownership/version-impact-record.json)
- [Planned persistent-state initialization and recovery](assets/retention-maintenance-task-ownership/persistent-state-migration-record.json)
