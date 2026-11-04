---
status: accepted
---

# ADR 0053: Managed PostgreSQL analytics migration and cutover

This ADR extends the PostgreSQL analytics foundation with an application-owned migration contract. It keeps SQLite as the authority for business facts, lets the service startup task upgrade the PostgreSQL analytics target, and makes PostgreSQL read eligibility an explicit, reversible cutover condition. Status: accepted and implemented for the service runtime; it supersedes ADR 0051's pre-provisioned-schema boundary for `ANALYTICS_BACKEND=postgres`.

## Grill and design tree

The decision tree below records the design frontier. The recommendations are the answers that make the tree internally consistent; questions marked **owner decision** remain explicitly open until the owner accepts the contract.

### Round 1: authority, credentials, and downtime

**Q1. Which store owns business facts?**

Recommended answer: SQLite remains the business-fact authority. PostgreSQL is a derived analytics replica. No migration in this contract copies business writes, account state, task state, routing state, or TerminalJournal ownership.

**Q2. Can one PostgreSQL login safely serve schema migration, analytics reads, and replica writes?**

Recommended answer: no. The secure contract uses a short-lived migration credential plus separate reader and maintenance credentials. A literal single-login contract must either grant DDL or combine read and replica-write authority, which is a deliberate security relaxation and is not silently accepted.

**Q3. Is online replica bootstrap part of this contract?**

Recommended answer: yes. The service uses a bounded SQLite consistent read transaction while writers continue, captures source watermarks with the snapshot, and catches up through the existing idempotent/CAS journal path. The manual `analytics-replica` CLI remains an explicit maintenance tool.

### Round 2: lifecycle ownership and readiness

**Q4. Where does PostgreSQL DDL run?**

Recommended answer: in the service's startup migration task, using a short-lived migration pool that is never retained by application or maintenance pools. This keeps the credential out of long-lived query/maintenance pools and preserves the reader/maintenance least-privilege boundary.

**Q5. When may PostgreSQL serve reads?**

Recommended answer: only after the schema contract, role contract, replica baseline, source checkpoint, projection state, semantic version, and reader privilege probe all pass. A connection or table probe alone is not analytics readiness.

**Q6. What happens when PostgreSQL is unavailable?**

Recommended answer: SQLite business operations remain authoritative; PostgreSQL analytics operations return a stable unavailable result and never fall back to an unbounded or semantically different SQLite analytics query. Core readiness and analytics readiness are separate conditions so a deployment gate cannot mistake a core-ready process for a PG-ready process.

### Round 3: compatibility and rollback

**Q7. How are old schema and old binaries handled?**

Recommended answer: migrations are ordered, additive-first, and checksum-verified. A schema newer than the running reader is rejected. Semantic migrations fence the legacy-visible eligibility flag before rebuilding. An older binary may continue on SQLite, but it must fail closed if it attempts to read a target whose schema or fact-date semantics it cannot interpret.

**Q8. What is the rollback unit?**

Recommended answer: configuration and traffic selection, not reverse-copy. Stop or drain the PG-selected process, restore the SQLite-selected configuration, verify core readiness, and retain the PG target for diagnosis. The migration unit never deletes the PG target as part of rollback.

## Proposed contract

### 1. Ownership and input boundary

The deployment contract has three logical credentials:

- `POSTGRES_MIGRATION_URL`: a temporary database/role provisioning credential. It may create or alter the fixed analytics roles and run schema DDL. It is consumed only by the migration unit and is never loaded by the long-lived service.
- `POSTGRES_DATABASE_URL`: the application reader URL. It uses the fixed reader role, has no DDL or analytics-table write privilege, and is redacted everywhere outside the connection layer.
- `POSTGRES_ANALYTICS_MAINTENANCE_URL`: the replica maintenance URL. It uses a distinct login role for bounded facts/keys/state DML and bootstrap control. It is never substituted with the application URL.

The target database itself is created by the PostgreSQL container's initialization contract (`POSTGRES_DB`) or by an explicitly supplied admin connection to a maintenance database. The target-database migration unit does not attempt to create or drop the database. This avoids hiding a cluster-level operation behind a database-scoped migration and makes a missing database a clear preflight failure.

