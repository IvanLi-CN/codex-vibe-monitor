# 可选 PostgreSQL Analytics Offload

> 本主题定义长期统计的可选 PostgreSQL 只读/离线承载边界；当前 SQLite 仍是默认且不变的现有状态来源。

## Context and Scope

- Context: 长期统计需要一个可以独立预置、校验和逐步承接查询的 PostgreSQL analytics 边界，同时不能改变现有 SQLite 状态读取、TerminalJournal、Summary freshness 或 retention 语义。
- In scope: 后端选择配置、连接串脱敏、有界连接池、schema/只读事务校验、规范化 analytics fact、rollup 和 query interface，以及 unavailable 语义。
- Out of scope: 业务写入迁移、历史数据复制/回填实现、SQLite schema 修改、自动 schema 部署、生产部署、前端切换和压力/吞吐性能结论。

## Terms and Interfaces

- `AnalyticsBackend`: `sqlite` 或显式选择的 `postgres`；缺省为 `sqlite`。
- `AnalyticsFact`: 只包含统计所需的规范化身份、时间、维度、token/cost/timing 和终态字段，不包含 raw payload、prompt、IP 或错误正文。
- `AnalyticsReadStore`: 面向 overview/series 的未来只读实现边界；它不改变现有 SQLite 查询合同。
- `AnalyticsUnavailable`: PostgreSQL 连接、schema 或只读事务不可用时的稳定错误边界；持久状态只保留有界错误码，不保存错误正文。

## Requirements

### REQ-PG18-001

- The system MUST keep SQLite behavior unchanged when no analytics configuration is supplied.
- `ANALYTICS_BACKEND` MUST accept only `sqlite` or `postgres` and MUST default to `sqlite`.
- `POSTGRES_MAX_CONNECTIONS` MUST default to `5` and MUST reject zero or values outside the supported integer range.
- Selecting `postgres` MUST require `POSTGRES_DATABASE_URL`; invalid values MUST fail closed without exposing the URL.

### REQ-PG18-002

- The system MUST redact `POSTGRES_DATABASE_URL` in `AppConfig` debug output, serialized configuration, and PostgreSQL failure diagnostics.
- PostgreSQL analytics facts and schema MUST NOT contain raw request/response payloads, prompt-cache keys, requester IPs, or unbounded error text.
- Fact fields MUST cover source identity, invoke identity, Shanghai statistics date, model/upstream dimensions, terminal/failure classification, token/cost samples, and existing timing samples needed by the long-term metrics contract.

### REQ-PG18-003

- The system MUST create a PostgreSQL pool only when `ANALYTICS_BACKEND=postgres` is explicitly selected, using the configured bounded maximum connection count.
- Startup verification MUST check all required analytics relations and execute a disposable read-only transaction.
- Application startup MUST NOT auto-create or mutate the PostgreSQL schema and MUST NOT migrate business writes into PostgreSQL.
- A PostgreSQL connection, relation, or transaction failure MUST expose `AnalyticsUnavailable`; it MUST NOT silently fall back to a heavy SQLite analytics query.

### REQ-PG18-004

- The future analytics read interface MUST preserve the existing long-term range set `7d|30d|180d|365d`, dimensions `overall|model|upstream`, `Asia/Shanghai` timezone, metrics, sample counts, projection state, and daily series shape.
- Read results MUST represent unavailable projection state explicitly and MUST not turn an incomplete or unavailable source into a fabricated zero-valued result.

### REQ-PG18-005

- The optional PostgreSQL foundation MUST preserve existing SQLite long-term stats, TerminalJournal, Summary freshness, retention, raw-safety, and unavailable contracts.
- No existing SQLite table or business write path may depend on PostgreSQL availability for this foundation.

## Verification

### VER-PG18-001

- Method: deterministic configuration and construction checks.
- covers: `REQ-PG18-001`
- Pass condition: no-config construction selects SQLite with max connections `5`; PostgreSQL selection requires a URL; zero max connections is rejected; the existing SQLite database URL remains the selected database URL.

### VER-PG18-002

- Method: serialization, diagnostic, fact-shape, and schema-contract checks.
- covers: `REQ-PG18-002`
- Pass condition: a URL containing credentials is absent from debug/JSON/error text; required fact fields are represented; the schema contract has no raw/prompt/IP columns.

### VER-PG18-003

- Method: bounded runtime contract checks and source inspection.
- covers: `REQ-PG18-003`
- Pass condition: the pool is absent for SQLite, required PostgreSQL relations are verified before readiness, the transaction probe is read-only and rolled back, and failure state remains unavailable without a SQLite fallback branch.

### VER-PG18-004

- Method: enum, DTO, and interface contract checks.
- covers: `REQ-PG18-004`
- Pass condition: all four ranges serialize to their public values, all three dimensions are represented, and overview/series DTOs retain metrics, samples, state, timezone, and daily points.

### VER-PG18-005

- Method: existing backend compile and targeted compatibility tests with analytics disabled.
- covers: `REQ-PG18-005`
- Pass condition: the existing SQLite state construction and long-term code compile and targeted checks pass with `PgAnalyticsRuntime::disabled()`.

## Related ADRs

- [ADR 0051: Optional PostgreSQL analytics offload](../../adr/0051-pg18-analytics-offload.md)

## Visual Evidence

- None. This foundation does not change an owner-facing UI surface.

## References

- `./IMPLEMENTATION.md`
- `./HISTORY.md`
- [`5k89c-long-term-usage-analytics`](../5k89c-long-term-usage-analytics/SPEC.md)
- [`9aucy-db-retention-archive`](../9aucy-db-retention-archive/SPEC.md)
