# 请求记录筛选范围控件与诊断维度增强实现状态（#f2w7m）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付状态与验证事实。

## Current Status

- Implementation: complete
- Lifecycle: active
- Catalog note: records 筛选 IA、复用范围控件、诊断维度扩展、Storybook 与 mock-only visual evidence 已在当前 worktree 落地并完成本地验证。

## Coverage / rollout summary

- 已落地 `DateTimeRangeField` 与 `NumericRangeField`，并在 Records 抽屉中把时间、总 Tokens、总耗时统一收口为单字段范围控件。
- `NumericRangeField` 当前以共享 `Slider` 原语承载双端 slider 主交互，支持字段内当前区间摘要以及与错误状态绑定的可访问性语义；slider 上界使用 records summary 的真实数值域，thumb 与轨道由同一 primitive 对齐渲染。
- `NumericRangeField` 已补充嵌入态 surface，供 Records 范围分组直接复用，避免在分组卡片内再套一层小卡片。
- 已按“范围 / 请求上下文 / 路由与上游 / 结果”四组重构 Records 抽屉，保留 stable snapshot、draft/applied 双态与 chips 摘要语义。
- 已将 Records owner-facing ID 语义收口到短 `invokeId` / `attemptId`，并贯通 `upstreamScope`、`upstreamAccountId`、`proxyDisplayName`、`transport`、`serviceTier`、`reasoningEffort` 的查询、实时回退判定、suggestions buckets 与 owner-facing labels。
- 已增强 `FilterableCombobox` 支持 label/value/searchText 分离，供 `upstreamAccount` 等 suggestion 字段复用。
- 已移除筛选抽屉标题下的冗余说明文案，给首屏筛选内容释放垂直空间。
- 已修正模型筛选器内层标签选择器的 overlay host 继承，避免嵌套 popup 逃到 `body` 层后落到抽屉遮罩下方或跑偏到错误位置。
- 已更新 Storybook mock、Vitest、Rust query/suggestion tests，并产出 mock-only visual evidence；最新证据已刷新为共享 `Slider` 原语版本。

## Remaining Gaps

- 当前 spec 范围内无剩余实现缺口；后续仅剩常规 review / commit / merge 流程动作。

## Validation Facts

- `cargo test --quiet invocation_query_filters_and_schema_migrations -- --nocapture`
- `cd web && bun run test`
- `cd web && bun run build-storybook`

## Visual Evidence

- `./visual-evidence/records-range-diagnostics-drawer.png`
- `./visual-evidence/records-model-filter-nested-selector.png`
- `./visual-evidence/date-time-range-field.png`
- `./visual-evidence/numeric-range-field.png`

## References

- `./SPEC.md`
- `./HISTORY.md`
