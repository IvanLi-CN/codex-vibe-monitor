# 有预算、可恢复的 Retention：主题关系与兼容性

## Lifecycle / Compatibility

主题为 active。Candidate 已实现新增可选观测字段并保留旧 API status；新的持久化续作状态采用前向修复，不承诺较旧 Minor 程序维护新状态的恢复语义。shared-testbox 三个 profile、同种子百万行对照及 v2.71.45/v2.80.2 前向修复夹具已通过；视觉比较仍决定是否可交付。

## Replacements / Background

本主题补充 retention 的运行预算、会话派生维护边界及任务观测，并不替代既有 archive 证明、保留策略、自主 raw 恢复或全局任务运维数据所有权。

- [归档与保留](../9aucy-db-retention-archive/SPEC.md) 继续拥有归档/汇总证明及删除安全。
- [自主恢复](../autonomous-retention-recovery/SPEC.md) 继续拥有 prepared archive、raw 恢复和 circuit breaker。
- [ADR 0021](../../adr/0021-prompt-cache-background-materialization.md) 继续拥有精确统计的暂不可用读契约。
- [ADR 0022](../../adr/0022-prompt-cache-adaptive-materialization.md) 的批次实现需在有界单 key 分页落地时同步；此前实现规则不能被默认为已经改变。
- [ADR 0023](../../adr/0023-task-operations-state-outside-main-database.md) 继续拥有独立维护库及异步观测。
- [ADR 0025](../../adr/0025-retention-core-and-conversation-derived-maintenance.md) 记录本主题的新完成边界。

## Related Changes

- `b018ae06`：提交本主题的设计基线与 ADR 0025。
- 当前实现位于 `th/retention-bounded-recovery-design`，尚未创建 PR 或发布关联。
- Candidate 资源 profile：lightweight 1,246/1,246、stateful-sqlite 1,375/1,375、archive-file-io 299/299；百万行对照见 [benchmark card](assets/shared-testbox-candidate-benchmark-card.md)。
- Web Storybook 136 项、任务页 E2E 7/7、Web 单测 1,682 项及 Rust check/Clippy 已通过；本地 Demo 截图的视觉比较仍待确认。
- 实现阶段新增主库统计代次/分页/孤儿 cursor 结构、维护库 nullable 运行观测字段，以及任务页 Demo 状态；具体覆盖和未验证证据见 [具体方案](IMPLEMENTATION.md)。

## References

- [长期需求](SPEC.md)
- [具体方案](IMPLEMENTATION.md)
