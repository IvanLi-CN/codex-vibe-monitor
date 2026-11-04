# 可选 PostgreSQL Analytics Offload 实现状态

> 当前有效规范仍以 `./SPEC.md` 为准；本文记录实现覆盖、验证边界和未纳入本基础任务的后续工作。

## Current Status

- Implementation: I1 配置与运行时入口、I2 fact/query interface、I3 schema/pool/transaction verification foundation、managed migration ledger、online bootstrap lifecycle 已实现。
- Lifecycle: active
- Catalog note: PostgreSQL 为显式 opt-in；SQLite 默认路径保持现状。

## Managed Migration and Cutover Proposal

- Status: implementation-verified; [ADR 0053](../../adr/0053-pg18-analytics-managed-migration.md) supersedes the ADR 0051 provisioning boundary for this runtime path.
- The release unit keeps SQLite as the business-fact authority and runs PostgreSQL migration from `POSTGRES_MIGRATION_URL`, followed by maintenance-role online snapshot and journal catch-up. The migration credential is not retained by the long-lived pool.
- The runtime order is: SQLite `ensure_schema`; PostgreSQL 18.x ordered ledger migration; application and maintenance pool verification; fenced online bootstrap; bounded journal catch-up; separate analytics readiness publication. Core readiness remains independent.
- A failed migration, bootstrap, source proof, or readiness probe leaves PostgreSQL unavailable and keeps SQLite facts usable. Rollback changes configuration to SQLite; it does not reverse-copy business facts or delete the PostgreSQL target.
- The associated migration and version-impact records are [managed-migration-state-record.json](./assets/managed-migration-state-record.json) and [managed-migration-version-impact.json](./assets/managed-migration-version-impact.json); both record the implemented contract and bounded validation still required for current-head delivery.

## Implementation Coverage

- `REQ-PG18-001`: `src/main.rs`、`src/config.rs`、`src/pg_analytics/mod.rs` 提供三组 DSN、`ANALYTICS_BACKEND` 和 `POSTGRES_MAX_CONNECTIONS`；按 SQLx 生效用户名（含 query 覆盖）拒绝相同 migration/application/maintenance 角色；URL 脱敏，PostgreSQL 18.x 版本门禁和缺失凭据 fail-closed；测试构造器显式使用 SQLite disabled runtime。
- `REQ-PG18-002`: `PostgresDatabaseUrl` 自定义 `Debug`/`Serialize`；`AnalyticsFact` 与 `src/pg_analytics/schema.rs` 只保留规范化 analytics 字段。
- `REQ-PG18-003`: `PgAnalyticsPool` 校验七个预置关系、fact-date 兼容列并执行 rollback 的 read-only transaction；`PgAnalyticsRuntime` 保存 ready/unavailable 状态，未实现 SQLite fallback。
- `REQ-PG18-004`: `AnalyticsRange`、`AnalyticsDimension`、metrics、projection state、overview/series DTO 和 `AnalyticsReadStore` 定义未来读取边界。
- `REQ-PG18-005`: `AppState` 仅增加可选 runtime 入口；现有 SQLite pool、TerminalJournal、long-term projection 和 retention 代码未改写。

## FU02 Summary All Design Boundary

- The operational `GET /api/stats/summary?window=all` contract is captured by [ADR 0052](../../adr/0052-summary-all-bounded-read-contract.md). It is distinct from the long-term `AnalyticsOverview.global` aggregate.
- A bounded PostgreSQL Summary All read requires a verified compact historical authority plus a keyset-paged live tail under one snapshot fence. A one-row aggregate over all facts is not bounded, and the current long-term daily/hourly rollups do not contain every operational Summary field.
- This follow-up is design-only. It does not implement `AnalyticsReadStore`, change the public `StatsResponse`, add schema/rollup columns, alter fact derivation, switch routes, or introduce SQLite fallback after explicit PostgreSQL selection.
- The required evidence is deterministic synthetic parity and fail-closed boundary coverage only; no pressure, throughput, scale, production-data, or p95 conclusion is part of this follow-up.

## Verification Commands

