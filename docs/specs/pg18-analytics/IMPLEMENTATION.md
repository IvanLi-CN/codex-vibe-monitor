# 可选 PostgreSQL Analytics Offload 实现状态

> 当前有效规范仍以 `./SPEC.md` 为准；本文记录实现覆盖、验证边界和未纳入本基础任务的后续工作。

## Current Status

- Implementation: I1 配置与运行时入口、I2 fact/query interface、I3 schema/pool/transaction verification foundation 已实现。
- Lifecycle: active
- Catalog note: PostgreSQL 为显式 opt-in；SQLite 默认路径保持现状。

## Implementation Coverage

- `REQ-PG18-001`: `src/main.rs`、`src/config.rs`、`src/pg_analytics/mod.rs` 提供 `ANALYTICS_BACKEND`、`POSTGRES_DATABASE_URL` 和 `POSTGRES_MAX_CONNECTIONS`；测试构造器显式使用 SQLite disabled runtime。
- `REQ-PG18-002`: `PostgresDatabaseUrl` 自定义 `Debug`/`Serialize`；`AnalyticsFact` 与 `src/pg_analytics/schema.rs` 只保留规范化 analytics 字段。
- `REQ-PG18-003`: `PgAnalyticsPool` 校验四个预置关系并执行 rollback 的 read-only transaction；`PgAnalyticsRuntime` 保存 ready/unavailable 状态，未实现 SQLite fallback。
- `REQ-PG18-004`: `AnalyticsRange`、`AnalyticsDimension`、metrics、projection state、overview/series DTO 和 `AnalyticsReadStore` 定义未来读取边界。
- `REQ-PG18-005`: `AppState` 仅增加可选 runtime 入口；现有 SQLite pool、TerminalJournal、long-term projection 和 retention 代码未改写。

## Verification Commands

- `cargo fmt --all`
- `cargo check --locked --all-targets --all-features`
- Focused `pg_analytics` unit tests and existing lightweight configuration compatibility tests are bounded deterministic checks.
- FU01 fact ingestion derives `stats_date` with explicit Asia/Shanghai calendar-boundary semantics for UTC-formatted source timestamps while preserving naive Shanghai timestamps; deterministic boundary regressions cover both snapshot and incremental fact paths.
- The validation boundary is intentionally limited to commands that complete within 180 seconds: no load generator, stress/throughput test, scale-up experiment, production-data copy, long-running suite, or p95 conclusion is part of this foundation.

## Rollout Facts

- With no new environment variables, `AppConfig::database_url()` remains the existing SQLite URL and the runtime creates no PostgreSQL pool.
- PostgreSQL requires the operator to provision the SQL in `PG_ANALYTICS_SCHEMA_SQL` before selecting the backend; application startup only verifies it.
- A selected but unavailable PostgreSQL backend is represented as unavailable. This foundation does not turn that state into an implicit SQLite analytics query.
- No business write migration, historical backfill, production deployment, or UI route switch is included.

## Remaining Gaps

- A future task must implement the fact ingestion/backfill adapter from existing SQLite/TerminalJournal sources.
- A future task must implement concrete `AnalyticsReadStore` query methods and wire only the intended read surfaces to the selected backend.
- PostgreSQL integration tests require a bounded, explicitly provisioned test fixture; no live external database was started for this foundation.

## Related Changes

- The compatibility contract is captured by [SPEC.md](./SPEC.md) and [ADR 0051](../../adr/0051-pg18-analytics-offload.md).

## References

- `./SPEC.md`
- `./HISTORY.md`
- `src/pg_analytics/mod.rs`
- `src/pg_analytics/schema.rs`
