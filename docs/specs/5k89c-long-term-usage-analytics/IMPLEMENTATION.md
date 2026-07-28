# Stats 长期用量与性能统计实现说明

## 当前实现边界

- 后端持久化、回填、soft-delete、overview/series API 与前端长期统计区均以 `5k89c/SPEC.md` 为契约。
- 现有 Stats 查询、筛选、bucket、SSE 与图表路径保持不变；长期区使用独立 endpoint、hooks 和组件状态。
- 新增 schema 必须兼容旧 SQLite：启动迁移可重复执行，回填状态可恢复，archive purge 只有在长期汇总 target materialized 后才可继续。

## 计划落点

- `src/schema.rs`：长期小时/日汇总、回填状态和 `pool_upstream_accounts.deleted_at`。
- `src/maintenance/`：live/archive 回填、materialization marker、小时 retention 与日汇总确认。
- `src/api/slices/long_term_stats_api.rs`：overview/series 读取合同和参数校验。
- `src/upstream_accounts/`：API Key 软删除及账号池/同步/路由隐藏。
- `web/src/features/stats/`、`web/src/hooks/`：独立长期区、图表、虚拟化表格与数据 hook。
- `web/src/demo/` 与 Storybook：mock-only 整页场景和可复用片段状态画廊。

## 本次升级

- `LongTermChart` 增加明确的 `line` / `stackedArea` 模式；模型用量和上游账号的 Token、成本、调用次数均使用绝对值堆叠面积。
- 堆叠数据按日期并集补零，保留已有 `null` 指标；自定义 tooltip 同时展示各系列和当日总计，并按 overview 顺序稳定图层、图例和 tooltip。
- 模型表新增 sticky 全量总计行，身份列改用 `ModelPerformanceModelIdentity`，模型行收紧至约 `40px`，搜索不影响总计。
- Storybook ready fixture 与 mock-only demo fixture 使用独立的模型思考程度字段，覆盖桌面/移动状态和关键交互。

## 验证记录

- 已落地 `src/long_term_stats.rs` 的三维小时/日汇总、可恢复 live/archive 回填、准备状态进度、overview/series API 与墙时区间并集。
- 已落地 `pool_upstream_accounts.deleted_at` 迁移、API Key 凭据/会话/路由运行状态清理，以及账号池/路由候选隐藏。
- 已落地独立 `LongTermStatsSection`、60 秒可见刷新 hook、mock demo handler、Storybook ready/preparing/empty/error 状态与关键 play。
- 已通过：前端 `bun run test`（1310 passed / 6 skipped）、目标组件 Vitest、`bun run build` 与 5 个变更文件 Biome 检查；根级 `lint:web` 仍有既有无关文件错误，未扩大范围修复。
- Storybook interaction/a11y、mock-only `ui_demo` 桌面/移动视觉证据及最终截图 SHA 在本次收口阶段补录到 `SPEC.md` 的 `## Visual Evidence`。