The migration unit may create the fixed roles when its credential has `CREATEROLE` or equivalent administrator authority. It must not create arbitrary user-supplied roles, log passwords, rotate an existing password implicitly, or persist generated credentials in the repository. A platform secret handoff is required when runtime role passwords are generated.

The fixed roles remain:

- `cvm_analytics_owner`: `NOLOGIN`, owns the `cvm_analytics` schema and its objects, and is used only through the migration credential's controlled `SET ROLE` boundary.
- `cvm_analytics_reader`: `NOLOGIN`, receives the reviewed column-level `SELECT` contract.
- `cvm_analytics_app`: `LOGIN`, inherits only the reader role.
- `cvm_analytics_maintenance`: `LOGIN`, receives only the reviewed replica DML and state-control contract.

The migration unit reconciles role attributes, schema ownership, `PUBLIC` privileges, and explicit grants. It fails closed when an expected object has an unexpected owner or type; it does not repair an object by dropping it or by applying `CASCADE`. Neither runtime role receives DDL, `CREATEROLE`, `CREATEDB`, `SUPERUSER`, `REPLICATION`, or `BYPASSRLS`.

**Owner decision:** whether the deployment surface supplies three DSNs, or supplies one temporary bootstrap DSN plus a platform-managed secret sink that hands two generated runtime DSNs to the application. The latter preserves the “run a container and provide one account” operator experience, but it requires a defined secret handoff. The application must not silently use one privileged login for all three purposes.

### 2. Schema version and migration ledger

The PostgreSQL target gains a migration ledger owned by `cvm_analytics_owner`, separate from the replica's fact-date semantic version. The two versions must never be conflated:

- **Schema version** describes relations, columns, constraints, indexes, grants, and migration code compatibility.
- **Fact-date semantic version** describes how a normalized fact maps an event timestamp to its Shanghai statistics date. The existing current value is `2`; legacy rows and old writers remain `1` until a bounded rebuild proves the target under the current semantics.

The ledger records at least `component`, `version`, immutable migration checksum, migration state, minimum reader compatibility, whether the migration requires a replica bootstrap, and bounded error code. A migration is visible as applied only after its complete unit commits. A checksum mismatch, an `in_progress` non-transactional step without a recognized repair path, or a schema version newer than the running binary blocks the target.

Every migration has these properties:

1. It is immutable once published; corrections use a new migration number.
2. It is idempotent when retried after a connection loss.
3. It uses expand/dual-read or expand/dual-write compatibility before any contract cleanup.
4. It never drops a column, relation, role, or grant during normal startup migration. Destructive cleanup needs a separate owner-authorized operation after the rollback window closes.
5. It declares whether it changes fact semantics or replica shape. Such a migration fences eligibility and requires a full bounded bootstrap before read cutover.

The migration unit acquires a session-level PostgreSQL advisory lock on a schema-migration key before reading or changing the ledger. The same connection owns the lock through the migration and is closed if the task is cancelled. Replica bootstrap uses a distinct advisory-lock key. A regular DDL migration is transactional where PostgreSQL permits it; non-transactional operations such as concurrent index builds are separate ledger steps with explicit recovery and invalid-index inspection. The migration unit sets bounded lock and statement timeouts and returns a non-zero result rather than marking a partial target ready.

The first managed run adopts an existing foundation only after catalog inspection proves the expected `cvm_analytics` relations, columns, constraints, and ownership. It records an adoption migration; it does not assume that a mounted SQL file was executed. A missing or incompatible foundation follows the normal ordered migrations or fails with a reviewable error.

### 3. Source schema and replica bootstrap

The source SQLite schema is prepared before any PostgreSQL target is allowed to become eligible. `ensure_schema` remains the source of truth for the local database migration, including `analytics_change_events` and its additive `fact_date_semantics_version` column. Existing journal rows and old writers are version `1`; current writers publish semantic version `2` while retaining `event_version = 1` and `AnalyticsFactV1` identity.

The startup migration/bootstrap lifecycle performs the following bootstrap sequence:

