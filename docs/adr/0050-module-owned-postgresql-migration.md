---
status: accepted
---

# Module-owned PostgreSQL migration

The main SQLite database mixes traffic facts, archive publication, statistics and account controls behind one writer. Migrate functional modules to PostgreSQL while the service runs: copy and reconcile their tables incrementally in the background, then briefly gate the module's database operations and switch the complete module after its final changes are verified. Migration does not require an offline database copy or a separate deployment stage, and historical work must not extend application startup.

A module is a transaction boundary, not merely a list of similarly named tables. Each table has one authority at a time. Tables participating in one business transaction move together unless that transaction is explicitly refactored into durable, idempotent obligations before migration. Traffic and archive correctness are the first priority; account, credential and routing controls may remain in SQLite. Existing maintenance and performance databases and archive files retain their separate ownership.

## Consequences

- Background copying is observable per table, but authority changes atomically per module. Mixed-backend transactions are rejected rather than silently split into two commits.
- A module uses one durable authority generation. A suggested three-second pre-commit switch budget bounds admission and drain attempts; an expired attempt reopens SQLite before retrying. An ambiguous or committed switch is recovered from durable authority, never by assuming that SQLite is still current.
- Competing heavy tasks stop admitting work and yield at safe checkpoints during active migration. Business journals, required authentication and routing, recovery obligations, migration progress and capacity protection continue. Operator enablement and schedules are not overwritten.
- Cross-module account observations use durable idempotent delivery and control-generation fences. They cannot turn a traffic commit into a synchronous two-database transaction.
- An older program cannot safely resume writes after any module has moved. Recovery is forward repair; restoring a backup is separate disaster recovery.

The [requirements and recovery protocol](../design/postgresql-module-migration.md) define the migration contract. [Task state ownership](0023-task-operations-state-outside-main-database.md), [terminal admission](0037-isolated-supplemental-recovery-and-continuous-attempt-reads.md), [journal recovery](0039-terminal-journal-startup-recovery.md), and [bounded maintenance](0040-bounded-maintenance-contention.md) remain correctness constraints.
