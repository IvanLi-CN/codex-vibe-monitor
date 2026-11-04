# 可选 PostgreSQL Analytics Offload

> 本主题定义长期统计的可选 PostgreSQL 只读/离线承载边界；当前 SQLite 仍是默认且不变的现有状态来源。

## Context and Scope

- Context: 长期统计需要一个可以独立预置、校验和逐步承接查询的 PostgreSQL analytics 边界，同时不能改变现有 SQLite 状态读取、TerminalJournal、Summary freshness 或 retention 语义。
- In scope: 后端选择配置、连接串脱敏、有界连接池、schema/只读事务校验、规范化 analytics fact、rollup 和 query interface，以及 unavailable 语义。
- Out of scope: 业务写入迁移、SQLite schema 修改、生产部署、前端切换和压力/吞吐性能结论。服务启动负责 PostgreSQL analytics schema migration、受控 archive/gzip 历史回放与 online replica bootstrap。

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
- Selecting `postgres` MUST require a distinct `POSTGRES_MIGRATION_URL`; startup owns PostgreSQL 18.x schema migration through an ordered, checksum-verified ledger and an advisory lock.
- `POSTGRES_MIGRATION_URL`, `POSTGRES_DATABASE_URL`, and `POSTGRES_ANALYTICS_MAINTENANCE_URL` MUST resolve to distinct effective login roles; migration credentials MUST be used only by the startup migration task.
- PostgreSQL replica control commands MUST use a separate `POSTGRES_ANALYTICS_MAINTENANCE_URL`; they MUST fail closed when it is absent, when it is identical to the runtime URL, or when both URLs name the same PostgreSQL login role. They MUST NOT use the runtime application URL for target writes.
- The current PostgreSQL analytics read paths MUST use a reader role with explicit column-level `SELECT` grants for the fact, parallel-work key, replica-state, and projection-status columns they query, including predicate and ordering columns; they MUST receive no default privileges or rollup-table access.
- The maintenance role MUST receive only the reviewed bounded grants: fact and parallel-work tables permit insert/update/delete plus column-level select on `source` and `source_row_id` for `MERGE` match keys and tombstone predicates, without fact-content or metric reads; replica and projection state permit select/update without insert/delete; applied changes permit insert/delete plus column-level select on `change_id` for its idempotency key; and the facts sequence permits usage only. Neither application nor maintenance role may execute schema DDL.

### REQ-PG18-002

- The system MUST redact `POSTGRES_DATABASE_URL` in `AppConfig` debug output, serialized configuration, and PostgreSQL failure diagnostics.
- PostgreSQL analytics facts and schema MUST NOT contain raw request/response payloads, prompt-cache keys, requester IPs, or unbounded error text.
- Fact fields MUST cover source identity, invoke identity, Shanghai statistics date, model/upstream dimensions, terminal/failure classification, token/cost samples, and existing timing samples needed by the long-term metrics contract.

### REQ-PG18-003

- The system MUST create PostgreSQL pools only when `ANALYTICS_BACKEND=postgres` is explicitly selected, using the configured bounded maximum connection count.
- Startup migration MUST accept PostgreSQL 18.x only, preserve the target database boundary, and fail closed on unknown schema shape, checksum mismatch, newer-than-reader ledger state, or insufficient privileges.
- Startup verification MUST check all required analytics relations and execute a disposable read-only transaction after migration.
- Application startup MUST NOT migrate business writes into PostgreSQL or create/delete the target database.
- A PostgreSQL connection, relation, or transaction failure MUST expose `AnalyticsUnavailable`; it MUST NOT silently fall back to a heavy SQLite analytics query.
- SQLite `ensure_schema` and core readiness MUST complete independently; `/health/analytics` MUST return 503 until migration, online snapshot plus journal catch-up, source coverage, and target projection verification publish readiness atomically.
- Online bootstrap MUST use a bounded SQLite repeatable read snapshot without pausing SQLite writers, replay every completed invocation archive through the verified archive loader before the live snapshot, reject missing/unreadable/schema-incompatible archive proof gaps, and keep the target fenced until source and target checkpoints converge.

### REQ-PG18-004

- The future analytics read interface MUST preserve the existing long-term range set `7d|30d|180d|365d`, dimensions `overall|model|upstream`, `Asia/Shanghai` timezone, metrics, sample counts, projection state, and daily series shape.
- Read results MUST represent unavailable projection state explicitly and MUST not turn an incomplete or unavailable source into a fabricated zero-valued result.

### REQ-PG18-005

- The optional PostgreSQL foundation MUST preserve existing SQLite long-term stats, TerminalJournal, Summary freshness, retention, raw-safety, and unavailable contracts.
- No existing SQLite table or business write path may depend on PostgreSQL availability for this foundation.

### REQ-PG18-006