1. Keep the core service and SQLite writer available, then acquire a dedicated PostgreSQL bootstrap connection.
2. Acquire the PostgreSQL session advisory bootstrap lock on that same connection; close the connection on cancellation.
3. Persist the legacy-visible rollback fence: `service_eligible = FALSE`; move a previously `caught_up` state to `bootstrapping`.
4. Verify the migrated schema, role contract, source history markers, source generation, and required archive coverage before clearing target data.
5. In one target transaction, clear `analytics_replica_applied_changes`, `analytics_parallel_work_keys`, and `analytics_facts`, reset the target checkpoint/baseline/coverage, set current fact-date semantics, and keep both target/projection state unavailable.
6. Snapshot the bounded normalized facts and parallel-work keys from one SQLite consistent read transaction, recording its generation, latest change id, coverage revision, and retention boundary.
7. Reject generation changes, retention gaps, unproven archive coverage, or snapshot deadline failures. Same-generation journal movement is caught up with the existing idempotent/CAS replay path.
8. Persist the fenced baseline, replay post-snapshot journal batches, acknowledge matching SQLite source coverage, and only then publish analytics readiness. The shared legacy-visible `service_eligible` flag remains false as the rollback fence, as required by the existing compatibility contract.
9. Release the advisory lock and connection before producing the final status report. A success report is valid only if a fresh read-only verification sees the same eligible state.

The source journal's existing identity de-duplication, applied-change primary key, checkpoint CAS, semantic-version validation, and compare-and-swap invalidation remain the required replay invariants. A legacy event, a source gap, retention exhaustion, a changed source fence, or a missing history proof invalidates or blocks the target before target DML can be considered serviceable.

Incremental replay after cutover is a separate bounded maintenance loop. While the target is behind, stale, blocked, or semantically incompatible, PostgreSQL reads remain unavailable. The loop must not turn a partially applied target into a successful response.

### 4. Readiness contract

Readiness is split into two conditions:

| Condition         | Required before it is true                                                                                                                                                                                                                            | Failure effect                                                                 |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| `core_ready`      | SQLite opens, local schema migration completes, required runtime state is initialized, and HTTP can serve the core service                                                                                                                            | Process is not admitted as a service                                           |
| `analytics_ready` | PG connection/TLS/auth, schema ledger, role/privilege probes, current schema compatibility, current fact-date semantics, eligible replica state, matching source checkpoint, ready projection state, and one repeatable-read read-only probe all pass | PG analytics reads return `AnalyticsUnavailable`; no SQLite analytics fallback |

The deployment cutover gate must require both conditions when `ANALYTICS_BACKEND=postgres`. `/health/analytics` is the explicit analytics condition; `/health` remains core readiness and may return 200 while PostgreSQL migration or bootstrap is unavailable.

The startup migration task runs the reviewed schema migration using the short-lived migration URL, while application and maintenance pools never execute DDL. It performs the full online bootstrap and never silently switches to SQLite analytics. A request that begins before a bootstrap fence may finish its single repeatable-read snapshot; a request that begins after the fence must reject the non-eligible state.

### 5. Upgrade and switch order

The deployment orchestrator owns process ordering and traffic admission. The first implementation keeps the SQLite writer online while the service startup task owns migration and bootstrap:

1. Start the PostgreSQL container and wait for the configured endpoint to accept the intended TLS/auth connection. A TCP or `pg_isready` result alone is not sufficient.
2. Start the service with the explicit PostgreSQL configuration; SQLite `ensure_schema` completes before the PG lifecycle task.
3. Wait for both `core_ready` and `analytics_ready` before sending traffic that depends on PostgreSQL analytics.
4. If migration/bootstrap is unavailable, leave the PG target fenced and retry with bounded backoff; rollback selection to SQLite when the owner chooses to restore the prior release.
5. Keep SQLite writes and business facts unchanged throughout. Do not start a reverse-copy or business-write migration as part of this sequence.

An additive-only optimization may add a read-only preflight before the stop and skip bootstrap when the target is already eligible, but it must preserve the same one-shot ledger, fence, readiness, and failure semantics. The orchestrator must never treat a manually mounted SQL file as proof of migration.

For an already-current, eligible target with an additive-only migration, the migration unit may complete without a target reset. For a semantic or replica-shape migration, it must fence and bootstrap even when the old target reports `caught_up`.

### 6. Rollback order

Rollback is configuration-first and data-preserving:

