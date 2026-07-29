# Stats 长期用量与性能统计实现说明

## 当前实现边界

- 后端持久化、回填、soft-delete、overview/series API 与前端长期统计区均以 `5k89c/SPEC.md` 为契约。
- 现有 Stats 查询、筛选、bucket、SSE 与图表路径保持不变；长期区使用独立 endpoint、hooks 和组件状态。
- 新增 schema 必须兼容旧 SQLite：启动迁移可重复执行，回填状态可恢复，archive purge 只有在长期汇总 target materialized 后才可继续。
- 完整性审计和修复只在长期统计链路运行：初始全量和增量候选日/小时都要先对照 `invocation_rollup_hourly` 的可信终态 overall 证明，再在单个事务内替换所有维度；有界 canonical 增量一律保持未证明，只有完整来源对账才能标记可信；修复队列持久化检测、重试时间和失败原因。

## 计划落点

- `src/schema.rs`：长期小时/日汇总、回填状态和 `pool_upstream_accounts.deleted_at`。
- `src/maintenance/`：live/archive 回填、materialization marker、小时 retention 与日汇总确认。
- `src/api/slices/long_term_stats_api.rs`：overview/series 读取合同和参数校验。
- `src/upstream_accounts/`：API Key 软删除及账号池/同步/路由隐藏。
- `web/src/features/stats/`、`web/src/hooks/`：独立长期区、图表、虚拟化表格与数据 hook。
- `web/src/demo/` 与 Storybook：mock-only 整页场景和可复用片段状态画廊。

## 本次升级

- `LongTermChart` 增加明确的 `line` / `stackedArea` 模式；模型用量和上游账号的 Token、成本、调用次数均使用绝对值堆叠面积。
- 堆叠数据以 `overview.daily` 的完整日期窗口补齐；缺失 point 和已有 `null` 指标均写为零值，使数据岛之间保持连续零基线。自定义 tooltip 同时展示各系列和当日总计，并按 overview 顺序稳定图层、图例和 tooltip；折线图继续保留原始缺失值语义。
- 模型表新增 sticky 全量总计行，身份列改用 `ModelPerformanceModelIdentity`，模型行收紧至约 `40px`，搜索不影响总计。
- Storybook ready fixture 与 mock-only demo fixture 使用独立的模型思考程度字段，覆盖桌面/移动状态和关键交互。
- Storybook `SparseSeries` 与 mock-only demo 的稀疏长期序列覆盖数据岛场景，作为连续零基线的可视化回归入口。

## 验证记录

- 已落地 `src/long_term_stats.rs` 的三维小时/日汇总、可恢复 live/archive 回填、准备状态进度、overview/series API 与墙时区间并集。
- 已落地 `pool_upstream_accounts.deleted_at` 迁移、API Key 凭据/会话/路由运行状态清理，以及账号池/路由候选隐藏。
- 已落地独立 `LongTermStatsSection`、60 秒可见刷新 hook、mock demo handler、Storybook ready/preparing/empty/error 状态与关键 play。
- 已落地长期统计完整性修复：不完整重建会同时从 partial/rebuilt 候选移除；调用归档物理删除前会扫描真实调用，按最晚墙时终点持久化来源安全下界。删除先在一个事务内写入该下界和 `cleanup_state=delete_pending`，归档保持 `completed` 可读；仅在文件删除成功或确认不存在后才删除元数据，失败时保留 `delete_pending` 供后续重试。历史遗留的已缺失文件在既有 materialization、replay 和保留期门槛已经满足时可直接完成元数据收尾，但不会从 manifest 猜测来源下界。请求尝试归档须先把账号映射追溯到可读调用来源，无法复核时不删除归档。暂时不可读的调用归档不再以覆盖日期猜测连续窗口，而是阻断该开始日及之后的候选 UPSERT、保留现有行并返回既有 `error` 状态。canonical 小时表新增与长期口径一致的终态调用/Token/成本证明及 `terminal_proof_complete`。有界增量写入会清除可信标记，后台只有在所有已完成调用 archive 与 live 来源都可读的完整扫描后才恢复标记；每小时审计都会重新扫描来源可用性，任一 archive 后续缺失或不可读、或对账结果不再包含既有可信桶，都会撤销相应终态证明并保持 `error`；该可用性错误会在每次后续刷新立即重试对账。无法完整重建的旧 canonical 行保持原运营值但不参与审计或替换，避免默认零值把历史日误判为空。canonical 空日会清理任一维度的非零残留，同时保留零调用、零 Token、零成本的合法墙时续段。无法证明的已排队日期保留既有行并写入持久化退避，在可执行队列清空前使 API 持续返回现有 `error` 状态；SQLite 锁仅在该后台链路按 `250ms/1s/3s` 有界重试。
- 删除收尾以 `id + dataset + 路径 + SHA-256 + delete_pending` 作为 CAS 身份，并在 `BEGIN IMMEDIATE` 中锁定该身份再删除文件与元数据；legacy writer 在文件替换前把相同 manifest 重新激活，避免重写与收尾竞态删除新归档。
- 已通过：前端 `bun run test`（1310 passed / 6 skipped）、目标组件 Vitest、`bun run build` 与 5 个变更文件 Biome 检查；根级 `lint:web` 仍有既有无关文件错误，未扩大范围修复。
- Storybook interaction/a11y、mock-only `ui_demo` 桌面/移动视觉证据及最终截图 SHA 在本次收口阶段补录到 `SPEC.md` 的 `## Visual Evidence`。
