# 数据保留维护任务归属背景

## Lifecycle / Compatibility

本主题建立四项维护的独立执行归属与自动触发控制。既有任务标识、归档 URL 和旧组合历史继续可读；新增清理任务使用独立标识。`RETENTION_ENABLED` 和 `XY_RETENTION_ENABLED` 完全退出配置输入，不做停用状态兼容导入，因此公开配置合同存在行为变更。

任务停用在本主题中表示暂停自动触发；显式手动运行仍允许，已开始的有界运行保持正常范围。该语义接管这四项任务的旧“禁用后拒绝立即运行”或“禁用后截断当前轮次”规则。新控制默认允许自动触发，已有持久控制和管理员计划保留。

## Replacements / Background

- 主人已确认完整需求集合，关联 ADR 已接受；已按主人明确批准的实现计划进入实现与验证。
- [Bounded retention runs](../bounded-retention-runs/SPEC.md) 继续拥有归档预算、正确性和资源合同。本主题接管其中会话派生维护的归档阶段归属、跨任务组合完成度和自动触发暂停的含义；不整体取代该主题。
- [Invocation identity](../proxy-invocation-identity/SPEC.md) 与 [raw retention recovery](../autonomous-retention-recovery/SPEC.md) 继续拥有身份和文件删除安全。本主题调整执行者归属，不放宽保护条件。
- [ADR 0034](../../adr/0034-retention-maintenance-task-ownership.md) 保存四项拆分、弃用环境输入、默认值、调度和 CLI 范围的取舍；词汇由根目录 `CONTEXT.md` 定义。

## Related Changes

- 文档在签名提交中保留；实现位于 `th/retention-maintenance-boundaries`，同一 [PR #1092](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/1092) 交付至 merge-ready。
- 直接兼容验证按独立发布来源执行；覆盖 v4.0.0–v4.0.6、v4.1.0–v4.1.3、v4.2.0 至 v4.2.2，不依据格式猜测合并来源，最终候选结果见 IMPLEMENTATION.md。
- PR Linux CI 暴露首次启动后 inode 补锁排序与路由不一致，修复统一排序，并保留初始化／离线 CLI 角色，避免数据库对锁提前发布 ready；新增 raw dry-run 回归移至独立批次测试文件，遵守既有源文件预算。

- 后续修复在 SQLite 打开前取得实际数据库文件及规范化路径／inode 对锁；在线准入同时验证 ready 角色、路径对与当前 inode，拒绝日志路径不确定的 hard-link 别名和离线多链接数据库。
- 同步主干的分页、实时 overlay 与统计公平队列；保留公平准入、源围栏、诊断及对应测试，仅按已批准合同固定维护轮次准入后的停用语义。
- 主干新增另一份 ADR 0033 后，本任务已批准的决策文件仅顺延为 ADR 0034 并更新引用，决策正文未变更。
- 正式并发审查发现既有 inode 对锁的快速返回遗漏路径／描述符重查，且 SQLite 随后可能打开替换后的文件；主人明确授权第四批修复。修复保留现有运行态独占合同，将校验接到 SQLite 原生打开边界，回归覆盖业务库、维护库、路径恢复后的不同原生句柄、后续连接、归档 ATTACH、硬链接和锁释放。

## References

- [Requirements](SPEC.md)
- [Implementation coverage](IMPLEMENTATION.md)
