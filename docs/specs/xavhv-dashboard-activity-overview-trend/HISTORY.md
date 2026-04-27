# Dashboard 活动总览自然日趋势增强 演进历史（#xavhv）

> 这里记录会影响 Agent 理解“为什么一步步变成现在这样”的关键演进；单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- 2026-04-27: 主人确认新增自然日 `趋势` 图，均线口径采用 `1 分钟原值`，中文命名固定为 `消费速率`，快车道推进到 PR merge-ready。
- 2026-04-27: PR #375 跟进确认自然日 `趋势` 与 `首字总耗时` 视觉降密度：趋势改为 10 分钟聚合面积图，次数图中的 `首字总耗时` 改为 10 分钟加权平均细线。

## Key Reasons / Replacements

- `2qsev` 的 KPI 速率历史口径使用 5m 均值和旧金额每分钟命名；本 spec 固化后续自然日趋势增强的最终命名与图表口径。
- 1 分钟趋势点在自然日全轴上过密且不利阅读；本 follow-up 保留后端 1 分钟 timeseries 契约，只在前端 chart-only 层做 10 分钟显示聚合。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
