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

## 验证记录

- 已落地 `src/long_term_stats.rs` 的三维小时/日汇总、可恢复 live/archive 回填、准备状态进度、overview/series API 与墙时区间并集。
- 已落地 `pool_upstream_accounts.deleted_at` 迁移、API Key 凭据/会话/路由运行状态清理，以及账号池/路由候选隐藏。
- 已落地独立 `LongTermStatsSection`、60 秒可见刷新 hook、mock demo handler、Storybook ready/preparing/empty/error 状态与关键 play。
- 已通过：`cargo check --locked`、`cargo test --locked long_term_stats`、前端生产 `bun run build`、定时刷新 hook 测试与变更文件 Biome 检查。
- 已完成完整质量门禁与桌面/移动视觉证据；最终图片写入 `SPEC.md` 的 `## Visual Evidence`，聊天回图使用不可变快照。
