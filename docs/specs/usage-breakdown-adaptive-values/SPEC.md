# Dashboard 用量明细自适应数值显示

> 本文是该主题的长期需求契约；实现覆盖与交付事实见 `./IMPLEMENTATION.md`，主题演进与兼容背景见 `./HISTORY.md`。

## Context and Scope

- Context: Dashboard 的「用量明细」浮层使用固定表格列和完整分组数字，Token、成本等长数值在窄列中挤压或溢出，降低扫描效率；项目的统计卡片已经具备按实际可用宽度选择紧凑量级和精度的共享能力。
- In scope: `UsageBreakdownTooltip` 的桌面表格与移动列表中的 Token、成本、比例数值；共享自适应数值组件的可选完整值 Tooltip；`TodayStatsOverview` 与 `DashboardWorkingConversationsSection` 提供的本地化和精度上下文；相关单元测试与 Storybook 回归。
- Out of scope: 后端用量聚合、成本计算、历史未知值语义、模型排序和列业务含义；整张浮层的列布局重做；统计卡片之外所有数字的全局迁移；把浏览器原生 `title` 作为可读性方案。

## Terms and Interfaces

- `用量明细自适应值`: 用量明细中的单个数值展示单元，根据自身内容宽度选择可读候选；它不同于统计卡片中的 `紧凑主值`，但复用相同的候选生成与测量原则。
- `完整值`: 调用方按当前 locale、货币和既有精度规则生成的未压缩文本；它是 Tooltip 中的权威 owner-facing 表达，不是重新计算出的近似值。
- `紧凑候选`: 共享自适应格式器生成的、可能降低小数位或使用 K/M/B/T 量级的候选文本；它不是通过 CSS 或字符串截断得到的残缺文本。
- `可用内容宽度`: 单元格扣除内边距、图标、标签和间距后，当前数值文本可以占用的单行宽度；它必须通过实际布局测量得到，不能用固定视口宽度代替。
- Interface: `web/src/features/dashboard/UsageBreakdownTooltip.tsx`、`web/src/features/shared/AdaptiveMetricValue.tsx`、`web/src/features/shared/adaptiveMetricValueSpec.ts` 和 `web/src/components/ui/tooltip.tsx`。

## Requirements

### REQ-UBAV-001 — 每个明细单元独立适配

- 每个 Token、成本和比例数值单元 MUST 独立根据自己的可用内容宽度选择候选；不得用整列一个固定字符串或同一行最长值决定所有单元的显示。
- 桌面表格和移动列表 MUST 都使用这一单元级契约；当前响应式结构、模型身份和列语义不得因适配而改变。
- 选择后的可见文本 MUST 在其单元格内保持单行可读并不产生横向溢出；布局变化后必须能够重新测量并重新选择候选。

### REQ-UBAV-002 — 共享格式阶梯与可读精度

- 自适应候选 MUST 复用 `adaptiveMetricValueSpec.ts` 的本地化和量级规则，而不是在 Dashboard 浮层内另造一套字符串缩写逻辑。
- 候选选择 MUST 遵循“完整本地化值 -> 在语义允许范围内降低小数位 -> 使用共享量级单位”的阶梯；计数和 Token 沿用 K/M/B/T，成本沿用 `$K`/`$M`/`$B`/`$T`，比例沿用项目百分比精度规则。
- 量级边界发生舍入进位时 MUST 升级到正确的下一量级；不得显示 `1000K` 这类不自然表达，也不得用固定小数位强行占满列宽。
- 候选只能改变呈现精度和单位，不得改变原始聚合值、成本语义或比较关系。

### REQ-UBAV-003 — 完整值与自定义 Tooltip

- 当可见候选不是完整值时，组件 MUST 通过项目现有 `Tooltip` 直接渲染完整本地化值；对于数字本身不能表达的单位（例如 Token），必须追加调用方提供的本地化单位文案，不能增加无信息量的通用 Label。Tooltip 内容 MUST 不重新计算业务数据。
- Tooltip MUST 支持现有项目的 hover、focus、click 和移动端长按交互语义；完整值不紧凑时不得为每个单元强制增加重复提示。
- 该能力 MUST 不依赖浏览器原生 `title`，不得产生浏览器默认样式或与项目浮层主题不一致的原生提示。

### REQ-UBAV-004 — 调用方提供类型化格式上下文

- `UsageBreakdownTooltip` MUST 接收能够生成自适应规格的类型化调用方上下文或 builder，而不是先接收不可逆的格式化字符串再尝试解析数值。
- `TodayStatsOverview` 和 `DashboardWorkingConversationsSection` MUST 继续提供各自的 locale、货币和既有精度语义；共享组件不得硬编码某一个页面的 locale 或美元展示规则。
- `AdaptiveDisplayValue` 的 Tooltip 能力 MUST 为可选扩展；未启用该扩展的既有统计卡片保持当前候选、布局和可访问行为。

