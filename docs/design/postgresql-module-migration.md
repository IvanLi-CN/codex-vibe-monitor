# PostgreSQL module migration

This document defines runtime migration from the existing SQLite main database to PostgreSQL. A functional module owns a closed set of tables and changes authority as a unit. Individual tables are copied and verified in bounded background work while the existing service continues serving traffic.

## Requirements

| ID     | Requirement                                                                                                                                                                                                                   |
| ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| MIG-01 | Deploy the migration-capable service once. Do not require an offline database copy, an offline importer, or a preliminary application deployment.                                                                             |
| MIG-02 | Organize migration by functional module. Copy tables incrementally; switch all tables in a module together after its dependencies and correctness checks pass.                                                                |
| MIG-03 | Keep one authoritative backend per table. A target under preparation is private migration state and must not serve ordinary business reads or writes.                                                                         |
| MIG-04 | Application startup reads only a bounded migration catalog and existing mandatory recovery state. Table scans, copying, index builds, checksums, backfills and catch-up run after basic HTTP readiness in background workers. |
| MIG-05 | Pause unnecessary tasks that compete with the active migration. Preserve proxy correctness, durable admission, necessary authentication and routing, capacity protection and progress reporting.                              |
| MIG-06 | Report committed progress per table and readiness per module, including pauses, errors, competing-task drain and switch attempts. Never infer completion from rows merely selected or sent.                                   |
| MIG-07 | After verification, briefly block new database operations for the switching module, drain its old transactions, apply the final delta, publish durable authority and reopen it on PostgreSQL. Other modules remain available. |
| MIG-08 | Use a small configurable pre-commit switching budget, initially three seconds. If that budget expires before the authority decision is committed, abandon that switch attempt and continue serving on SQLite.                 |
| MIG-09 | Resume from durable checkpoints after interruption. A PostgreSQL failure before cutover must not move authority or make the SQLite service wait for migration retries.                                                        |
| MIG-10 | After authority changes, retain the old source for controlled recovery and reject stale writers. Do not automatically fail back to its outdated contents.                                                                     |
| MIG-11 | Prioritize modules that remove observed writer contention and expensive queries. Migration is not evidence that upstream latency or unrelated application CPU problems are solved.                                            |

## Module ownership

The migration catalog must enumerate every table, index, trigger, foreign key, writer, reader, background owner, archive dependency and transaction partner before a module becomes eligible. Each table has exactly one module owner. Shared tables cannot be assigned to different modules merely because their rows have different dataset labels.

Module separation is logical ownership; remaining control and traffic tables may still share the original SQLite file. This design does not require first copying control tables into another SQLite file. Architecture diagrams show those responsibilities separately, without implying independent SQLite writer capacity while they still share a file.

| Module                          | Initial boundary                                                                                                                                                                                                                                                            | Ordering and conditions                                                                                                              |
| ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| Traffic and archive             | Invocation facts and live identities; final pool/forward-proxy attempts; archive manifests, publication proofs and recovery state; Raw blob ownership and links; source-change obligations; any rollup or coverage state required by the same fact/publication transaction. | First priority. Compute transaction closure before moving data. Include all users of shared archive/Raw ownership tables.            |
| Analytics                       | Rebuildable Summary, timeseries, long-term and prompt-cache projections and their consumer checkpoints, where their writes are already independent of fact commits.                                                                                                         | May follow traffic. A remaining same-transaction dependency joins the first module or is refactored into a durable obligation first. |
| Account and routing controls    | Accounts, credentials, groups, global policies, sticky controls and their generation fences.                                                                                                                                                                                | May remain in SQLite until measurements justify a later move. Their readers must not scan the old traffic tables.                    |
| Task operations and performance | Existing maintenance SQLite controls/history and the independent performance store.                                                                                                                                                                                         | Keep their existing ownership; moving business data must not put operational writes back into the main database.                     |

