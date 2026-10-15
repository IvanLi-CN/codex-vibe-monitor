# Terminal Projection 中间层与增量物化演进记录

4.6.0 首次线上升级因完整 legacy 日志中的浮点重新编码校验误报而回滚。后续修复按主人要求采用历史坏帧隔离、尾部残片忽略和 warning，并保持新事件的真实日志确认。ADR 0039 取代 ADR 0037 的主日志历史损坏启动失败边界；遗留隔离原件不自动覆盖活动数据，未知版本及可靠写入不可用仍保留来源并报告错误。

PR #1110 的有限审查修复将两处重试失败 capture 移到终态快照构造前，并更新实际 pending 记录，避免完成文件只存在于随后丢弃的副本中。终态物化回归覆盖有界 capture 拒绝后的完整 raw 元数据和零请求侧 SQLite 写准入；分页与 locate 的纯持久排序同时恢复兼容。日志格式、schema、硬容量与既有压力预算保持不变。

PR #1110 的有限原始日志收尾修复以真实单槽回归证明旧顺序可能阻止终态生产者继续。收尾同时轮询请求与响应写入，在保留两份完整日志后进入原有终态确认和物化流程；原有 32 请求、确认屏障、零生命周期 SQL 与延迟断言保持不变。

## 关键决策

- ADR 0038 细化主日志终态容量预留，避免小请求在实际磁盘仅约 214 KiB 时被固定 4 MiB 额度限制到 127 个并发。保留 512 MiB 硬上限、关键终态恢复义务及原写锁压力验收，不更改 journal 格式、SQLite schema 或派生任务边界。

- PR #1110 的 shutdown terminal 补救恢复采用主人明确要求的隔离与告警策略，尾部半帧忽略，不因旧补救文件直接阻止服务启动；ADR 0037 取代 ADR 0035 的无条件启动失败规则。终态 attempt 提交前保持有界读投影，提交 ACK 才释放，避免异步物化期间查询缺口。
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