### REQ-UBAV-005 — 缺失、零值和历史语义不变

- `null`、历史上游成本未知和其他既有 unknown 状态 MUST 继续显示 `—` 或当前项目约定的 unknown 文本，不得被压缩为 `0`、空字符串或伪造金额。
- 已知零成本 MUST 继续显示本地化的零金额；总计行仍必须遵循现有聚合和成本可用性语义。
- 自适应显示只改变文本表达，不得改变模型身份、generation 展示、缓存命中率、总计计算、筛选和排序。

### REQ-UBAV-006 — 本地化和响应式一致性

- 完整值和紧凑候选 MUST 使用调用方传入的 locale；分组分隔符、小数点、货币符号和百分比符号必须保持项目既有本地化约定。
- 在桌面约束宽度和移动 `390px` 级别视口下，浮层内容 MUST 在容器内完成布局，不得依赖横向滚动来容纳数值。
- 选中的候选在布局宽度改变、字体加载完成或浮层切换布局后 MUST 可重新测量；不得把首次渲染时的宽度判断永久缓存为全局结论。

### REQ-UBAV-007 — 可验证的实现边界

- 实现 MUST 为共享候选选择、完整值 Tooltip、用量明细既有语义和浮层响应式布局分别提供可读的回归覆盖。
- Storybook MUST 保留一个窄桌面约束浮层和一个 `mobile390` 状态，用可重复的长 Token/成本样本验证候选切换、完整值交互和零横向溢出。
- 回归覆盖 MUST 能证明普通完整值、降低小数位、量级切换、边界进位、unknown 成本、已知零成本以及总计行都不发生语义回退。

## Verification

### VER-UBAV-001

- Method: `AdaptiveMetricValue` / `adaptiveMetricValueSpec` 单元测试，以可控可用宽度覆盖完整值、精度降低、量级切换和边界进位。
- covers: `REQ-UBAV-001`, `REQ-UBAV-002`, `REQ-UBAV-004`, `REQ-UBAV-006`
- Pass condition: 每个测量槽只显示能放入自身宽度的最高可读候选；候选格式、locale 和量级边界与共享组件既有规则一致，组件未被要求解析格式化字符串。

### VER-UBAV-002

- Method: `UsageBreakdownTooltip` 组件测试，覆盖桌面表格、移动列表、总计、模型身份、unknown 成本、已知零成本和历史成本缺失。
- covers: `REQ-UBAV-001`, `REQ-UBAV-005`, `REQ-UBAV-006`, `REQ-UBAV-007`
- Pass condition: 内容表达和总计语义与现有测试契约一致，长值可以切换候选，unknown/zero 不被误归一化，桌面和移动 DOM 都不改变业务字段含义。

### VER-UBAV-003

- Method: Storybook `ConstrainedOverlay` 与 `Mobile390` interaction/DOM checks，配合项目 `Tooltip` 交互测试。
- covers: `REQ-UBAV-003`, `REQ-UBAV-006`, `REQ-UBAV-007`
- Pass condition: 紧凑候选出现时可通过 hover、focus、click 或移动端长按读取带标签的完整值；不存在浏览器原生 `title`，且浮层/表格/移动列表的 `scrollWidth` 不超过容器 `clientWidth`。

## Related ADRs

None

## Visual Evidence

- Source: Storybook canvas, `Dashboard/UsageBreakdownTooltip/ConstrainedOverlay` and `Dashboard/UsageBreakdownTooltip/Mobile390`.
- Assets: [`constrained-overlay.png`](./assets/constrained-overlay.png), [`mobile390.png`](./assets/mobile390.png).
- Acceptance: owner-confirmed screenshots show single-line column headings, independently adaptive values, unit-bearing full-value Tooltip content without a native `title`, and no horizontal overflow at the constrained desktop width or `390px` viewport.
- Preflight: both source-managed surfaces passed the required margin, containment, opaque-background, and horizontal-overflow checks; the candidate comparison against baseline `83436dd4cb4f72ad33f2284a52187a773c30497a` was `current-only` because these asset paths did not exist in the baseline.

## References

- `./IMPLEMENTATION.md`
- `./HISTORY.md`
- [`CONTEXT.md`](../../../CONTEXT.md)
- [`web/src/features/dashboard/UsageBreakdownTooltip.tsx`](../../../web/src/features/dashboard/UsageBreakdownTooltip.tsx)
- [`web/src/features/shared/AdaptiveMetricValue.tsx`](../../../web/src/features/shared/AdaptiveMetricValue.tsx)
- [`web/src/features/shared/adaptiveMetricValueSpec.ts`](../../../web/src/features/shared/adaptiveMetricValueSpec.ts)
