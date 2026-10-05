# 有预算、可恢复的 Retention：主题关系与兼容性

## Lifecycle / Compatibility

主题为 active。基础运行预算与精确 Prompt 维护已在 v2.82.0 发布。自动追赶和 7 天小时观测由 PR #1068 交付，当时主人授权先发布已证明的积极效果，完整常态容量与严格在线延迟尚未签收。

本轮实现任务内批次闭环和高吞吐月度文件更新，独立验证 REQ-BRR-022..023 / VER-BRR-012。此前的阶段交付授权不豁免本轮 50 倍新增速率、固定存量 24 小时消化、安全及在线影响验收。兼容边界是保留主库结构、既有月度归档路径和证明事实，旧吞吐缺失保持未知。

## Replacements / Background

ADR 0029 将归档文件批次与主库写批次区分，规定月份只决定目标文件、每轮完成自身选中批次和不依赖跨任务 staging。它细化 ADR 0027 的自动追赶：后续资格用于处理剩余 live rows，不用于延续上一轮中间状态。本轮新增 REQ-BRR-022..023 及 VER-BRR-012，50 倍新增速率与当前候选容量需要重新证明。

本主题补充 retention 的运行预算、会话派生维护边界及任务观测，并不替代既有 archive 证明、保留策略、自主 raw 恢复或全局任务运维数据所有权。

- [归档与保留](../9aucy-db-retention-archive/SPEC.md) 继续拥有归档/汇总证明及删除安全。
- [自主恢复](../autonomous-retention-recovery/SPEC.md) 继续拥有 raw 恢复和 circuit breaker；ADR 0029 收窄 prepared continuation 在普通 retention 任务中的用途。
- [ADR 0021](../../adr/0021-prompt-cache-background-materialization.md) 继续拥有精确统计的暂不可用读契约。
- [ADR 0022](../../adr/0022-prompt-cache-adaptive-materialization.md) 保留自适应与操作控制，并由 ADR 0025 承接单 key 分页及跨提交续作语义。
- [ADR 0023](../../adr/0023-task-operations-state-outside-main-database.md) 继续拥有独立维护库及异步观测。
- [ADR 0025](../../adr/0025-retention-core-and-conversation-derived-maintenance.md) 记录本主题的新完成边界。
- [ADR 0027](../../adr/0027-retention-catchup-independent-of-inspection-schedule.md) 明确 retention 的自定义计划控制巡检而非限制恢复窗口，补充 ADR 0024 的有效计划展示；不改变其他 Managed Task 的计划语义。既有覆盖值保留，界面须展示该语义及独立追赶资格，禁用仍是阻止后续追赶的控制。

## Related Changes

- 同步 main 的 PR #1075（v3.0.0）后，任务批次、月度目标和验收口径保持不变。同 Minor 读取来源更新为 v3.0.0；v2.85/v2.86 指纹保留为历史较早 Major 证据，候选的完整运行验证仍独立执行。

- 同步 main 的 PR #1074（v2.86.0）后，保留其独立任务工作量观测与趋势，并将去重发现量、已提交 invocation 数量和固定起点 cutoff 接入任务内批次路径。旧候选 a53ad60c 的两次完整容量结果只作历史记录；整合后的候选必须重新完成实测、验证和视觉门禁。

- `b018ae06`：提交本主题的设计基线与 ADR 0025。
- [PR #1062](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/1062)：最终 topic head `802bbc0da27c791267dfd1240ecc53d60fcaade2`，合并 commit `146ee9124388af96c4a59fca76684eb7c7e3d6f8`，随 v2.82.0 发布。
- 原交付资源 profile：lightweight 1,246/1,246、stateful-sqlite 1,375/1,375、archive-file-io 299/299；Prompt 百万行对照见 [benchmark card](assets/shared-testbox-candidate-benchmark-card.md)。该试验不执行真实 retention 归档，不能证明 24 小时存量消化或在线延迟不劣于基线。
- 原交付 Web Storybook 136 项、任务页 E2E 7/7、Web 单测 1,682 项及 Rust check/Clippy 已通过；本地 Demo 截图已由主人确认准确。
- 实现阶段新增主库统计代次/分页/孤儿 cursor 结构、维护库 nullable 运行观测字段，以及任务页 Demo 状态；具体覆盖和未验证证据见 [具体方案](IMPLEMENTATION.md)。
- 线上 v2.82.1 只读核实保留基础 retention 路径；连续运行的起点积压约 1,257,051 → 1,257,040，新增候选因此实现独立追赶资格、准确停止边界和 7 天小时快照，并要求真实消化速度实测，而非仅验收 partial 成果。
- 此前追赶交付 head `9a3f915c` 的 shared-testbox retention 容量实测在 1,300,000 行固定 cohort、500,000 行倾斜 key、64 个孤儿 raw 文件、64 个 invocation 关联 raw 行和在线读写负载下，候选三次均观察归零；基线三次 partial 窗口各提交 192 行并剩余 1,299,808 行。候选在线读 p95/p99 中位数为 67/90us，基线为 53/87us，因此合成 fixture 下的 source cohort 消化已有积极证据，但完整常态负载容量和严格延迟不劣未签收；完整日志与限制见 [capacity card](assets/shared-testbox-retention-capacity-card.md)。
- `9a3f915c` 增加容量 benchmark 的有界 partial 模式，使无法在 24 小时内追平的基线仍能输出固定轮数、剩余 cohort、raw link 和在线 p95/p99；默认完整归零验收行为保持不变。
- Owner disposition: 先交付自动追赶、合成固定 source cohort 消化改善和 7 天观测；完整生产常态/峰值重放、在线 p95/p99 严格不劣与进一步优化作为后续独立 PR，不阻断此前 PR 合并发版。长期规范要求仍未完全验收；删除安全、预算、迁移和兼容门禁保留。
- 此前追赶交付 head 后端资源 profile 已完成：lightweight 1,275/1,275、archive-file-io 300/300；stateful-sqlite 主跑 1,384/1,385，唯一已有路由超时断言在隔离重跑 1/1 通过。该环境抖动不涉及 retention 代码，原始日志保留在 shared-testbox agent 目录。
- Q1..Q5 确认最长归档逾期、invocation 主图、普通在线负载 24 小时固定存量目标、自动追赶/覆盖巡检语义与每小时末次准确快照；新增 ADR 0027，并在同一主题保留 REQ-BRR-001..015 身份后扩展 016..021。

## References

- [长期需求](SPEC.md)
- [具体方案](IMPLEMENTATION.md)

- PR #1068 使用 `type:minor` / `channel:stable`，主人授权推进至合并和实际发布；不包含生产部署或本地清理。此前双图 Demo 资产与 v2.82.0 的旧任务页资产分开记录。

- 最终 Tier 4 第一轮发现默认计划恢复、未知积压丢失追赶资格及 observer 准入三个 in-scope 边界；归为同一 Repair Batch，累计使用 2 批。修复后按调度/并发影响刷新所有五 lane，并刷新当前候选的实测与 CI。