The first module is intentionally larger than `codex_invocations`. The terminal materializer currently commits invocation facts, final attempt snapshots and the Summary source descriptor together ([flush.rs](../../src/sqlite_batch_writer/flush.rs#L1011)). Retention commits archive publication, source conversion/deletion, required rollups and Raw-reference safety under one owner ([task_batches.rs](../../src/maintenance/retention/task_batches.rs#L1034), [rollup protection](../../src/maintenance/retention/task_batches.rs#L1750)). Invocation and attempt changes also maintain Raw links through triggers ([runtime_system.rs](../../src/schema/runtime_system.rs#L1409)). Merely redirecting one table would break these boundaries.

Move the expensive traffic/retention paths with their data. During the observed incident, maintenance held approximately 49% of the main write coordinator's measured time; this is a prioritization signal for that incident, not a promised performance improvement. Moving only a statistics query backend leaves those SQLite maintenance writes intact.

## Runtime interfaces

```text
ModuleStorage
  with_transaction(module, authority_generation, operation)
  read(module, authority_generation, selection)

MigrationCatalog
  module → tables, dependencies, backend, generation, migration_state

ModuleMigrator
  prepare → copy tables → reconcile → verify → gate → publish authority

MigrationTaskPolicy
  effective permission = operator policy ∩ migration admission policy
```

Acquire a module lease at the database operation or transaction boundary. Capture one backend and generation for the whole transaction; never route individual SQL statements of a transaction independently. Every API, journal consumer, scheduled/manual worker and shutdown/recovery writer must use the same authority gate. An existing HTTP stream does not hold a database lease for its entire network lifetime.

Keep the SQLite write coordinator for remaining SQLite operations. PostgreSQL work gets its own bounded pool and workload admission, rather than remaining serialized behind that coordinator. PostgreSQL needs translated queries, constraints and concurrency rules; replacing the connection URL is insufficient.

An analytics consumer can read authoritative traffic facts and commit its projection plus cursor in the analytics backend. It uses a stable source identity, version and watermark, and retains the source change obligation until that commit is acknowledged. No cross-backend SQL transaction is implied.

Account last-activity/last-selected observations currently share a derived SQLite batch with rollups ([flush.rs](../../src/sqlite_batch_writer/flush.rs#L1328)). Before separating these owners, replace that accidental transaction coupling with a durable, idempotent observation channel. Keep routing generations, account disablement, credential updates and immediate runtime routing visibility authoritative in the control module. An observational timestamp cannot overwrite a newer control decision. Queries that previously joined owners must use bounded module APIs, immutable observation snapshots or projections with declared freshness.

This includes archive-driven historical account activity ([writers.rs](../../src/maintenance/archive/writers.rs#L1972)), not just terminal activity. Sticky escape also reads recent final attempts for routing ([candidate_loading.rs](../../src/upstream_accounts/routing/candidate_loading.rs#L1308)); it must resolve the traffic module's current backend or use a dedicated authoritative routing projection. A stale historical query against retained SQLite attempts cannot decide live routing after cutover.

## Background migration protocol

```text
pending → preparing → copying → catching_up → verifying → ready
                                                        ↓
                                                   switching → postgres
                                                        ↓ before authority commit only
                                                   catching_up

Any background state may pause or report blocked/error with a durable reason.
An uncertain switch enters recovering until its durable authority is resolved.
```

1. Validate source identity, schema compatibility, table membership and target capacity. Prepare target schema and source change capture in bounded, retryable background operations. If an SQLite schema lock is unavailable, defer instead of delaying startup or competing indefinitely with proxy writes.
2. Start durable transactional change capture before copying any baseline rows. Capture insert, update, delete, key changes and replacement semantics. Inventory all write connections and trigger interactions; untracked delete/replace behavior blocks migration. The new application is responsible for capture, so this design does not depend on an older binary understanding a new migration protocol.
3. Copy each table through short keyset pages, with a fixed upper scan bound and stable identities. Persist copied data and its cursor in one target transaction. A paginated baseline is not a single point-in-time snapshot; replay changes from capture start, including changes to rows already copied, to reconcile it. Do not keep a long SQLite read transaction open across the copy.
4. Tail compact versioned changes in bounded batches. Read the selected keys, their current source states and relevant watermark in the same short source transaction. Apply rows/tombstones and the consumer checkpoint together in PostgreSQL. Acknowledge only the source version actually applied, after target commit; newer changes must remain pending. Replays are idempotent and may not overwrite a newer target version.
5. Build required indexes and validate constraints, stable IDs, sequence/reservation ceilings, source identity, chunk digests, deletion coverage, archive proofs and consumer coverage in the background. Compare normalized values, not backend-specific JSON/text serialization. Revalidate chunks affected by later changes and retain their proof state. Row counts alone are insufficient.
6. Mark the module ready only when every table is complete, required consumers are covered, no correctness obligation is missing, native work has safe drain boundaries and the remaining delta fits the short switch budget. If replay throughput does not exceed incoming changes, report non-convergence and adjust load/batching before attempting a switch.

Start with one active module and one bounded source-copy worker. Tune small row/byte/time budgets from measured lock waits, CPU, disk headroom and successful catch-up throughput. Release source connections between batches. Pause and resume controls must preserve durable progress; a retry cannot restart a full scan unnecessarily.

An applied watermark represents a proven complete prefix, not the largest event ID seen. Retain pending earlier obligations even when later events were applied or coalesced. Copy/replay may run storage invariants but must not resend business notifications, duplicate routing observations or delete Raw/archive files as a side effect of mirroring.

Bound capture and target-staging storage and expose their capacity. Before source headroom is exhausted, pause or invalidate the uncommitted migration attempt safely; if capture continuity is lost, require a new capture generation and baseline before eligibility returns. Never discard an unacknowledged change and still claim that the current copy can cut over. Source authority remains intact when a private target preparation is abandoned.

## Short module switch

All expensive index building, historical validation and bulk transfer must already be complete before closing the gate.

1. Close admission for the module's new database operations. Queue only a bounded number of callers with deadlines; excess demand receives an explicit retry response. Unrelated modules continue.
2. Drain old-generation transactions, native cleanup and module workers. Release leases only when underlying work has committed or rolled back and actually stopped. If drain cannot fit the pre-commit budget, reopen the source and retry later.
3. Freeze the final source watermark `H`. Apply and verify every change through `H`, including deletions and any journal records accepted but not yet materialized. Required obligations may instead be durably handed to the new materializer, with ownership and exactly-once effects proved by stable identities and idempotent acknowledgement.
4. Prepare a durable source fence and target readiness certificate. Atomically publish the small authority manifest `{installation, module, backend: postgres, generation, H, target_identity}` using an acknowledged durable write. The authority record is not best-effort task-page telemetry. Enforce single-process ownership, drain all old leases and fence source write transactions; disabling a scheduler alone cannot prevent a stale write. Recovery resolves any prepared fence from the manifest before reopening either backend.
5. Publish the new router generation, invalidate affected caches/read-model generations, hand consumers their durable checkpoints and reopen admission. Retain confirmed terminal/routing journal entries until their actual destination commit is acknowledged; never acknowledge them merely because the module changed backend.

Three seconds is the suggested **pre-commit attempt budget**, not a guarantee that a stalled operating-system durability call returns within three seconds. Timeout before publishing authority reopens SQLite safely. If manifest publication has started and its outcome is unknown, keep the module in `recovering`, resolve the durable decision, and never issue conflicting writes. After a confirmed PostgreSQL decision, repair forward on PostgreSQL rather than reverting authority.

Existing upstream streams continue. Their later terminal operation obtains the current module lease and lands in the current authority, or remains in its confirmed durable journal subject to existing bounded capacity admission. No network stream is forcibly terminated to drain a database generation.

## Task suspension and restoration

Use a migration admission overlay, separate from persisted operator enable switches and schedules. Expose the effective reason `migration_hold`, the blocking module and last safe checkpoint. Apply it before spawning startup backfills and before scheduled, adaptive, event-driven and manual heavy work; controlling the central scheduler alone misses independent runtime workers.

| Treatment                            | Work                                                                                                                                                                                                                                                                              |
| ------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Pause at a safe checkpoint           | Retention/archive loops, detail pruning, quota compaction, expired-row/history cleanup, identity and orphan sweeps; automatic Raw compression/inventory; startup historical backfills/hourly bootstrap; optional index preparation and bulk archive verification/materialization. |
| Pause expensive derived work         | Summary coverage repair, full snapshot rebuilds, long-term/minute/timeline projections, prompt-cache historical materialization and dashboard reconciliation that read/write the migrating module. Preserve dirty markers and recovery obligations.                               |
| Keep                                 | Proxy forwarding; necessary OAuth renewal and valid-route availability; terminal/routing durable journals and their essential materializers; stable-ID allocation; authoritative control writes; bounded essential orphan recovery; basic health and migration status.            |
| Keep under a strict emergency budget | Disk-capacity checks and proven safe cleanup needed to avoid storage exhaustion. Never remove unacknowledged migration changes, live Raw references, archive recovery evidence or confirmed journal entries.                                                                      |

Some existing tasks mix required authentication/routing with optional quota polling, inventory or subscription probing. Give these subtasks independent migration admission; do not disable an entire task if that would let credentials expire or remove the only valid route. Likewise, retaining a compact recovery marker does not authorize an expensive full projection rebuild.

Existing execution-class labels are insufficient for this policy: `maintenance_retention` includes account maintenance and orphan recovery, and `p2_derived` includes forward-proxy subscription refresh ([maintenance_store.rs](../../src/maintenance_store.rs#L1487)). Subscription maintenance can replace actual routing endpoints as well as flush dashboard rollups; retain the availability-related part and hold the optional rollups. Classify individual obligations rather than suspending everything with one label.

A currently running task may finish its current bounded atomic batch and checkpoint. Stop its continuation and wait for native cleanup before releasing its ownership. Already queued heavy manual work remains explicitly deferred; new manual starts return the migration hold reason. Do not silently accept a task and never run it.

Basic proxy and admin paths stay available. Statistical views use existing trusted read-model and data-quality rules; a stale result must show its age/quality, and an unsafe or absent baseline remains unavailable. Request handlers cannot defeat suspension by running historical reconstruction synchronously. The plan does not depend on blanket shutdown of history-query endpoints.

Restore tasks on the backend that now owns their module. Remove only the migration overlay and honor the operator's latest enablement and schedule generation; do not restore a stale snapshot of settings. Stagger heavy work and resume checkpoints, without replaying every missed schedule occurrence. An accepted cutover need not wait for optional analytics to become fresh.

Pause competing heavy tasks only while the migration actively copies, reconciles, verifies or switches the relevant module. For a paused or persistently blocked migration, finish/drain its current operation, release the migration hold and keep change capture intact; no silent indefinite task suspension. Resuming migration reacquires the hold before background work resumes. Capacity guards retain independent authority throughout.

## Progress contract

One persistent migration identity spans restart and module transitions. The system page and structured status endpoint must report the same facts; an SSE view may subscribe to updates. Control and observation use existing administrator permissions. Avoid secrets, raw records and account identifiers in migration telemetry.

| Scope     | Required signals                                                                                                                                                                       |
| --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Migration | Source/target identity; active module; queued/completed modules; current authority generations; heartbeat; pause/block/error reason; safe resume action.                               |
| Table     | State; copied committed rows/bytes; scan cursor and fixed scan bound; index preparation; validated/dirty chunks; pending changed keys and oldest-change age; applied source watermark. |
| Module    | Dependency readiness; unapplied changes; final proof/constraint coverage; task-hold and drain status; switch attempt count, duration, result and failure reason.                       |
| Pressure  | Source lock waits and P1 backlog, migration throughput, incoming-change rate, convergence trend, disk/journal headroom and active budget.                                              |

Copying, reconciliation, verification and authority are separate dimensions. Show a bounded baseline-copy percentage where its denominator is known. Live catch-up uses backlog and age, not an invented percentage; show an ETA only when a measured convergence rate supports it. A table can finish copying while its module still serves SQLite. A module is complete only after its PostgreSQL authority decision is durable and recognized by the router. Total service migration can remain partial with control modules intentionally on SQLite.

Progress observations may be cached or best effort, but the view labels stale observations and never substitutes them for authoritative checkpoints. Each pause, failure and restart must retain the last committed position and enough bounded diagnostic detail to explain why work stopped.

## Startup, storage and failure recovery

- Separate PostgreSQL connection configuration from SQLite and local data-directory paths. Preserve Raw, archive, terminal/routing journals, installation secrets, maintenance database and performance database locations; they cannot be derived from a PostgreSQL URL.
- The prior production observation found a roughly 26 GB main file, a roughly 2.6 GB WAL and only roughly 2.5 GiB available on its filesystem. Provision measured target and source-WAL/change-log headroom before enabling migration. Using PostgreSQL on the same nearly full filesystem does not resolve capacity pressure.
- Before cutover, a failed batch or PostgreSQL outage pauses migration, preserves source service and retries with bounded backoff. After cutover, PostgreSQL unavailability is a module outage subject to the existing durable journal capacity contract; it does not authorize writes to stale SQLite tables.
- Restart inspects the small durable authority manifest and reconciles any interrupted gate/fence. Completed modules stay on PostgreSQL. Unfinished copies resume at committed checkpoints. Target data without a matching source/installation identity is rejected.
- Losing or corrupting authority metadata is an explicit recovery problem. Do not infer authority from row counts, whichever database is reachable, or a fresh task-page status. Protect and back up authority metadata with the installation and both stores.
- Keep old module tables and change evidence until post-cutover verification and an independent retention decision. Later deletion/file compaction is a separately scheduled heavy operation. Logical migration alone does not shrink the SQLite file; do not run a large `VACUUM` on application startup or during the cutover gate.
- No Redis, ClickHouse or distributed transaction coordinator is required by this design. Existing journals plus compact transactional obligations provide recovery boundaries; PostgreSQL and the retained SQLite/file owners need explicit backup and restore coordination.

## Compatibility and acceptance

The initial supported source is the deployed program state from `4.13.11`; other versions require explicit fixtures before support is claimed. Public proxy/HTTP/SSE behavior remains compatible, while the new module authority and PostgreSQL state require an anticipated minor persistent-state change. An older program must fail closed against a migrated installation rather than writing retained SQLite tables. Implement and verify a recognized compatibility fence before the first cutover; until then, deployment controls must prevent rollback to an unfenced binary.

DDL, current-data routing/authority changes and historical copying remain separately identified, observable and pauseable in the [persistent-state migration record](../adr/assets/module-owned-postgresql-migration/persistent-state-migration-record.json). The [acceptance fixture record](../adr/assets/module-owned-postgresql-migration/acceptance-fixtures.json), [acceptance contract](../specs/postgresql-module-migration/ACCEPTANCE.md) and [operations runbook](../runbooks/postgresql-online-migration-acceptance.md) define how runtime evidence is recorded. This is not completed runtime validation.

The [version-impact record](../adr/assets/module-owned-postgresql-migration/version-impact-record.json)
keeps public API and durable-state compatibility separate. The [pg01 inventory contract](postgresql-module-inventory.md) is the executable
table/trigger/FK/transaction/recovery classification and preserves the A1
complete-enumeration gate. Runtime actor call-site reconciliation and PostgreSQL
adapters remain implementation deliverables; the former is a pg08 boundary.

Required implementation evidence:

1. Complete module/transaction inventory; reject a transaction whose tables resolve to different backends; verify all API, scheduler, journal, recovery and shutdown writers use the same generation fence.
2. Concurrent insert/update/delete/key change/replace during copy, page retries and checkpoint restart; prove no missing facts, duplicate side effects, resurrected deletions or stale overwrites.
3. Crash/fault injection around target commit, source acknowledgement, source fencing, authority publish, router publication and journal acknowledgement, including uncertain filesystem/commit outcomes.
4. Invocation/attempt/source-descriptor atomicity; archive proof/source conversion/Raw ownership safety; stable ID/reservation ceilings; account-control generation protection across module boundaries.
5. Startup duration independent of historical table size for newly added migration work; PostgreSQL down before and after cutover; representative live traffic and bounded source-WAL/disk growth.
6. Switching under ongoing requests, long upstream streams, slow readers and running native maintenance: successful short gates, safe timed-out attempts and recovery after committed cutover.
7. Accurate table/module progress across pause, error and restart; task-overlay restoration preserves newer operator choices and avoids a catch-up storm.
8. Compare contention, P1 persistence backlog, endpoint latency, successful work and data coverage against the same representative workload. Reduced work caused by a paused feature must be reported separately from permanent performance improvement.

Implementation should begin with the storage router and module inventory, durable migration state and task admission overlay; then add the first module's PostgreSQL adapter, background copy/change capture, verification and cutover. Subsequent modules reuse the same protocol. No release or production migration has been performed by this design task.