- The PostgreSQL analytics replica state MUST persist `fact_date_semantics_version`; a fresh schema MUST initialize the current version as `2`, while a schema missing the compatibility column MUST fail closed until the schema owner applies the reviewed compatibility DDL with default `1` and an explicit bootstrap rebuilds the bounded target snapshot.
- Replica service eligibility MUST require the current fact-date semantics version, a completed baseline, and matching target/source checkpoints. Legacy fact rows MUST NOT become service eligible under the corrected semantics without an explicit bootstrap rebuild. The shared `service_eligible` flag MUST remain false after a current v2 bootstrap so programs that ignore the additive semantic-version column fail closed during rollback; the current program MUST derive effective eligibility from the versioned state instead. Any owner or bootstrap rollback fence that observes a `caught_up` state MUST also transition its lifecycle to `bootstrapping` before preflight can fail, so current readers fail closed while the rebuild is incomplete.
- The SQLite source journal MUST persist an additive `fact_date_semantics_version` column on each change event. Existing rows and writers that do not know the column MUST remain marked as legacy version `1`; current writers MUST explicitly publish version `2` while preserving journal `event_version = 1` and `AnalyticsFactV1` identity. A v2 replica consumer MUST validate invocation upsert and tombstone events before opening the target apply transaction; a legacy event MUST leave the target checkpoint and source acknowledgement unchanged, CAS-invalidate the observed replica state, and require an explicit bounded bootstrap rebuild.
- Explicit bootstrap MUST use the dedicated maintenance connection, persist the shared legacy-visible eligibility fence and transition a `caught_up` state to `bootstrapping` immediately after acquiring the bootstrap lock and before schema/source/history preflight, clear target facts, applied-change state, parallel-work keys, and the target checkpoint before rebuilding with the current fact-date semantics. The owner-applied compatibility contract MUST also set that shared eligibility fence false and move a `caught_up` state to `bootstrapping` before the explicit rebuild. The rebuild MUST be serialized by a PostgreSQL session advisory lock held on the same connection as the target writes, including when the configured pool has one connection, MUST close that connection if the future is cancelled while the lock is held, and MUST release that connection before the final status report. Neither the runtime application role nor the maintenance role may execute schema DDL.
- Analytics read admission and all fact/key queries contributing to one PostgreSQL response MUST execute in one `REPEATABLE READ, READ ONLY` PostgreSQL transaction. A reader that starts before the fence MUST observe one complete pre-fence snapshot; a reader that starts after the fence MUST fail closed on the non-`caught_up` state. This prevents an in-flight reader from observing a partially cleared or rebuilt target or a response assembled from mixed snapshots.
- The owner-applied compatibility contract MUST be applied as one transaction. If any schema or fence statement fails, the transaction MUST roll back the compatibility change as a unit; the runbook MUST NOT direct operators to execute the schema contract statements individually.
- Legacy-semantics invalidation MUST compare the observed semantic version, status, source generation, and checkpoint before writing invalidated state, then reload the current state. It MUST NOT overwrite a state that has advanced through bootstrap or another compatible transition.
- `AnalyticsFactV1` identity and journal event version `1` MUST remain unchanged. Controlled archive replay is limited to the fenced PostgreSQL bootstrap rebuild and MUST NOT alter Summary All, public query responses, or direct rollups.

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

### VER-PG18-006

- Method: deterministic PostgreSQL schema SQL, catalog-inspection, state-eligibility, compare-and-swap, advisory-lock, and bounded fact-ingestion contract checks, plus an explicitly provisioned PostgreSQL fixture test for role separation and target reset.
- covers: `REQ-PG18-006`
- Pass condition: version `2` is required for eligibility; missing/legacy state fails closed; the source journal marks old writers as version `1` and current writers as version `2` without changing journal event version `1`; a legacy invocation event is rejected before target DML, leaves the checkpoint and source acknowledgement unchanged, and CAS-invalidates the observed state; owner-applied compatibility SQL and bootstrap preflight persist the shared legacy-visible eligibility fence false and move a previously caught-up state to `bootstrapping` before a rebuild can fail; the shared legacy-visible eligibility flag remains false for v2 state; owner compatibility DDL is atomic; each composed PostgreSQL response uses one repeatable-read read-only transaction for admission and all target queries; schema-owner DDL is separate from maintenance DML; the maintenance grant split permits only identity-column reads needed by conflict/tombstone DML, denies fact-content/metric reads and state insert/delete, and denies rollup DML; explicit bootstrap cleanup and checkpoint reset are represented; completed invocation archives are replayed through the bounded verified loader and unreadable or schema-incompatible archives keep the target fenced; catalog inspection is compatible with the least-privilege role; bootstrap serialization, cancellation-safe connection closure, and single-connection release are explicit; stale invalidation cannot overwrite advanced state; fact and journal version `1` identity remains unchanged; controlled archive replay does not change query/rollup surfaces.

## Related ADRs

- [ADR 0051: Optional PostgreSQL analytics offload](../../adr/0051-pg18-analytics-offload.md)

## Visual Evidence

- None. This foundation does not change an owner-facing UI surface.

## References

- `./IMPLEMENTATION.md`
- `./HISTORY.md`
- `./assets/persistent-state-migration-record.json`
- `./assets/version-impact-record.json`
- [`5k89c-long-term-usage-analytics`](../5k89c-long-term-usage-analytics/SPEC.md)
- [`9aucy-db-retention-archive`](../9aucy-db-retention-archive/SPEC.md)
