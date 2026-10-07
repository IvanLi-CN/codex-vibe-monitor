# 数据保留维护任务归属背景

## Lifecycle / Compatibility

本主题建立四项维护的独立执行归属与自动触发控制。既有任务标识、归档 URL 和旧组合历史继续可读；新增清理任务使用独立标识。`RETENTION_ENABLED` 和 `XY_RETENTION_ENABLED` 完全退出配置输入，不做停用状态兼容导入，因此公开配置合同存在行为变更。

任务停用在本主题中表示暂停自动触发；显式手动运行仍允许，已开始的有界运行保持正常范围。该语义接管这四项任务的旧“禁用后拒绝立即运行”或“禁用后截断当前轮次”规则。新控制默认允许自动触发，已有持久控制和管理员计划保留。

## Replacements / Background

- 主人已确认完整需求集合，关联 ADR 已接受；已按主人明确批准的实现计划进入实现与验证。
- [Bounded retention runs](../bounded-retention-runs/SPEC.md) 继续拥有归档预算、正确性和资源合同。本主题接管其中会话派生维护的归档阶段归属、跨任务组合完成度和自动触发暂停的含义；不整体取代该主题。
- [Invocation identity](../proxy-invocation-identity/SPEC.md) 与 [raw retention recovery](../autonomous-retention-recovery/SPEC.md) 继续拥有身份和文件删除安全。本主题调整执行者归属，不放宽保护条件。
- [ADR 0033](../../adr/0033-retention-maintenance-task-ownership.md) 保存四项拆分、弃用环境输入、默认值、调度和 CLI 范围的取舍；词汇由根目录 `CONTEXT.md` 定义。

## Related Changes

- 文档在签名提交中保留，并对齐发布基线 v4.1.0；实现位于 `th/retention-maintenance-boundaries`，PR 尚未创建。
- v4.0.0–v4.0.6 与 v4.1.0 的发布镜像分别生成了状态夹具；候选升级和完整中断恢复验证尚未完成，源码分析不替代这些证据。

## References

- [Requirements](SPEC.md)
- [Implementation coverage](IMPLEMENTATION.md)
