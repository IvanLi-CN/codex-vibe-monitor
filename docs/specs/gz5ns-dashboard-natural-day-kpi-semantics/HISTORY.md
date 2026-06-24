# Dashboard 自然日七卡 KPI 语义与布局重构 演进历史（#gz5ns）

> 这里记录会影响 Agent 理解“为什么一步步变成现在这样”的关键演进；单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- 2026-06-22：创建 active spec，冻结自然日七卡的四区布局、`较昨日` 统一右上、以及 Dashboard 与账号详情共用同一 KPI 语义的边界。
- 2026-06-22：明确本轮继续走 `summary` / SSE `summary` 快路径，新增增强字段而不是前端独立 KPI 轮询。
- 2026-06-23：将自然日金额图固定为“累计金额”而非“每分钟金额”语义，并把成本视图改为 `Success + Non-success` 堆叠累计面积；`Non-success` 文案显式承载 `failed + interrupted` 口径。
- 2026-06-23：修正 CRS relay delta 不应污染 `nonSuccessCost` 的口径错误；该旁路只提供总成本与 success/failure 计数，无法安全拆出失败成本时，金额图失败层保持 0 而不是错误抬升。
- 2026-06-23：补齐金额图 i18n，固定领域术语仍为 `Non-success = failed + interrupted`，但 owner-facing 图例与 tooltip 按 locale 正确显示，中文环境使用“非成功”。
- 2026-06-24：将第五张卡主语义固定为 `首字用时`，右下次指标固定为最近 5 分钟完整调用结束的 `t_total_ms` 均值 `响应时间`；不再把进行中等待均值留在 owner-facing 卡面中。
- 2026-06-24：补齐 `avgTotalMs` 的后端 timeseries 聚合、前端归一化与本地 SSE patch，并同步修复 `DashboardActivityOverview` Storybook mock，让活动总览里的 `响应时间` 次指标始终有可验证样本。
- 2026-06-24：统一微调七卡主值字号，并追加活动总览桌面态与单卡裁切的视觉证据，确保这次 follow-up 的 UI 结果可直接复核。

## Key Reasons / Replacements

- 历史 `#r99mz` 与 `#2qsev` 负责把今日 KPI 并入总览并完成 7 tile 扩展，但没有冻结“右上 comparison + 右下语义位 + account-scoped reuse”这一层长期契约，因此需要新的 active topic spec 承接。
- 账号详情统计 read-model 已由 `#t6d9r` 收紧为 account-scoped 准确读路径；本 spec 只在此约束上补齐自然日 KPI 的显示语义，不替代 `#t6d9r` 的 read-model 范围。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
