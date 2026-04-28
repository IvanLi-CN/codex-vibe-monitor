# Dashboard 活动总览趋势图增强 实现状态（#a6pm6）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: 已实现，待 PR / CI / review-proof 收敛
- Lifecycle: active
- Catalog note: fast-track / Dashboard activity overview natural-day trend chart

## Coverage / rollout summary

- 自然日图表已支持 `趋势` 模式；`今日 / 昨日` 可显示 TPM 与消费速率 10 分钟增量面积图。
- `次数` 图已保留成功 / 失败 / 进行中分钟级柱状结构，并叠加 10 分钟降采样的低权重 `首字总耗时` 曲线。
- `金额/分钟` / `Cost/min` UI 与测试语义已统一为 `消费速率` / `Spend rate`。
- Storybook 与 mock-only 视觉证据已归档在 `./SPEC.md` 的 `## Visual Evidence`。

## Remaining Gaps

- 待完成 follow-up PR、CI、review-loop 与主干合入收敛。

## Related Changes

- None

## References

- `./SPEC.md`
- `./HISTORY.md`
