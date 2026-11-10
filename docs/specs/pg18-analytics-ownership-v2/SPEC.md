# PostgreSQL Analytics Projection Ownership Cutover

> 本主题定义 analytics projection 的第一阶段 PostgreSQL ownership cutover；当前阶段的业务事实和 source journal 仍由 SQLite 保存，最终业务事实迁移与 SQLite 清退属于后续独立阶段。

## Related ADRs

- [ADR 0051: Optional PostgreSQL analytics offload](../../adr/0051-pg18-analytics-offload.md)
- [ADR 0052: PostgreSQL analytics projection ownership cutover](../../adr/0052-pg18-analytics-write-ownership.md)

## Context and Scope

长期 analytics 同时存在 SQLite projection、PostgreSQL replica 和 source journal。若没有统一 serving owner，handoff、rollback 和 cleanup 可能让一个响应混合不同 generation 或不同存储的快照。

本主题覆盖 facts、parallel-work keys、daily/hourly rollups、interval/read models 及其 repair、bucket、publication、checkpoint 状态的 PostgreSQL serving ownership。`Summary All`、live overlay、业务事实、原始 payload、archive source 和 archive maintenance metadata 不属于本次 cutover。

本主题的阶段性边界不是最终数据架构。后续阶段会把业务事实和 source journal 的写入权迁移到 PostgreSQL，并在事实、analytics projection、archive source 和恢复证据全部迁移后清退 SQLite；该迁移必须另行定义 source-migration 合同。

## Goals

- 让一个 durable owner epoch 和 serving fence 决定长期 analytics 的唯一读取来源。
- 在 PG serving 前完成 source/archive、snapshot/journal、identity/digest 和 projection parity proof。
- 保留一天可提升的 SQLite rollback projection，并在 cleanup gate 通过后安全清理。
- 在 Journal 或 rollback 容量不足前触发 backpressure，禁止丢失未消费事件或伪造 coverage。
- 让重启、handoff 中断、PG 故障、rollback rebuild、restore/replay 和 cleanup 都可恢复、可观测、可重入。

## Non-goals

- 本阶段不迁移 `codex_invocations`、`pool_upstream_request_attempts` 等业务事实的写入权。
- 本阶段不改变 `Summary All`、live overlay、raw payload、archive source 或 archive maintenance 的 serving 语义。
- 本阶段不通过自动 fallback 把 PG 失败转换成重型 SQLite analytics 查询。
- 本阶段不定义最终业务事实迁移到 PG 的 schema、双写窗口或 SQLite 完全退役步骤。

## Requirements

### REQ-PG18-OWN-001 — Phase boundary and future end state

当前阶段 MUST 保持 SQLite 业务事实和 source journal 的写入权，同时允许 PostgreSQL 接管长期 analytics 派生 projection 的写入、维护和查询。文档 MUST 明确未来业务事实迁移到 PostgreSQL 并清退 SQLite 是独立后续阶段，不得把阶段性 SQLite authority 描述成永久架构。

### REQ-PG18-OWN-002 — Independent ownership switch

系统 MUST 使用独立的 `ANALYTICS_OWNERSHIP_V2=disabled|enabled` 控制本主题的 owner lifecycle，默认值为 `disabled`；`ANALYTICS_BACKEND` 的既有 read/replay 语义 MUST 保持不变。开关只在同一应用容器重启时生效，禁止通过第二个完整业务容器、手工 SQL、隐式 fallback 或双 writer 完成切换。

启动 MUST 读取 durable owner state：`disabled` 遇到 `postgres_owned` 时先执行受控 rollback，`enabled` 遇到 `sqlite_owned` 时先完成 shadow/parity/handoff；状态不一致时 analytics MUST 保持 unavailable。

### REQ-PG18-OWN-003 — Complete table-level owner claim

PG owner claim MUST 同时覆盖 facts、parallel-work keys、daily/hourly rollups、interval/read models 及其 repair、bucket、publication 和 checkpoint state。任一 PG target、schema、generation、checkpoint、parity 或 serving fence proof 缺失时，系统 MUST 保持 shadow 或 unavailable，不得宣称 PostgreSQL 已成为 analytics owner。

### REQ-PG18-OWN-004 — Journal replay and idempotent projection writes

SQLite source transaction MUST 继续原子写入业务事实和 source journal。PG maintenance writer MUST 按连续 generation 和 change interval replay Journal，并在同一 PG transaction 中提交 projection DML、`(projection, change_id)` applied ledger、source identity 去重和 checkpoint；重复事件 MUST 幂等，缺口、跨代跳跃或 retention gap MUST 阻止 checkpoint 前进。

### REQ-PG18-OWN-005 — Single serving fence and snapshot contract

owner epoch、generation、serving ack 和 retention floor MUST 组成一个可 CAS 的 serving fence。handoff 前 PG MUST 覆盖切换点 `Jc`，所有 serving projections MUST 使用同一 generation、revision 和 fence；CAS 未提交或 fence 不一致时不得读取 PG。

一次 analytics 响应涉及的所有 PG fact/key/read-model 查询 MUST 在同一个 `REPEATABLE READ, READ ONLY` transaction 中执行。读取 MUST 使用一个 serving fence，禁止混合 SQLite 和 PG，也禁止 PG 失败后隐式退回重型 SQLite analytics 查询。

### REQ-PG18-OWN-006 — One-day rollback projection

PG serving 期间 MUST 从同一 source journal 持续维护独立的 SQLite rollback projection，覆盖本阶段所有 PG 派生对象；它不参与正常 serving、不拥有业务事实，也不写 PostgreSQL 表。`rollback_window` 固定为 1 天。

