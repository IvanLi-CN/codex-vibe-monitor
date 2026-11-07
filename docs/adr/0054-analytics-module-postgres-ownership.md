---
status: proposed
---

# Analytics module 的 PostgreSQL 写入与查询归属

为减少 SQLite 的统计派生写入争用，将 analytics 派生事实、持久读模型、投影修复与统计查询整体交给 PostgreSQL，SQLite 继续拥有业务事实与 Analytics 源 Journal。业务事务以事实与 Journal 在同一 SQLite 事务中提交为完成条件，analytics materializer 异步消费 Journal 并在 PG 内提交派生状态。选择这一边界，是为了明确统计读写归属，并让业务事实在 PG 故障期间仍可继续写入，代价是统计存在异步延迟和独立的不可用状态。

## 后果

- PG 接管后停止 SQLite analytics 派生写入，以减少持续争用和双投影成本；回切 SQLite 时需先重建派生数据，期间统计不可用。owner 已接受这个取舍。
- 历史明细已退役时，允许使用有完整性证明的现存日/小时汇总种子，但只能保留原有聚合能力；缺少证明则阻断切流。owner 已接受这个取舍。

迁移协议、验收要求和待确认边界见[迁移设计](../design/analytics-module-postgres-ownership.md)。本决定仍为 proposed；现有已实现行为继续由 [ADR 0053](0053-pg18-analytics-managed-migration.md) 描述。
