# Dashboard 用量明细自适应数值显示实现状态

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实。

## Current Status

- Implementation: 已完成本地实现、回归验证与视觉验收
- Lifecycle: active
- Catalog note: Dashboard 用量明细已接入共享自适应候选与项目 Tooltip；长值复用 `usage-breakdown` presentation 的两个千位分隔符硬阈值优先量级显示；表头保持单行并使用紧凑可见标签避免列间重叠；当前进入 follow-up PR 交付。

## Implementation Coverage

- Requirement coverage: `REQ-UBAV-001` 至 `REQ-UBAV-007` 已由共享组件、用量明细调用方、共享 builder 回归测试和 Storybook 状态覆盖。
- Implemented boundary: `UsageBreakdownTooltip` 复用 `AdaptiveDisplayValue` 的可选项目 Tooltip 扩展；两个 Dashboard 调用方继续注入 locale、货币、精度和 Token 单位上下文；未启用扩展的统计卡片保留原生 title 行为。
- Layout boundary: 桌面和移动表头使用单行、不换行的紧凑可见标签；完整标签仍保留在排序按钮的无障碍名称与提示中，表头和按钮均有宽度约束与溢出保护。
- Verification commands: 针对性 Web 单测通过（4 files / 179 tests）；`test-storybook` 通过（36 files / 220 tests，48 skipped）；`typecheck:web`、`lint:web`、Storybook 构建和 Web 构建通过；Storybook canvas 视觉验收已覆盖 `ConstrainedOverlay` 与 `Mobile390`，两者均通过横向溢出检查并观察到带 Token 单位的完整值 Tooltip；已持久化两张 owner-confirmed 视觉证据资产。
- Rollout facts: 不涉及后端 API、数据库 schema、持久化迁移或运行时配置。

## Coverage / rollout summary

- 当前主题只记录前端呈现变更，未产生运行期 rollout 事实。

## Remaining Gaps

- `test-storybook` 仍受本机缺失 Playwright Chromium 环境限制；Storybook 构建与同一 canvas 的浏览器人工验收已通过。

## Related Changes

- `web/src/features/shared/AdaptiveMetricValue.tsx`
- `web/src/features/dashboard/UsageBreakdownTooltip.tsx`
- `web/src/features/dashboard/UsageBreakdownTooltip.stories.tsx`

## References

- `./SPEC.md`
- `./HISTORY.md`