- `cargo fmt --all`
- `cargo check --locked --all-targets --all-features`
- Focused `pg_analytics` unit tests and existing lightweight configuration compatibility tests are bounded deterministic checks.
- FU01 fact ingestion derives `stats_date` with explicit Asia/Shanghai calendar-boundary semantics for UTC-formatted source timestamps while preserving naive Shanghai timestamps; deterministic boundary regressions cover both snapshot and incremental fact paths.
- FU01 compatibility persists fact-date semantics version `2` on the PostgreSQL replica state and on current SQLite source-journal events, while existing rows and older writers remain explicitly marked as legacy version `1`. Legacy or schema-incomplete replicas fail closed and require an explicit schema-owner provisioning step followed by maintenance-role bootstrap; the owner-applied contract and bootstrap preflight fence the shared legacy-visible eligibility flag and move a previously caught-up state to `bootstrapping` before any rebuild precondition can fail; bootstrap clears the target, replays the verified completed invocation archives, rebuilds the bounded live snapshot, resets the checkpoint, and only then makes the replica eligible. The shared legacy-visible `service_eligible` flag remains false for v2 state as a rollback fence, while the current program derives effective eligibility from the versioned state and lifecycle. A v2 consumer rejects legacy invocation upsert/tombstone events before target DML, CAS-invalidates the observed replica state, and requires the same explicit bootstrap path. Runtime startup checks the compatibility column through privilege-independent `pg_catalog` metadata, while each composed PostgreSQL analytics response keeps admission and all fact/key queries in one repeatable-read, read-only transaction; bootstrap holds a PostgreSQL advisory lock for the complete bounded rebuild and closes the lock-holding connection if cancelled; it never executes DDL. The owner-applied schema contract is applied atomically. `AnalyticsFactV1` identity and journal event version remain unchanged; controlled archive replay is limited to the fenced bootstrap target rebuild.
- The validation boundary is intentionally limited to commands that complete within 180 seconds: no load generator, stress/throughput test, scale-up experiment, production-data copy, long-running suite, or p95 conclusion is part of this foundation.

## Rollout Facts

- With no new environment variables, `AppConfig::database_url()` remains the existing SQLite URL and the runtime creates no PostgreSQL pool.
- PostgreSQL startup migration executes the reviewed `PG_ANALYTICS_SCHEMA_SQL` and records its checksum in the ordered ledger; the application still does not create/delete the target database.
- A selected but unavailable PostgreSQL backend is represented as unavailable. This foundation does not turn that state into an implicit SQLite analytics query.
- No business write migration, production deployment, or UI route switch is included. Archive replay is limited to the controlled PostgreSQL bootstrap target rebuild.

## Remaining Gaps

- A future task must implement the fact ingestion/backfill adapter from existing SQLite/TerminalJournal sources.
- A future task must implement concrete `AnalyticsReadStore` query methods and wire only the intended read surfaces to the selected backend.
- A future Summary All implementation must first select a reviewed compact authority for the operational success/failure/non-success-cost and usage fields; without it, the PostgreSQL path remains unavailable and the existing SQLite path remains the source of truth.
- The ignored `pg_analytics::tests::postgres_analytics_fixture_verifies_role_separation_and_bounded_reset` test requires an explicitly authorized disposable PostgreSQL database whose name ends with `_fixture`, plus owner, app, and maintenance URLs; when explicitly provisioned, it verifies role separation, catalog visibility, identity-only maintenance reads, production fact/key/applied-change DML (including conflict and tombstone paths), owner/bootstrap eligibility fences, advisory-lock contention, and bounded target reset.

## Related Changes

- The compatibility contract is captured by [SPEC.md](./SPEC.md) and [ADR 0051](../../adr/0051-pg18-analytics-offload.md).
- The managed migration and cutover proposal is captured by [ADR 0053](../../adr/0053-pg18-analytics-managed-migration.md).
- The durable-state migration record is captured in [persistent-state-migration-record.json](./assets/persistent-state-migration-record.json).
- The separate compatibility impact record is captured in [version-impact-record.json](./assets/version-impact-record.json).
- The proposed managed-migration records are [managed-migration-state-record.json](./assets/managed-migration-state-record.json) and [managed-migration-version-impact.json](./assets/managed-migration-version-impact.json).

## References

- `./SPEC.md`
- `./HISTORY.md`
- `src/pg_analytics/mod.rs`
- `src/pg_analytics/schema.rs`
- `assets/persistent-state-migration-record.json`