1. Stop or drain the PG-selected release, or remove it from traffic.
2. Restore `ANALYTICS_BACKEND=sqlite` and the previous release configuration.
3. Verify `core_ready` and the SQLite business/read paths before restoring traffic.
4. Retain the PostgreSQL schema, ledger, target rows, and bounded error state for diagnosis. Do not drop the database, revoke roles, or reverse-copy data automatically.
5. If the old release later attempts a PG connection, a schema-newer-than-reader result or the shared legacy fence must make its analytics path fail closed. The old release may use SQLite explicitly.
6. Retry the migration/bootstrap only after the source and target failure reason is understood. A retry is safe because committed migration rows, source identities, target applied-change identities, and checkpoint CAS make the operation idempotent.

Rollback does not claim that every arbitrary historical binary can use every future schema. The supported rollback window is a versioned contract: a release declares its maximum readable schema and fact semantics, and migrations preserve old columns/roles only for that declared window.

### 7. Failure semantics

- SQLite remains the authoritative business-fact store on every PostgreSQL failure.
- A missing endpoint, invalid credential, missing relation/column, wrong owner, failed grant probe, schema checksum mismatch, schema-newer-than-reader condition, bootstrap failure, source-history gap, or stale checkpoint produces a bounded error code and a fenced/unavailable analytics target.
- No partial target, stale target, mixed snapshot, or incomplete projection may be represented as zero-valued successful analytics data.
- Selecting PostgreSQL never authorizes an implicit heavy SQLite analytics query. The caller receives the existing unavailable boundary instead.
- Secrets, connection URLs, raw payloads, prompts, IPs, and unbounded error bodies remain absent from migration state and diagnostics.
- Failure before cutover leaves the previous SQLite service usable. Failure after cutover rolls back selection to SQLite; it does not mutate business facts.

## Compatibility matrix

| Observed state                                           | Migration-unit action                                                        | PG read result                                              | Old release behavior                                |
| -------------------------------------------------------- | ---------------------------------------------------------------------------- | ----------------------------------------------------------- | --------------------------------------------------- |
| Fresh database, no analytics schema                      | Create/adopt roles and schema, then bootstrap                                | Unavailable until final verification; ready after bootstrap | Remains on SQLite                                   |
| Known older schema version                               | Apply each ordered migration and verify checksum                             | Fenced while a bootstrap-required migration runs            | May continue on SQLite if the migration is additive |
| Current schema, current semantic version, caught up      | Verify only; no reset                                                        | Eligible after role and readiness probes                    | Compatible only within declared reader window       |
| Legacy fact-date version or missing compatibility column | Apply additive compatibility migration, keep fence false, run full bootstrap | Unavailable until rebuilt with current semantics            | Must not serve the target; use SQLite               |
| Schema newer than this binary or checksum mismatch       | Stop before DDL/data changes                                                 | Unavailable                                                 | Use SQLite or a compatible release                  |
| Migration/bootstrap interrupted                          | Preserve committed ledger/state, retain fence, retry/repair recognized step  | Unavailable                                                 | Previous SQLite release remains the rollback path   |
| PostgreSQL endpoint unavailable                          | Return non-zero from migration; do not cut over                              | Unavailable                                                 | SQLite remains authoritative                        |

## Orchestration defects versus Rust implementation defects

The pre-provisioned-schema failure class is an orchestration gap: a healthy PG container does not prove that the reviewed schema was executed, that application/maintenance roles exist, or that the selected target contains the required relations. Mounting `src/pg_analytics/schema.rs` output at a path is not execution of that contract.

The following are remaining delivery gates, not claims that the current runtime contract is absent:

- Verify role ownership/grants and ledger adoption against a disposable PostgreSQL 18 fixture.
- Add bounded retry/fencing coverage for migration interruption, permission failure, source generation movement, retention gaps, and concurrent starts.
- Keep the existing `analytics-replica` CLI as the explicit offline maintenance path (`src/maintenance/cli.rs:146-169`).

The following are deployment responsibilities, not fixes to be hidden inside Rust:

- Start PostgreSQL and provide the secret/role inputs; do not expect a mounted file to execute itself.
- Invoke the migration unit before the new process and stop on a non-zero result.
- Propagate both core and analytics readiness conditions to traffic routing; the online path does not take the offline SQLite runtime lock.
- Restore the SQLite configuration on rollback; do not delete PG state as a shortcut.

## Minimum validation matrix

Every default validation item is deterministic and bounded to three minutes or less. No pressure, throughput, backup, production-data copy, or p95 conclusion is required.

