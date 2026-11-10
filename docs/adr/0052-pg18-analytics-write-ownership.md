---
status: accepted
---

# ADR 0052: PostgreSQL analytics projection ownership cutover

长期 analytics 的 SQLite projection、PostgreSQL replica 和 source journal 需要一个单一 serving owner，否则 handoff、回滚和清理会把同一查询拆成混合快照。我们决定先让 PostgreSQL 接管 facts、parallel-work keys、daily/hourly rollups、interval/read models 及其维护状态的 analytics projection serving；当前阶段的业务事实和 source journal 仍由 SQLite 保存，最终业务事实迁移与 SQLite 清退作为后续独立阶段。

## Decision

- 使用独立的 `ANALYTICS_OWNERSHIP_V2=disabled|enabled` 控制本阶段 ownership；`ANALYTICS_BACKEND` 保持既有 read/replay 语义。切换只通过同一应用容器重启完成，不启动第二套完整 writer，不使用手工 SQL 或隐式 fallback。
- 只有所有表级 PG target、generation/checkpoint、source/archive proof、identity/digest、projection parity 和 serving fence 同时成立时，才能 CAS 发布 PostgreSQL owner。任何缺项都保持 shadow 或 `AnalyticsUnavailable`。
- PG serving 期间保留一套由同一 Journal 驱动的 SQLite rollback projection，rollback window 为 1 天。rollback projection 不参与正常查询，也不拥有业务事实。
- Journal 与 rollback projection 使用按最近生产数据估算的自动容量策略，同时受事件数和字节数约束。容量接近边界时告警；达到硬上限前触发 backpressure，禁止淘汰未消费事件、跳过 replay 或伪造 coverage。
- handoff 后 observation window 与 rollback window 合并为 1 天。通过 parity、重启/restore/replay、持续 serving 健康和 Journal 无缺口等证据后，cleanup gate 才能删除 SQLite 派生表、rollback projection 和旧 projection state。
- handoff、rollback proof、PG 连接、schema、checkpoint 或 parity 失败时 fail closed；读取不得混合 SQLite 与 PG，也不得在 PG 失败后自动退回重型 SQLite analytics 查询。

## Consequences

本阶段把“analytics projection serving owner”和“业务事实 authority”分开，允许先验证 PG projection 的完整性，再安排未来业务事实迁移。一天 rollback window 限制了临时 SQLite 存储和 Journal retention 的最低要求；容量不足会造成受控延期或 unavailable，而不是静默丢失事实。未来业务事实迁移和 SQLite 完全清退必须另行记录 source-migration ADR 与持久状态迁移合同。

## Related documents

- [PostgreSQL analytics projection ownership cutover Spec](../specs/pg18-analytics-ownership-v2/SPEC.md)
- [PostgreSQL analytics write ownership cutover design](../design/pg18-analytics-write-ownership-cutover.md)
- [ADR 0051: Optional PostgreSQL analytics offload](0051-pg18-analytics-offload.md)
