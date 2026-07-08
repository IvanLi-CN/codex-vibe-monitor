# Impeccable Audit

本报告按 `$impeccable audit` 的技术质量维度记录问题，只审计、不修复。当前结论基于静态代码扫描、现有 `docs/ui` 文档、README 页面说明和前端命令验证结果。

## Audit Health Score

| #         |         Dimension |     Score | Key Finding                                                            |
| --------- | ----------------: | --------: | ---------------------------------------------------------------------- |
| 1         |     Accessibility |         3 | ARIA 与 focus 基础较好，但部分 dense controls 低于 44px touch target。 |
| 2         |       Performance |         2 | 全局 transition、固定多层背景、blur/glass surface 和重阴影需要治理。   |
| 3         |           Theming |         2 | OKLCH token 已存在，但 chart 与组件仍有大量 hex/RGBA 硬编码。          |
| 4         | Responsive Design |         2 | 核心页面可用，但移动端依赖横向滚动和压缩控件。                         |
| 5         |     Anti-Patterns |         2 | 产品框架可信，但 glass/blur/orb/gradient 用法偏默认化。                |
| **Total** |                   | **11/20** | **Acceptable, significant work needed**                                |

## Anti-Patterns Verdict

当前界面不像纯 AI 生成稿，信息架构、路由、表格、Storybook 覆盖和状态语义都是真实产品的形态。但它有明显“AI dashboard polish”风险：多处半透明 surface、模糊、orb 背景、渐变背景和发光 pulse 同时出现，容易把“观测实验室”推向泛化深蓝科技感。

最需要警惕的是 `glassmorphism as default`。`web/src/index.css` 中的应用背景、surface panel、dialog、Storybook preview，以及多个业务 overlay 都在叠加 `backdrop-filter`、透明底色和大阴影。它们单独看合理，组合后会削弱产品工具应有的干净层级。

## Executive Summary

- Audit Health Score: **11/20**，Acceptable。
- Total issues found: **P1: 2, P2: 3, P3: 2**。
- Top issues: token drift、glass/blur 默认化、全局 transition、移动端密度债。
- Recommended next step: 先确认本报告，再按 `$impeccable colorize`、`$impeccable optimize`、`$impeccable adapt`、`$impeccable polish` 分批治理。

## Detailed Findings by Severity

### [P1] Token drift across charts and surfaces

- **Location:** `web/src/lib/chartTheme.ts`, `web/src/features/stats/ParallelWorkStatsSection.tsx`, `web/src/features/dashboard/DashboardTodayActivityChart.tsx`, `web/src/features/stats/SuccessFailureChart.tsx`
- **Category:** Theming
- **Impact:** 图表和局部组件绕开 OKLCH 语义 token 使用 hex/RGBA，light/dark 主题一致性、对比校验和未来换肤成本都会上升。
- **WCAG/Standard:** WCAG 1.4.3 Contrast, 需要逐色验证。
- **Recommendation:** 建立 chart token 到全局语义 token 的映射层，至少把 count/cost/token/success/failure/axis/tooltip 背景集中到可审计 token。
- **Suggested command:** `$impeccable colorize`

### [P1] Glass and blur are over-applied

- **Location:** `web/src/index.css`, `web/src/features/app-shell/AppLayout.tsx`, `web/src/components/ui/floating-surface.ts`, account-pool overlays
- **Category:** Performance / Anti-Pattern
- **Impact:** 半透明、blur、orb、radial gradient 和强 shadow 叠加后会增加绘制成本，也让产品层级显得装饰化。密集表格和长期驻留监控场景更需要低噪声。
- **WCAG/Standard:** WCAG 1.4.11 Non-text Contrast, 需要验证边框与 surface 分离度。
- **Recommendation:** 明确 blur 只用于 dialog/popover/drawer 等浮层；普通 panel 退回 tonal background + border + light shadow。
- **Suggested command:** `$impeccable quieter`

### [P2] Global transition rule affects every element

- **Location:** `web/src/index.css`
- **Category:** Performance / Motion
- **Impact:** `*` 上的 transition 会让所有元素的颜色、边框和阴影变化都进入动画路径，可能在大表格、实时刷新、主题切换和状态批量更新时造成额外样式工作。
- **WCAG/Standard:** WCAG 2.3.3 Animation from Interactions, 需要 reduced motion 策略时复查。
- **Recommendation:** 将全局 transition 收敛到 button、link、interactive surface、popover、dialog 等明确组件类。
- **Suggested command:** `$impeccable optimize`

### [P2] Touch targets below 44px

- **Location:** `web/src/components/ui/button.tsx`, `web/src/pages/Records.tsx`, `web/src/pages/account-pool/UpstreamAccountCreate.primary-card.tsx`
- **Category:** Accessibility / Responsive
- **Impact:** `h-8`、`h-9` 和 32px icon buttons 对桌面鼠标可接受，但对移动端触控和精细操作不友好。
- **WCAG/Standard:** WCAG 2.5.8 Target Size (Minimum), WCAG 2.1 AA guidance。
- **Recommendation:** 保留桌面 dense 尺寸，但在 touch/mobile breakpoint 提供 44px interaction box，或增加可点击 padding。
- **Suggested command:** `$impeccable adapt`

