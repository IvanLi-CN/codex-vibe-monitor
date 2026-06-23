# Dashboard 自然日七卡 KPI 语义与布局重构 实现状态（#gz5ns）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: 已实现
- Lifecycle: active
- Catalog note: natural-day KPI semantics/layout follow-up for Dashboard + account detail reuse

## Coverage / rollout summary

- 已完成 `TodayStatsOverview` 七卡四区布局重构，`较昨日` 统一移动到右上，底部左右辅助位统一改为 inline `label + value`。
- 已完成 summary augmentation 字段扩展：strict in-progress retry、进行中等待均值、失败/中断 cost 与 tokens，同时覆盖全局 Dashboard 与 `upstreamAccountId` 账号作用域。
- 已完成 natural-day timeseries `nonSuccessCost` 契约扩展，以及 `DashboardTodayActivityChart` 金额模式从单累计面积图切换为 `Success + Non-success` 堆叠累计面积图。
- 已完成 Dashboard / 账号详情复用链路、账号活动总览 Storybook 场景与视觉证据落盘，以及前后端 targeted tests。

## Remaining Gaps

- None.

## Related Changes

- `src/api/slices/invocations_and_summary.rs`
- `src/stats/mod.rs`
- `src/tests/slices/pool_failover_window_h.rs`
- `src/api/slices/prompt_cache_and_timeseries/timeseries.rs`
- `web/src/components/TodayStatsOverview.tsx`
- `web/src/components/DashboardTodayActivityChart.tsx`
- `web/src/components/dashboardTodayActivityChartData.ts`
- `web/src/components/DashboardActivityOverview.stories.tsx`
- `web/src/components/DashboardTodayActivityChart.stories.tsx`

## References

- `./SPEC.md`
- `./HISTORY.md`
