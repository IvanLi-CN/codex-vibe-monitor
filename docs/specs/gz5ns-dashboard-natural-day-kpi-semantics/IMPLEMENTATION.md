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
- 已完成 `首字用时` follow-up：主值固定为首字耗时，右下 `响应时间` 改为最近 5 分钟完整调用结束的 `t_total_ms` 均值，并打通后端 `avgTotalMs` 聚合、前端归一化与本地 SSE patch。
- 已完成 `DashboardActivityOverview` Storybook timeseries fixture 的 `avgTotalMs` mock 补齐，活动总览今日桌面态中的 `响应时间` 次指标不再空白。
- 已统一微调七卡主值字号，并刷新单卡裁切图与活动总览桌面态视觉证据。

## Remaining Gaps

- None.

## Related Changes

- `src/api/slices/invocations_and_summary.rs`
- `src/stats/mod.rs`
- `src/tests/slices/pool_failover_window_h.rs`
- `src/api/slices/prompt_cache_and_timeseries/timeseries.rs`
- `src/api/slices/settings_models_and_cache.rs`
- `web/src/components/TodayStatsOverview.tsx`
- `web/src/components/TodayStatsOverview.test.tsx`
- `web/src/components/TodayStatsOverview.stories.tsx`
- `web/src/components/DashboardTodayActivityChart.tsx`
- `web/src/components/dashboardTodayActivityChartData.ts`
- `web/src/components/DashboardActivityOverview.stories.tsx`
- `web/src/components/DashboardTodayActivityChart.stories.tsx`
- `web/src/hooks/useTimeseries.ts`
- `web/src/lib/api/core-foundation.ts`
- `web/src/i18n/translations.ts`

## References

- `./SPEC.md`
- `./HISTORY.md`