| Case                 | Fixture and assertion                                                                                                                                                                                                                                                                 |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Fresh provision      | Disposable PostgreSQL 18 database: role creation/reconciliation, schema ledger, seven required relations, exact grants, no DDL for app/maintenance roles, and final read-only probes.                                                                                                 |
| Re-run idempotency   | Run the migration unit twice against the same synthetic target; assert no duplicate ledger rows, no privilege drift, no target reset for additive-only migrations, and the same final contract hash.                                                                                  |
| Legacy upgrade       | Seed the known low-version relation set and a v1 replica/source journal; apply ordered migrations; assert the compatibility column, fence, full v2 bootstrap, matching checkpoints, and eventual eligibility.                                                                         |
| Interrupted run      | Inject failure after a committed migration step, after the bootstrap fence, and during a target batch; rerun and assert the target never reports ready before verification and reaches the same final state after retry.                                                              |
| Source compatibility | Use a legacy SQLite fixture without the additive journal column, run `ensure_schema` twice, assert old rows remain v1, current writes publish v2, and no business rows are rewritten.                                                                                                 |
| Fail closed          | Exercise PG down, wrong password, missing relation/column, wrong role grant, `bootstrapping`, stale checkpoint, source gap, and schema-newer-than-reader; assert bounded unavailable results, no zero fabrication, no SQLite analytics fallback, and unchanged SQLite business facts. |
| Cutover/rollback     | Fake a non-zero migration or failed analytics readiness probe; assert orchestration does not admit the PG release. Then select SQLite and assert core readiness without any reverse-copy.                                                                                             |

## Owner decisions before implementation

1. Choose the credential handoff: three explicit DSNs, or one temporary bootstrap DSN plus a platform secret sink for generated reader/maintenance credentials. The recommended security choice is the latter only when the secret sink is already a supported deployment primitive; otherwise use three explicit secret references.
2. Confirm whether the target database is always created by the PG container (`POSTGRES_DB`) or whether an admin-database URL must be supported. The recommended first implementation accepts the former only and fails closed when the target database is absent.
3. Confirm whether a planned stop is acceptable for the first bootstrap. The recommended implementation preserves the existing offline CLI contract and does not promise zero downtime.
4. Choose the supported rollback window, including the oldest application release allowed to encounter a migrated target. The recommended window is one prior release with additive schema compatibility and semantic eligibility fencing.
5. Confirm the PostgreSQL major-version admission policy. The recommended first implementation admits PostgreSQL 18.x only and refuses automatic cross-major upgrades.
6. Decide whether `/health` should aggregate analytics readiness or whether a separate readiness condition/endpoint is preferred. The recommended design keeps core readiness and analytics readiness separate and makes the deployment cutover require both.

## Non-goals

This proposal does not migrate business writes, account routing, task state, SQLite retirement, historical raw-data backup, pressure testing, production deployment, or a zero-downtime snapshot protocol. It does not authorize changes to 101, PostgreSQL containers, Compose files, credentials, or remote repositories.

## Evidence and references

- Existing foundation: [`docs/adr/0051-pg18-analytics-offload.md`](0051-pg18-analytics-offload.md)
- Existing compatibility contract: [`docs/specs/pg18-analytics/SPEC.md`](../specs/pg18-analytics/SPEC.md)
- Existing operations boundary: [`docs/runbooks/pg18-analytics.md`](../runbooks/pg18-analytics.md)
- SQLite migration order: [`src/schema.rs`](../../src/schema.rs), [`src/analytics_changes.rs`](../../src/analytics_changes.rs)
- Runtime and CLI evidence: [`src/runtime.rs`](../../src/runtime.rs), [`src/maintenance/cli.rs`](../../src/maintenance/cli.rs), [`src/pg_analytics/mod.rs`](../../src/pg_analytics/mod.rs)
- PostgreSQL 18 [`CREATE ROLE`](https://www.postgresql.org/docs/18/sql-createrole.html), [`CREATE DATABASE`](https://www.postgresql.org/docs/18/sql-createdatabase.html), [`CREATE SCHEMA`](https://www.postgresql.org/docs/18/sql-createschema.html), [`advisory locks`](https://www.postgresql.org/docs/18/explicit-locking.html), and [`CREATE INDEX`](https://www.postgresql.org/docs/18/sql-createindex.html) documentation.
