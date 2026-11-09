status: accepted

# ADR 0055: Analytics module ownership migration from SQLite to PostgreSQL

## Decision summary

The migration is staged by analytics module boundary. SQLite remains the authority for business facts and the source journal. PostgreSQL becomes the write and read owner of each analytics-derived table only after that table has a complete replay, write, read, fence, rollback, and retention contract.

The first migration substrate is `cvm_analytics.analytics_facts`, and the first SQLite table released is `long_term_usage_daily`, replaced by `cvm_analytics.analytics_daily_rollups`. Migrating only this one SQLite table is reasonable as phase one because the PG fact substrate has the existing idempotency key `(source, source_row_id)` and journal replay path, while the daily rollup can be rebuilt entirely from PG facts. It is not reasonable to claim the whole analytics module is migrated after this step: other SQLite rollups, projection state, and the source journal remain in service until their own boundaries migrate.

The owner approved this candidate after the hard gates below. If any gate fails during implementation, the migration remains replica-only and the SQLite table is retained.

## Ownership model

1. A business transaction commits the SQLite business fact and its source-journal event atomically. SQLite is the fact and journal authority throughout the staged migration.
2. A journal consumer reads each committed event, validates its generation and semantic version, and applies an idempotent upsert/tombstone to PostgreSQL `analytics_facts`.
3. The PostgreSQL transaction writes the fact, records the event id in `analytics_replica_applied_changes`, and advances `consumer_checkpoint` and `source_acknowledged_through` together. A retry is safe because the event id and `(source, source_row_id)` are unique.
4. PostgreSQL owns analytics-fact writes only after the write fence is enabled. New analytics fact corrections then enter through a PG-owned command that also emits an acknowledgement record; the command cannot bypass the SQLite business-fact journal. The acknowledgement is a derived write receipt, not a second business fact authority.
5. Reads of the migrated table run in PostgreSQL `REPEATABLE READ, READ ONLY` transactions and are admitted only when the durable fence is `caught_up`, generation and coverage match, and the projection/read model reports ready.

## Why one table first

The first unit must minimize cross-store joins and make correctness observable. The pair `analytics_facts` + `analytics_daily_rollups` satisfies that better than moving `invocation_rollup_*` or summary coverage tables, which currently depend on SQLite projection state, archive repair state, or multiple source tables. Only `long_term_usage_daily` is released in this phase; moving it first would otherwise leave a query split across stores and make a successful PG health check insufficient proof of correctness.

The one-table phase is acceptable only if all supported queries for that phase use PG facts plus PG-owned dimensions/rollups. The application must not join SQLite business tables at request time. Any endpoint that still needs SQLite data remains on the SQLite path until its complete dependency closure is migrated.

## Cutover sequence

### 0. Prepare

Create the PG schema and roles, record schema checksum, and provision capacity for the full retained fact range plus replay headroom. Keep SQLite writes unchanged. Start journal capture metrics and a shadow comparator.

### 1. Baseline and replay

Fence PG reads and PG-owned writes. Replay every verified archive, then take a bounded SQLite repeatable-read snapshot and replay journal events after the snapshot watermark. Persist `generation`, `coverage_revision`, `coverage_watermark`, `consumer_checkpoint`, and `source_acknowledged_through`.

### 2. Completeness proof

The proof must include: (a) no archive gap or unreadable archive; (b) source journal retained interval covers the snapshot-to-head interval; (c) source and target counts by day and source; (d) set difference of `(source, source_row_id)` keys is empty; (e) tombstone count and duplicate-event count are reconciled; (f) semantic-version and generation match; (g) PG projection checksums match a deterministic recomputation from PG facts; and (h) retention horizon is at least the configured query horizon plus replay safety window. PG readiness alone is insufficient.

### 3. Read switch

Enable PG reads for the migrated contract only after the proof passes. Readers capture the fence identity and high watermark in one transaction; a fence or generation change invalidates the cursor and returns unavailable rather than mixed data.

### 4. Write switch

Acquire the PG write fence, increment generation, and stop accepting the old SQLite analytics projection writer for the migrated table. The SQLite business fact and journal writer continues. The PG consumer becomes the sole analytics-fact materializer. During a bounded dual-write window, compare PG receipts against journal events; do not count a journal event twice.

### 5. Observe and release

