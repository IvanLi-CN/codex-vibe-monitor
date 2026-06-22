# Dashboard 自然日七卡 KPI 语义与布局重构 演进历史（#gz5ns）

> 这里记录会影响 Agent 理解“为什么一步步变成现在这样”的关键演进；单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- 2026-06-22：创建 active spec，冻结自然日七卡的四区布局、`较昨日` 统一右上、以及 Dashboard 与账号详情共用同一 KPI 语义的边界。
- 2026-06-22：明确本轮继续走 `summary` / SSE `summary` 快路径，新增增强字段而不是前端独立 KPI 轮询。

## Key Reasons / Replacements

- 历史 `#r99mz` 与 `#2qsev` 负责把今日 KPI 并入总览并完成 7 tile 扩展，但没有冻结“右上 comparison + 右下语义位 + account-scoped reuse”这一层长期契约，因此需要新的 active topic spec 承接。
- 账号详情统计 read-model 已由 `#t6d9r` 收紧为 account-scoped 准确读路径；本 spec 只在此约束上补齐自然日 KPI 的显示语义，不替代 `#t6d9r` 的 read-model 范围。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