rollback projection proof 不完整时，rollback MUST 改走 source rebuild；rollback 或 rebuild 未完成前 analytics MUST 保持 unavailable。rollback projection 只能在 cleanup gate 通过后删除。

### REQ-PG18-OWN-007 — Automatic capacity and backpressure

Journal 和 rollback projection MUST 使用自动容量策略：系统根据最近生产数据估算完整 1 天 rollback window 的峰值，并加安全余量；容量同时受事件数和字节数约束。实现 MAY 将估算结果物化为可观测配置，但不得要求 owner 预先猜测静态容量数字。

容量接近安全边界时 MUST 告警；达到硬上限前 MUST 触发 backpressure，禁止淘汰尚未被 PG 消费的 Journal 事件、跳过 replay、推进虚假 coverage 或删除 rollback projection。容量不足时 source 写入按既有业务优先级受控延期，analytics 保持 unavailable。

### REQ-PG18-OWN-008 — Proof and parity gates

owner claim MUST 通过 source/archive manifest、SHA、schema、bounds、row count、snapshot/Journal 连续性、generation、source identity、canonical tuple digest、facts、keys、daily/hourly rollups、interval/read models parity proof。任何 proof 失败 MUST 阻止 fence 发布并保留可重试状态。

`observation_window` 与 `rollback_window` 合并为 1 天。cleanup gate MUST 同时具备 handoff parity、至少一次 restart/restore/replay、持续 serving 健康、Journal 无缺口、rollback capacity 和 backup evidence；仅 owner CAS 成功不得授予清理权限。

### REQ-PG18-OWN-009 — Fail-closed lifecycle

handoff、rollback proof、PG connection、schema、checkpoint、generation、parity 或 capacity 失败 MUST 暴露稳定的 `AnalyticsUnavailable`。读取不得返回 fabricated zero、partial projection 或 mixed snapshot；Journal MUST 保持可重放，直到 rollback 或 source rebuild 完成。

### REQ-PG18-OWN-010 — Future source migration boundary

后续业务事实迁移 MUST 使用独立的 schema、双写/切换、事实保留、恢复和清退合同。不得通过本主题的 cleanup gate 删除业务事实或 source journal，也不得把 rollback projection 的删除解释为 SQLite 业务存储已经完成迁移。

### REQ-PG18-OWN-011 — Operational closeout evidence

部署、观测窗口、回滚、PG 故障恢复、容量背压和 restore/replay MUST follow the [cutover runbook](../../design/pg18-analytics-ownership-cutover.md)。迁移确认后 SQLite rollback snapshot MUST remain available for one complete day. Cleanup MUST retain an auditable evidence bundle covering owner CAS, parity, one-day observation/rollback, restart/restore/replay, Journal continuity, capacity, backup and deletion targets; missing evidence MUST leave the snapshot in place. Final SQLite business-fact retirement MUST be a separate source-migration delivery.

## Verification

### VER-PG18-OWN-001

- Method: 文档、配置状态和 durable owner state contract review。
- covers: `REQ-PG18-OWN-001`, `REQ-PG18-OWN-002`。
- Pass condition: 阶段性 SQLite authority、最终 PG end state、独立 V2 开关、重启切换和不一致 unavailable 行为均有明确合同。

### VER-PG18-OWN-002

- Method: schema-to-target mapping、generation/checkpoint、applied ledger 和 owner CAS contract review。
- covers: `REQ-PG18-OWN-003`, `REQ-PG18-OWN-004`, `REQ-PG18-OWN-005`。
- Pass condition: 所有列出的 PG target 均有对应 state、连续 replay、幂等 ledger、single fence 和 single-snapshot read contract。

### VER-PG18-OWN-003

- Method: 一天 rollback window 的容量计算、backpressure、rollback rebuild 和 cleanup gate scenario review。
- covers: `REQ-PG18-OWN-006`, `REQ-PG18-OWN-007`, `REQ-PG18-OWN-008`。
- Pass condition: 自动容量至少覆盖完整一天并含安全余量；容量压力不会淘汰未消费事件；cleanup 只在全部 evidence 通过后发生。

### VER-PG18-OWN-004

- Method: fault-injection and restart matrix。
- covers: `REQ-PG18-OWN-009`, `REQ-PG18-OWN-010`。
- Pass condition: handoff interruption、PG outage、rollback proof failure、restore/replay failure 和 future source migration boundary 均 fail closed 且可重入，业务事实不会被本主题清理。

### VER-PG18-OWN-005

- Method: runbook and document-contract review using `.github/scripts/check_pg18_analytics_ownership_docs.py`.
- covers: `REQ-PG18-OWN-006`, `REQ-PG18-OWN-007`, `REQ-PG18-OWN-008`, `REQ-PG18-OWN-009`, `REQ-PG18-OWN-010`, `REQ-PG18-OWN-011`.
- Pass condition: deployment switch, one-day observation/rollback, fail-closed PG recovery, automatic capacity/backpressure, replay order, one-day snapshot retention and separate SQLite retirement boundary are all present and auditable.

## Related implementation surfaces

- `src/analytics_changes.rs`
- `src/long_term_stats/schema.rs`
- `src/long_term_stats/intervals.rs`
- `src/long_term_stats/projection_rebuild.rs`
- `src/pg_analytics/schema.rs`
- `src/pg_analytics/read.rs`
- `src/pg_analytics/mod.rs`
- `src/config.rs`

## Visual Evidence

None. This topic changes persistence and serving ownership contracts, not an owner-facing UI surface.
