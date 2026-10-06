# 有预算、可恢复的 Retention：主题关系与兼容性

## Lifecycle / Compatibility

主题为 active。基础运行预算与精确 Prompt 维护已在 v2.82.0 发布。自动追赶和 7 天小时观测由 PR #1068 交付，当时主人授权先发布已证明的积极效果，完整常态容量与严格在线延迟尚未签收。

本轮实现任务内批次闭环和高吞吐月度文件更新，独立验证 REQ-BRR-022..023 / VER-BRR-012。此前的阶段交付授权不豁免本轮 50 倍新增速率、固定存量 24 小时消化、安全及在线影响验收。兼容边界是保留主库结构、既有月度归档路径和证明事实，旧吞吐缺失保持未知。

## Replacements / Background

ADR 0032 将归档文件批次与主库写批次区分，规定月份只决定目标文件、每轮完成自身选中批次和不依赖跨任务 staging。它细化 ADR 0027 的自动追赶：后续资格用于处理剩余 live rows，不用于延续上一轮中间状态。本轮新增 REQ-BRR-022..023 及 VER-BRR-012，50 倍新增速率与当前候选容量需要重新证明。

本主题补充 retention 的运行预算、会话派生维护边界及任务观测，并不替代既有 archive 证明、保留策略、自主 raw 恢复或全局任务运维数据所有权。

- [归档与保留](../9aucy-db-retention-archive/SPEC.md) 继续拥有归档/汇总证明及删除安全。
- [自主恢复](../autonomous-retention-recovery/SPEC.md) 继续拥有 raw 恢复和 circuit breaker；ADR 0032 收窄 prepared continuation 在普通 retention 任务中的用途。
- [ADR 0021](../../adr/0021-prompt-cache-background-materialization.md) 继续拥有精确统计的暂不可用读契约。
- [ADR 0022](../../adr/0022-prompt-cache-adaptive-materialization.md) 保留自适应与操作控制，并由 ADR 0025 承接单 key 分页及跨提交续作语义。
- [ADR 0023](../../adr/0023-task-operations-state-outside-main-database.md) 继续拥有独立维护库及异步观测。
- [ADR 0025](../../adr/0025-retention-core-and-conversation-derived-maintenance.md) 记录本主题的新完成边界。
- [ADR 0027](../../adr/0027-retention-catchup-independent-of-inspection-schedule.md) 明确 retention 的自定义计划控制巡检而非限制恢复窗口，补充 ADR 0024 的有效计划展示；不改变其他 Managed Task 的计划语义。既有覆盖值保留，界面须展示该语义及独立追赶资格，禁用仍是阻止后续追赶的控制。

## Related Changes

- 按主人禁止不必要性能测试进入通用 PR 必过集的明确指令，性能预算和 CPU 诊断保持 Actions 辅助检查，Build Artifacts 仅依赖 smoke artifact producer；同步质量门禁合同与自测。历史开销超标、runner 压力缺测和归档容量目标保持未签收，不以删除依赖伪造性能通过；当前交付继续要求普通验证、必需 CI、Tier 4 审查和可合并状态。
- PR #1079 的 `bfc0d23f` 普通 Actions 检查通过，但 run `37436462176` 首次因宿主 I/O 压力无法完成预算测量；原失败作业重跑一次后，六个有效稳定窗口的 CPU/请求开销为 5.61%，超过 5% 门槛，p95 开销 0.64% 通过。继续诊断而非签收性能改善；补齐开关两种模式的同阶段 CPU profile，保持预算与容量要求不变，不开启普通修复批次或正式审查。
- 同步 main 的 PR #1081（v4.0.2）后，保留 Prompt 缓存事件唤醒的启用控制、代次围栏及压力退避，以及按 run 绑定的性能镜像 artifact。主线新增 ADR 0031，本主题 ADR 改为 0032；任务批次、月度目标、删除证明和容量验收不变。旧基线上的 CI 和审查不能证明合入后的候选，需刷新当前 head 的证据。
- PR #1079 的 `f7fa7ea9` 移除稀疏 ID 分支的 `NOT INDEXED` 并更新触发器定义识别。此前将该子句解释为强制全表扫描的判断已撤回：SQLite 仍可访问 rowid。改动仅作为待验证候选，SQL 结构断言不是性能证明；当前 Actions、历史有效预算失败及不稳定窗口分别绑定，根因保持未确认。查询注释和实施记录同步去除已撤回的因果描述。
- PR #1079 的第二个 Repair Batch 补齐 `archiveBatches[].timeoutCount`，将未完成批次触发的执行期限按数据集/月归属，保留整轮兼容值及预算标志；增加确定性轻量回归和旧 JSON/准确零兼容验证。旧 manifest 拥有的隔离记录可能保留 degraded 诊断，作为观测限制记录，不改变删除保护或任务准入。
- [PR #1079](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/1079)：主人将本次停靠点改为 PR 合并，并明确暂不在本地运行 stateful profile，以其他验证及 GitHub Actions 必过检查完成交付。性能测试仅允许在 GitHub Actions，完整容量目标仍待独立验证；不声称 50 倍吞吐、24 小时归零或在线延迟不劣已签收。
- 本次任务内批次的 Tier 4 第一轮归为一个 Repair Batch：修正 gzip ISIZE 回绕造成的空间预检低估、同月较早记录追加导致的保留期缩短，并将新增取消/目录锁测试改为确定性状态断言，去除 wall-clock 性能阈值。修复候选重新交由 Actions 验证并刷新全部五 lane；与此前追赶交付的审查计数分别记录。
- PR #1079 的任务页 SSE 重连 story 将固定延迟断开改为交互步骤先确认已连接再发出 error；保留重连提示和时间线断言。针对性 Storybook 与任务页/SSE 单元验证通过，生产重试逻辑、任务归档契约和已确认吞吐展示不变。
- 同步 main 的 PR #1071 后，保留其外部观测入口与旧性能库退役决定；任务内批次、月度目标、维护库运行吞吐及删除证明保持本主题约定。
- main 的外部观测已发布为 v4.0.0，同 Minor 归档读取来源相应更新；六个读取表面再次逐一比较一致，v3.0/v3.1 指纹保留为较早 Major 历史来源。本主题依然是兼容 patch，完整容量结论仍未签收。

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

- 同步 main 的 PR #1077 后，已分配调用 ID 的生命周期保护继续由既有 range manager 提供；任务内批次和月度目标约定不变。main 已使用 ADR 0029/0030，本主题 ADR 调整为 0031。旧候选的中断容量结果仅保留为历史，新候选重新验证。

- main 的调用 ID 预留功能已发布为 v3.1.0，本主题同 Minor 读取来源相应更新；六个归档读取表面与当前源码逐一比较一致，v3.0.0 留作较早 Minor 来源。此源码证明不替代当前候选运行验收。
