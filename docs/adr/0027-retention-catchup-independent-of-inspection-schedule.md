# Retention Catch-up Continues Independently of the Inspection Schedule

Status: Accepted

An enabled retention task automatically continues bounded recovery rounds while eligible archival work remains. The default hourly interval or administrator override controls inspection opportunities rather than limiting recovery to those occurrences; disabling the task stops admission of subsequent recovery work at a safe committed boundary. This prevents an hourly partial result from making a large backlog effectively permanent while preserving explicit operator control.

Inspection, manual triggers, and catch-up share one execution owner and single-instance boundary. Each round retains the 60-second work budget, short transactions, verified archive publication, and foreground-request priority; it yields between rounds and resumes after a reason-specific pressure backoff. Fair access to background capacity must prevent starvation without bypassing database-pressure protection. The operator can distinguish the next inspection from the next eligible recovery attempt.

## References

- [Autonomous retention recovery](../specs/autonomous-retention-recovery/SPEC.md)
- [Bounded retention runs](../specs/bounded-retention-runs/SPEC.md)
- [Coordinated SQLite write admission](0015-coordinated-runtime-sqlite-write-admission.md)
- [Effective schedules](0024-task-runtime-observation-and-effective-schedules.md)
- [Retention core and derived maintenance](0025-retention-core-and-conversation-derived-maintenance.md)