### [P2] Mobile density relies on horizontal scroll

- **Location:** `web/src/features/invocations/InvocationTable.tsx`, `web/src/pages/Settings.tsx`, `web/src/features/dashboard/WeeklyHourlyHeatmap.tsx`, `web/src/features/dashboard/Last24hTenMinuteHeatmap.tsx`
- **Category:** Responsive Design
- **Impact:** 横向滚动对表格和热力图有时合理，但目前多个核心配置和分析页面都依赖固定 `min-w` 或 desktop table structure，小屏排障效率会下降。
- **WCAG/Standard:** WCAG 1.4.10 Reflow, 需要逐页面验证。
- **Recommendation:** 为 Settings、Records、Invocation table 和 Account Pool 的最高频流程补移动端摘要卡或分段列视图，保留横滚给低频完整数据。
- **Suggested command:** `$impeccable adapt`

### [P3] Verification setup initially lacked dependencies

- **Location:** `web/node_modules`
- **Category:** Performance / Quality Gate
- **Impact:** 初次执行 `bun run lint` 和 `bun run build` 时，当前 worktree 缺少 `web/node_modules`，命令在进入实际检查前失败。安装依赖后，lint、build 和 Storybook 测试已通过。
- **WCAG/Standard:** 不适用。
- **Recommendation:** 后续新 worktree 执行 UI 审计前，先确认 `cd web && bun install` 已完成，避免把环境缺口误判为代码问题。
- **Suggested command:** `$impeccable audit`

### [P3] Existing UI docs are useful but split from root design context

- **Location:** `docs/ui/README.md`, `docs/ui/foundations.md`, `docs/ui/components.md`, `docs/ui/data-viz.md`
- **Category:** Theming / Documentation
- **Impact:** `docs/ui` 已经记录了很多实现真相，但缺少 PRODUCT/DESIGN 根级入口，AI agent 难以先读战略再读细节。
- **WCAG/Standard:** 不适用。
- **Recommendation:** 在后续文档维护中让 `PRODUCT.md` / `DESIGN.md` 作为入口，`docs/ui` 继续承载更细的实现真相。
- **Suggested command:** `$impeccable document`

## Patterns & Systemic Issues

- **Token system is partial.** CSS OKLCH token、Tailwind utility、chart hex token、组件内 RGBA 同时存在，需要把“允许硬编码”的边界写清楚。
- **Surface language is too decorative by default.** 透明面板、blur、orb 和 shadow 已经成为常规视觉语言，而不是例外。
- **Desktop density is stronger than mobile ergonomics.** 产品对大屏很友好，但移动端经常靠横向滚动、截断和较小 target 维持布局。
- **Storybook foundation is strong.** 大量页面和组件已有 story，这是后续做视觉证据、a11y 和 screenshot review 的优势。

## Completed Setup

- `PRODUCT.md`、`DESIGN.md` 和 `DESIGN.json` 已在本次改动中补齐，后续 impeccable 任务可以先读取根级产品与视觉上下文。
- `$impeccable` loader 已确认 `PRODUCT.md` 和 `DESIGN.md` 可识别。

## Positive Findings

- `web/src/index.css` 已经有 light/dark OKLCH 主题 token，基础方向正确。
- `docs/ui/` 已经沉淀 foundations、components、patterns、data-viz 和 Storybook 规范，说明项目不是无文档状态。
- `web/src/components/ui/` 已经有 button、input、card、badge、alert、dialog、popover、tooltip、segmented-control 等 primitive family。
- 多数 icon-only action 和复合控件已经有 `aria-label`、`aria-expanded`、`role` 或 Radix 语义支持。
- 图表语义在 `chartTheme.ts` 中已集中为 count/cost/token/success/failure，虽然还需要接入全局 token。

## Recommended Actions

1. **[P1] `$impeccable colorize`**：收敛 chart 和组件硬编码颜色，把 count/cost/token/status 接回可审计 token。
2. **[P1] `$impeccable quieter`**：降低 glass/blur/orb 默认化，让普通 panel 回到 tonal layering。
3. **[P2] `$impeccable optimize`**：移除 `*` 全局 transition，检查 blur、shadow 和背景固定层的绘制成本。
4. **[P2] `$impeccable adapt`**：为小屏补 44px touch target 和高频表格的移动结构。
5. **[P3] `$impeccable audit`**：后续修复后重跑 lint/build/Storybook/a11y，更新分数。
6. **[P3] `$impeccable polish`**：在修复批次结束后做最终一致性检查。

You can ask me to run these one at a time, all at once, or in any order you prefer.

Re-run `$impeccable audit` after fixes to see your score improve.
