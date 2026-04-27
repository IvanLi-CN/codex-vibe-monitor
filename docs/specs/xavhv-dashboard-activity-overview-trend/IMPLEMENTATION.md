# Dashboard 活动总览自然日趋势增强 实现状态（#xavhv）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: 已实现，待 PR / CI / review-proof 收敛
- Lifecycle: active
- Catalog note: fast-track / Dashboard activity overview natural-day trend mode

## Coverage / rollout summary

- 自然日范围新增 `趋势` 图表类型。
- `趋势` 图改为 10 分钟聚合面积图，`Tokens` 与 `消费速率` 均在前端 chart-only 层从 1 分钟 timeseries 点求和。
- `次数` 图叠加 10 分钟加权平均后的 `首字总耗时` 细曲线，成功/失败/进行中柱仍保持分钟级明细。

## Remaining Gaps

- 待完成 PR、CI 与 review-loop 收敛。

## Related Changes

- None

## References

- `./SPEC.md`
- `./HISTORY.md`
