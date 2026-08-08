# Terminal Projection 中间层与增量物化演进记录

## 关键决策

- P2 pressure defer 与 SQLite retry 分离：派生写使用 250ms 固定合并并按 gate eligibility 唤醒，只有实际执行失败才推进 retry/backoff。

- P1 journal 是 raw terminal durability 边界，不把投影完成当作 journal 回收条件。
- Dashboard 与长期统计共享 terminal admission/ACK 事实，但各自持有 cursor，避免一个消费者的慢恢复阻塞另一个。
- 增量物化持久化每个 rollup key 的 interval segment，并在内存维护规范化区间并集；因此无需把既有 `wall_time_ms` 直接相加，也无需为正常 terminal 流量重建整天。
- System Status 只展示投影健康和积压，不展示调用 payload、调用 ID 或 SQL。
- Retention 保留轻量 live 行时，目标桶 repair 以 `rowId` 合并 archive 详情与 retained columns，不能因相同 ID 去重而丢失 archive 维度。
- 长期统计保留期约束同时覆盖增量写入的 hourly rollup 与 hourly interval 段；daily 历史仍保持永久累计。
- 墙上时间按调用区间并集计算。增量 upsert 保存当前完整并集，重启后先恢复持久区间再接收新 delta，不能改成相加。
- Long-term normal terminal handling stays cursor-incremental even when a recoverable `proxy_interrupted` record later reaches its terminal state; that transition must not restart a natural-day rebuild.
- Cursor-incremental handling is safe only before the cursor has crossed the row. A late terminal transition for an older row queues an exact affected-day repair, preserving totals when requests finish out of insertion order.

# Repair Source Boundaries

- Targeted long-term repairs treat both invocation archives and attempt archives as durable input. A rewritten archive queues its old and new covered dates, and an unavailable archive source defers only that date while preserving the last accepted rollup.
- Archive reads for a targeted repair are bounded to rows intersecting the repaired Shanghai natural day; a repair does not hydrate an entire archive batch merely to discard unrelated rows.

- 内存问题先走观测门禁：进程匿名 RSS、已知组件估算和操作峰值必须分开记录，不能因为 `liveInvocationsCount` 或数据库行数较大就推断存在同等规模的内存对象。未归因占比达到阈值前不启用 allocator 诊断，也不以硬上限方式回收数据。
