# Dashboard 活动总览自然日趋势增强 演进历史（#xavhv）

> 这里记录会影响 Agent 理解“为什么一步步变成现在这样”的关键演进；单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- 2026-04-27: 主人确认新增自然日 `趋势` 图，均线口径采用 `1 分钟原值`，中文命名固定为 `消费速率`，快车道推进到 PR merge-ready。

## Key Reasons / Replacements

- `2qsev` 的 KPI 速率历史口径使用 5m 均值和旧金额每分钟命名；本 spec 固化后续自然日趋势增强的最终命名与图表口径。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