After the agreed observation window, freeze the SQLite table, verify no writer, retain a checksum/export and rollback snapshot, and rename it to an archive/quarantine name. Physical deletion is a separate owner-approved action after retention and restore rehearsal. The source journal is never deleted in this phase.

## Dual-write, replay, and failure rules

- Dual-write means SQLite business fact + journal plus PG materialization, not two independent business writes.
- A PG failure leaves the SQLite fact committed and the journal pending; retries resume from the last acknowledged checkpoint.
- A journal gap, generation mismatch, legacy semantic version, retention gap, or checksum mismatch sets the target to `blocked`/`invalidated`, lowers the shared eligibility fence, and prevents reads and writes until a bounded rebuild.
- Rebuild clears PG facts, applied-event ids, projection state, and checkpoint in one fenced maintenance transaction, then replays archive + snapshot + journal. It never deletes SQLite source data.
- Rollback is configuration/traffic rollback to SQLite plus PG fencing; there is no reverse-copy from PG to SQLite. Rollback is safe only while SQLite business facts and source journal remain intact.

## Later module stages

After the first release of `long_term_usage_daily`, migrate the remaining PG-owned rollups and query read models in dependency order: hourly rollups, parallel-work keys, then summary/coverage state. Each stage repeats the same proof and fence. A SQLite table may be released only when no supported query, writer, repair task, archive loader, or retention job references it. The final state removes SQLite analytics projections while retaining only the business facts and source journal until those domains are separately migrated.

## Docker operations contract

Use two Compose profiles or environment bundles: `sqlite` and `postgres`. `sqlite` starts the current runtime and never requires PG. `postgres` starts PG, runs schema migration/bootstrap/replay, and exposes readiness only after the table proof and fence pass. Operators perform mode changes by selecting the bundle and restarting; no manual cross-database SQL is required. Capacity, WAL, archive, and disk checks are health gates. The host's current high disk usage is a release blocker for physical SQLite cleanup, not a reason to delete source data.

## Rejected alternatives

- **Migrate a rollup first:** rejected because its current dependency closure crosses SQLite projection and repair state.
- **Treat PG replica readiness as ownership:** rejected because it proves neither PG write authority nor SQLite table release safety.
- **Distributed transaction across SQLite and PG:** rejected as operationally complex and unavailable in the current deployment; journal receipt plus idempotent replay gives the required recovery semantics.
- **Delete SQLite immediately after read switch:** rejected because rollback and gap recovery require the source table and journal until the observation and restore gates pass.

## Resolved owner decisions

1. `analytics_facts` is the first PG write substrate, and `long_term_usage_daily` is the first SQLite table released in favor of `analytics_daily_rollups`. No alternative release table is admitted without an equal dependency-closure proof.
2. The dual-write and shadow observation window is seven consecutive days covering normal traffic and one scheduled retention/maintenance pass. Key-set, tombstone, duplicate, and journal checkpoint differences must be zero. Numeric aggregates must match after the existing deterministic decimal/float normalization; any unexplained difference blocks cutover.
3. PG-owned corrections use a journal-linked receipt: the command carries `(source, source_row_id, source_generation, journal_change_id)`, applies an idempotent upsert/tombstone, and records the receipt in the applied-change table. It never creates a business fact outside SQLite.
4. SQLite `long_term_usage_daily` is quarantined first and retained through the configured analytics horizon plus the replay safety window, one backup cycle, and one restore rehearsal. Physical deletion requires a separate owner-approved cleanup change and a zero-reference scan.
5. `/health` remains core-only. `/health/analytics` is the PG traffic gate and must be ready before PG analytics reads or writes receive production traffic.

## Acceptance checklist

- [ ] One table has an explicit owner, writer, reader, source, and dependency closure.
- [ ] Archive, snapshot, journal, generation, retention, key-set, tombstone, and projection proofs pass.
- [ ] Duplicate, lost, out-of-order, legacy-version, and partial-cutover tests pass.
- [ ] PG read and write fences are durable and generation-bound.
- [ ] SQLite rollback path is exercised without reverse-copy.
- [ ] SQLite table is quarantined only after no references remain and restore is rehearsed.
- [ ] Docker `sqlite` and `postgres` profiles are the only operator mode switches.
- [ ] Disk/WAL/backups/health evidence is recorded; no SQLite source data is deleted by this design.
