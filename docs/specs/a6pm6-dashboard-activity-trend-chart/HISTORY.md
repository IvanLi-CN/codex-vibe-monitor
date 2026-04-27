# Dashboard 活动总览趋势图增强 演进历史（#a6pm6）

> 这里记录会影响 Agent 理解“为什么一步步变成现在这样”的关键演进；单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- 2026-04-27: 主人锁定 fast-track，要求 `今日 / 昨日` 增加 `趋势`，中文统一为 `消费速率`，趋势口径使用 1 分钟原值，不做 5m / 15m 平滑。

## Key Reasons / Replacements

- 本 spec 作为 `#r99mz`、`#mpgea` 与 `#2qsev` 的自然日活动总览 follow-up，专门承载趋势图与命名统一，避免继续扩大旧 KPI spec 的主题边界。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
