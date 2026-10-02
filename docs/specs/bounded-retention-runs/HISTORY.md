# 有预算、可恢复的 Retention：主题关系与兼容性

## Lifecycle / Compatibility

主题为 active。基础实现已在 v2.82.0 发布，当前候选在保留旧 API status 的基础上增加可选追赶/小时观测字段；新的持久化续作状态采用前向修复，不承诺较旧 Minor 程序维护新状态的恢复语义。原 shared-testbox 三个 profile、Prompt 百万行对照、v2.71.45/v2.80.2 前向修复夹具与 Demo 视觉确认已有交付记录；当前 head 的固定 cohort harness 单次观察达到 1,300,000 行归零，A7 三次基线/候选容量门槛、迁移、审查和新图表视觉证据仍待交付门禁收敛。

线上少量提交不能证明积压消化有效，新增 REQ-BRR-016..021 明确自动追赶、24 小时固定存量目标、准确等待原因和独立 7 天小时历史。当前分支已形成对应候选实现，但容量、迁移、审查和新图表视觉证据仍是交付门槛；旧测试和迁移记录不得沿用为新需求的通过证据。新增小时历史不回填升级前数据，旧字段/未观测值保持未知。

## Replacements / Background

本主题补充 retention 的运行预算、会话派生维护边界及任务观测，并不替代既有 archive 证明、保留策略、自主 raw 恢复或全局任务运维数据所有权。

- [归档与保留](../9aucy-db-retention-archive/SPEC.md) 继续拥有归档/汇总证明及删除安全。
- [自主恢复](../autonomous-retention-recovery/SPEC.md) 继续拥有 prepared archive、raw 恢复和 circuit breaker。
- [ADR 0021](../../adr/0021-prompt-cache-background-materialization.md) 继续拥有精确统计的暂不可用读契约。
- [ADR 0022](../../adr/0022-prompt-cache-adaptive-materialization.md) 保留自适应与操作控制，并由 ADR 0025 承接单 key 分页及跨提交续作语义。
- [ADR 0023](../../adr/0023-task-operations-state-outside-main-database.md) 继续拥有独立维护库及异步观测。
- [ADR 0025](../../adr/0025-retention-core-and-conversation-derived-maintenance.md) 记录本主题的新完成边界。
- [ADR 0027](../../adr/0027-retention-catchup-independent-of-inspection-schedule.md) 明确 retention 的自定义计划控制巡检而非限制恢复窗口，补充 ADR 0024 的有效计划展示；不改变其他 Managed Task 的计划语义。既有覆盖值保留，界面须展示该语义及独立追赶资格，禁用仍是阻止后续追赶的控制。

## Related Changes

- `b018ae06`：提交本主题的设计基线与 ADR 0025。
- [PR #1062](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/1062)：最终 topic head `802bbc0da27c791267dfd1240ecc53d60fcaade2`，合并 commit `146ee9124388af96c4a59fca76684eb7c7e3d6f8`，随 v2.82.0 发布。
- 原交付资源 profile：lightweight 1,246/1,246、stateful-sqlite 1,375/1,375、archive-file-io 299/299；Prompt 百万行对照见 [benchmark card](assets/shared-testbox-candidate-benchmark-card.md)。该试验不执行真实 retention 归档，不能证明 24 小时存量消化或在线延迟不劣于基线。
- 原交付 Web Storybook 136 项、任务页 E2E 7/7、Web 单测 1,682 项及 Rust check/Clippy 已通过；本地 Demo 截图已由主人确认准确。
- 实现阶段新增主库统计代次/分页/孤儿 cursor 结构、维护库 nullable 运行观测字段，以及任务页 Demo 状态；具体覆盖和未验证证据见 [具体方案](IMPLEMENTATION.md)。
- 线上 v2.82.1 只读核实保留基础 retention 路径；连续运行的起点积压约 1,257,051 → 1,257,040，新增候选因此实现独立追赶资格、准确停止边界和 7 天小时快照，并要求真实消化速度实测，而非仅验收 partial 成果。
- 当前 head `9f6ab720` 的 shared-testbox retention 容量实测在 1,300,000 行固定 cohort、500,000 行倾斜 key、64 个孤儿 raw 文件和在线读写负载下，38 轮真实归档后观察剩余为 0；15 轮在 60 秒工作预算停止，22 次锁压力重试，在线读 p95/p99 为 63/97 微秒。开发基线首轮在同一锁负载返回 SQLite extended error 517，第二轮无进展，未取得可比较的三次 p95/p99 中位数；完整限制记录见 [capacity card](assets/shared-testbox-retention-capacity-card.md)。
- 当前 head 后端资源 profile 已完成：lightweight 1,275/1,275、archive-file-io 300/300；stateful-sqlite 主跑 1,384/1,385，唯一已有路由超时断言在隔离重跑 1/1 通过。该环境抖动不涉及 retention 代码，原始日志保留在 shared-testbox agent 目录。
- Q1..Q5 确认最长归档逾期、invocation 主图、普通在线负载 24 小时固定存量目标、自动追赶/覆盖巡检语义与每小时末次准确快照；新增 ADR 0027，并在同一主题保留 REQ-BRR-001..015 身份后扩展 016..021。

## References

- [长期需求](SPEC.md)
- [具体方案](IMPLEMENTATION.md)
