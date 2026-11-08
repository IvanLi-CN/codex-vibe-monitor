---
status: accepted
---

# ADR 0055: Analytics Journal 完整来源载荷的前向存储迁移

`analytics_change_events.fact_json` 是 Analytics 来源 Journal 的持久事实载荷。基线 schema 通过 `CHECK(length(fact_json) <= 65536)` 把单条来源载荷限制在 64 KiB，这个存储限制会截断后续完整来源协议的可用范围。C18 只迁移 Journal 存储边界；typed DTO、接收器、PostgreSQL materializer 和生产容量策略仍属于其他任务。

## 决策

保留历史 Journal 行的全部字段和字节内容，向前迁移到同一列集合的 canonical 表；canonical 表保留 `event_kind` 约束、`change_id` 自增身份、现有索引和兼容列，但移除 `fact_json` 长度检查。历史 DDL/已提交数据不做就地改写，也不使用 `PRAGMA writable_schema`。

迁移入口在兼容列补齐前执行只读 preflight，识别的来源状态限定为 12 列基础表加上任意已知兼容列子集。列类型、非空/主键属性、`event_kind` 约束、已知索引、触发器和外键均需满足已声明合同；未知列、未知索引、未知约束、遗留 rebuild 表或不匹配的 marker 直接 fail closed，且不先执行 `ALTER TABLE`。

未发现已提交 marker 时，迁移在一个取得的 SQLite 连接上执行 `BEGIN IMMEDIATE`，依次完成 marker 表准备、canonical 临时表创建、按 `change_id` 复制所有字段、旧表删除、临时表重命名、索引创建、`sqlite_sequence` 高水位恢复、行 manifest 校验和 marker 写入，并一次提交。复制不解析、不重编码、不规范化 `fact_json`；manifest 同时验证行数、最大 ID、所有持久字段和载荷原字节的摘要。

marker 记录独立的迁移身份、来源/目标 schema fingerprint、来源 sequence、行数、最大 ID、manifest 摘要和提交时间。重启或提交响应未知时，下一次启动只接受“marker 与 canonical schema 同时匹配”的状态并幂等返回；只存在临时表、marker 不完整或 marker 与结构不匹配时拒绝猜测和自动清理。事务中断通过 SQLite rollback 保留旧表，已提交迁移不反向降级，错误版本通过新的前向修复处理。

`analytics_change_state`、coverage、source snapshot 和 retention pin 表不属于 rebuild 对象，因此其既有行、generation、coverage revision、retention boundary 与 pin identity 保持不变。C18 不执行历史事实 backfill，不改变生产定时或锁预算，也不声明测试载荷大小是生产容量承诺。

## 后果

- 新版本可在同一 SQLite Journal 中保存超过 64 KiB、包括超过 1 MiB 的完整来源载荷；旧版本仍可读取这些 TEXT 行，但其旧写入路径继续受自身 64 KiB 检查和应用上限约束。
- 前向迁移需要一次短暂的 SQLite 写事务，并在提交前持有 `BEGIN IMMEDIATE`；失败时保留可诊断的旧状态或完整 canonical 状态，不提供自动 reverse-copy。
- marker、schema fingerprint、manifest 和 `sqlite_sequence` 校验使迁移可重入，但也意味着未知 schema、遗留临时表和不完整提交必须由后续修复或备份恢复处理。

## 验证范围

状态库测试覆盖历史行字段、event kind、state 默认值、空表 sequence 高水位、未知 schema fail closed 和超过 1 MiB 的真实载荷。文件/WAL 测试覆盖旧 12 列状态、兼容列补齐、真实文件重启、marker/schema 幂等识别和 WAL 保持；测试使用单连接隔离 fixture，不把测试时长或锁等待推断为生产预算。

实现与持久化契约见 [`src/analytics_changes/journal_migration.rs`](../../src/analytics_changes/journal_migration.rs)、[`persistent-state-migration-record.json`](assets/analytics-journal-complete-inline-payload/persistent-state-migration-record.json) 和 [`version-impact-record.json`](assets/analytics-journal-complete-inline-payload/version-impact-record.json)。
