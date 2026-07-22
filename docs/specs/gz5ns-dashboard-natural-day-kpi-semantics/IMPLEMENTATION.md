# Dashboard 自然日七卡 KPI 语义与布局重构 实现状态（#gz5ns）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: 已实现
- Lifecycle: active
- Catalog note: natural-day KPI semantics/layout follow-up for Dashboard + account detail reuse

## Coverage / rollout summary

- 已完成 `TodayStatsOverview` 七卡四区布局重构，`较昨日` 统一移动到右上，底部左右辅助位统一改为 inline `label + value`。
- 已完成 Dashboard open-range / `previous7d` summary `usage_breakdown` 的读路径收口：`today / 1d / 7d / yesterday / previous7d` 改走内部 hourly `model + reasoning` breakdown rollup + exact boundary tail，Dashboard 总览与 comparison summary 的字段、分组与排序保持不变，但不再依赖整段 raw aggregate 重算。
- 已补齐 breakdown rollup 的升级期修补：对历史上已经 `historical_rollups_materialized_at` 的 invocation archive batch，只有 `usage/stats` 这类 legacy account rollup 仍可沿用“materialized 即视为 replayed”的 shortcut；新的 `usage_breakdown` target 若缺真实 hourly rows，会在启动期被重新挂回 historical rollup backlog，而不是继续误判成健康已回放。
- 已完成 summary augmentation 字段扩展：strict in-progress retry、进行中等待均值、失败/中断 cost 与 tokens，同时覆盖全局 Dashboard 与 `upstreamAccountId` 账号作用域。
- 已完成 natural-day timeseries `nonSuccessCost` 契约扩展，以及 `DashboardTodayActivityChart` 金额模式从单累计面积图切换为 `Success + Non-success` 堆叠累计面积图。
- 已完成 Dashboard / 账号详情复用链路、账号活动总览 Storybook 场景与视觉证据落盘，以及前后端 targeted tests。
- 已完成 `首字用时` follow-up：主值固定为首字耗时，右下 `响应时间` 改为最近 5 分钟完整调用结束的 `t_total_ms` 均值，并打通后端 `avgTotalMs` 聚合、前端归一化与本地 SSE patch。
- 已完成 `DashboardActivityOverview` Storybook timeseries fixture 的 `avgTotalMs` mock 补齐，活动总览今日桌面态中的 `响应时间` 次指标不再空白。
- 已统一微调七卡主值字号，并刷新单卡裁切图与活动总览桌面态视觉证据。
- 已完成 `AdaptiveMetricValue` 候选驱动重构：主值不再只在同一单位内裁小数，而是支持完整值、compact、多精度与邻近单位回退的有序候选集。
- 已完成 `TodayStatsOverview` 内主值、右上 comparison/meta、底部 secondary 的结构化自适应数值渲染；红框同类位不再依赖整行 `truncate`。
- 已补齐 `1.05B / 1.0B / 1B / 1,050M` 等 `B/M` 临界值规则，保证在真实窄宽度下优先保留 `1.0B` 等最低必要小数位，而不是视觉上塌成 `1B`。
- 已刷新 `TodayStatsOverview` 与 `DashboardActivityOverview` 的 Storybook 视觉证据，确保取证仅使用仓库支持的 desktop viewport，并去除 story 内部人为 `max-width` 制造的伪窄态。
- 已补齐 tile 级自适应布局退化：当单卡真实宽度不足时，右上 comparison 与底部两个 secondary 会自动下沉到主值下方逐行展示；宽度恢复后回到原四区布局。
- 已完成共享 `AdaptiveDisplayValue` 候选防抖：当前候选只会在真实超宽时降级，只有更高信息量候选额外满足 `6px` headroom 时才升级，重复 `ResizeObserver` / resize 评估不再让同一数值在两种长度之间抖动。
- 已完成共享货币 profile 扩展：`default` 保持累计金额现有非补零语义，`rate` 固定走 `2 位小数 -> 1 位小数 -> 0 位小数 -> compact` 梯度，并把 full 候选固定成两位小数。
- 已完成 `TodayStatsOverview` 的 `消费速率` 主值、`日均`、`每对话` 接入 `rate` profile；`今日成本`、`失败成本` 与其余累计金额调用保持 `default` profile，不扩散 `.00` 风格。
- 已补齐 `AdaptiveMetricValue.test.tsx`、`TodayStatsOverview.test.tsx` 与 `TodayStatsOverview.stories.tsx` 的回归覆盖，并追加 rate 精度 / antijitter Storybook 证据。
- 已调整 `TodayStatsOverview` 七卡顺序：`进行中调用` 位于 `成功` 之前，并同步更新 unit / Storybook 顺序断言。

## Remaining Gaps

- None.

## Related Changes

- `src/api/slices/invocations_and_summary.rs`
- `src/stats/mod.rs`
- `src/tests/slices/pool_failover_window_h.rs`
- `src/api/slices/prompt_cache_and_timeseries/timeseries.rs`
- `src/api/slices/settings_models_and_cache.rs`
- `web/src/features/dashboard/TodayStatsOverview.tsx`
- `web/src/features/dashboard/TodayStatsOverview.test.tsx`
- `web/src/features/dashboard/TodayStatsOverview.stories.tsx`
- `web/src/features/shared/AdaptiveMetricValue.tsx`
- `web/src/features/shared/AdaptiveMetricValue.test.tsx`
- `web/src/features/shared/adaptiveMetricValueSpec.ts`
- `web/src/features/dashboard/DashboardTodayActivityChart.tsx`
- `web/src/features/dashboard/dashboardTodayActivityChartData.ts`
- `web/src/features/dashboard/DashboardActivityOverview.stories.tsx`
- `web/src/features/dashboard/DashboardTodayActivityChart.stories.tsx`
- `web/src/hooks/useTimeseries.ts`
- `web/src/lib/api/core-foundation.ts`
- `web/src/i18n/translations.ts`

## References

- `./SPEC.md`
- `./HISTORY.md`
