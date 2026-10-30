# Module-owned PostgreSQL migration

## Context and Scope

Online migration moves functional modules from the existing SQLite main database to PostgreSQL while retaining one authoritative backend for each table. The module boundary covers its transaction partners, durable authority and recovery, bounded background migration, and task admission during migration. Maintenance and performance databases, archive files, independent backup ownership and unrelated service features retain their existing owners.

## Terms and Interfaces

- A module is a closed transaction boundary whose tables change authority together.
- An authority generation identifies the backend and generation used by a module transaction; the durable authority manifest is the recovery source of truth.
- A capture watermark is a proven complete source-change prefix, not merely the largest observed event ID.
- The migration catalog, control/status interfaces and acceptance reports expose migration progress without disclosing credentials or raw business payloads.

## Requirements

### REQ-PGMIG-001

- The service MUST prepare and copy module data in bounded background work after basic readiness, without an offline copy or a separate preliminary application deployment. Prepared target data MUST NOT serve normal business operations before authority changes.

### REQ-PGMIG-002

- Each transaction MUST use one module backend and authority generation. Tables participating in one transaction MUST move together unless their coupling has first been replaced by durable, idempotent obligations; cross-backend transactions MUST NOT silently split into separate commits. Cross-module account observations MUST use durable idempotent delivery and control-generation fences so stale observations cannot override newer control decisions.

### REQ-PGMIG-003

- Source mutations MUST be captured transactionally before baseline copy, including inserts, updates, deletes, key changes and replacements. Replay and its target checkpoint MUST commit together; acknowledgement MUST follow the target commit and cover only the proven complete prefix. Recovery and retries MUST be idempotent and preserve newer changes. If capture continuity is lost, a new capture generation and baseline MUST be established before the module can again become eligible for cutover.

### REQ-PGMIG-004

- After bounded copy, catch-up and verification, switching MUST gate new module operations, drain old transactions, fence source writers and durably publish one authority generation. A pre-publication timeout MUST safely retain or reopen SQLite authority; an uncertain publication MUST remain in recovery until durable authority is resolved. A confirmed PostgreSQL authority MUST NOT silently fall back to stale SQLite.

### REQ-PGMIG-005

- Migration admission MUST pause competing heavy work at safe checkpoints without overwriting operator enablement or schedules. Required proxy, authentication, routing, journals, recovery, capacity protection and migration progress MUST remain available under their existing contracts. Restoration MUST honor current operator settings and module authority.

### REQ-PGMIG-006

- Source, target and authority identities and durable checkpoints MUST support restart and forward recovery. Unsupported older writers MUST fail closed after a migrated authority; backup restoration is a separate disaster-recovery operation. Before production cutover, target capacity, WAL backlog and data-volume headroom, backup/restore and a natural-traffic baseline MUST be independently verified.

### REQ-PGMIG-007

- Acceptance reports MUST bind results to the tested candidate and base, preserve relevant raw evidence and distinguish passed, failed, blocked, not-run and unverified coverage. Pressure tests MUST NOT be run under the current owner restriction and are not acceptance requirements. A performance-testing workflow MUST finish in strictly less than three minutes overall; longer workflows MUST NOT be an acceptance condition.

## Verification

### VER-PGMIG-001

- Method: inspect startup and migration execution boundaries and exercise bounded background preparation.
- covers: `REQ-PGMIG-001`
- Pass condition: target preparation remains private, startup does not perform historical migration work, and copy work respects configured bounds.

### VER-PGMIG-002

- Method: exercise module transactions and authority transitions, including unsupported mixed-backend operations.
- covers: `REQ-PGMIG-002`, `REQ-PGMIG-004`
- Pass condition: one generation owns each transaction, stale observations cannot override newer control generations, publication is durable, and timeout or uncertain publication cannot admit conflicting authority.

### VER-PGMIG-003

- Method: use source/target fixtures and deterministic commit, acknowledgement and restart faults.
- covers: `REQ-PGMIG-003`, `REQ-PGMIG-006`
- Pass condition: committed changes and complete-prefix checkpoints survive retry without loss, duplicate effect, stale overwrite or resurrection; lost capture continuity blocks cutover until a new generation and baseline are established; stale writers are rejected.

### VER-PGMIG-004

- Method: inspect task-admission, recovery and deployment evidence at the relevant boundaries.
- covers: `REQ-PGMIG-005`, `REQ-PGMIG-006`
- Pass condition: essential work retains its contract, restored tasks respect current operator settings, and production capacity, WAL, backup/restore and baseline evidence is independently verified before cutover.

### VER-PGMIG-005

- Method: inspect candidate-bound reports and workflow configuration.
- covers: `REQ-PGMIG-007`
- Pass condition: missing or unverified evidence is not reported as passed, prohibited pressure tests are not run, and any performance-testing workflow has an overall runtime strictly below three minutes.

## Related ADRs

- [Module-owned PostgreSQL migration](../../adr/0050-module-owned-postgresql-migration.md)

## References

- [Acceptance contract](./ACCEPTANCE.md)
- [Implementation coverage](./IMPLEMENTATION.md)
- [Topic history](./HISTORY.md)
- [Migration design and recovery protocol](../../design/postgresql-module-migration.md)
