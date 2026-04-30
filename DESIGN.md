---
name: Codex Vibe Monitor
description: Self-hosted OpenAI-compatible proxy observability workspace
colors:
  base-100-light: "oklch(98.7% 0.006 255)"
  base-200-light: "oklch(95.2% 0.01 255)"
  base-300-light: "oklch(90.1% 0.015 257)"
  base-content-light: "oklch(27.2% 0.02 260)"
  primary-light: "oklch(57.5% 0.195 258)"
  secondary-light: "oklch(61.5% 0.118 188)"
  accent-light: "oklch(67.7% 0.154 68)"
  success-light: "oklch(70.2% 0.167 154)"
  warning-light: "oklch(80.4% 0.167 82)"
  error-light: "oklch(66.2% 0.218 26)"
  base-100-dark: "oklch(24.5% 0.019 258)"
  base-200-dark: "oklch(18.4% 0.016 258)"
  base-300-dark: "oklch(32.1% 0.019 258)"
  base-content-dark: "oklch(93.6% 0.013 258)"
  primary-dark: "oklch(76.3% 0.133 232)"
  secondary-dark: "oklch(79.1% 0.113 189)"
  accent-dark: "oklch(81.4% 0.149 84)"
  success-dark: "oklch(78.4% 0.16 155)"
  warning-dark: "oklch(84.9% 0.163 83)"
  error-dark: "oklch(75.5% 0.176 24)"
typography:
  display:
    fontFamily: "IBM Plex Sans, Avenir Next, Segoe UI, PingFang SC, Hiragino Sans GB, Microsoft YaHei, sans-serif"
    fontSize: "1.25rem"
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: "0"
  headline:
    fontFamily: "IBM Plex Sans, Avenir Next, Segoe UI, PingFang SC, Hiragino Sans GB, Microsoft YaHei, sans-serif"
    fontSize: "1.125rem"
    fontWeight: 600
    lineHeight: 1.25
    letterSpacing: "0"
  title:
    fontFamily: "IBM Plex Sans, Avenir Next, Segoe UI, PingFang SC, Hiragino Sans GB, Microsoft YaHei, sans-serif"
    fontSize: "1rem"
    fontWeight: 600
    lineHeight: 1.35
    letterSpacing: "0"
  body:
    fontFamily: "IBM Plex Sans, Avenir Next, Segoe UI, PingFang SC, Hiragino Sans GB, Microsoft YaHei, sans-serif"
    fontSize: "0.875rem"
    fontWeight: 400
    lineHeight: 1.6
    letterSpacing: "0"
  label:
    fontFamily: "IBM Plex Sans, Avenir Next, Segoe UI, PingFang SC, Hiragino Sans GB, Microsoft YaHei, sans-serif"
    fontSize: "0.75rem"
    fontWeight: 600
    lineHeight: 1.35
    letterSpacing: "0.08em"
rounded:
  field: "0.75rem"
  box: "1rem"
  selector: "1.9rem"
  full: "9999px"
spacing:
  control-x: "0.75rem"
  panel: "1.25rem"
  page-x: "1rem"
  section: "1.5rem"
components:
  button-primary:
    backgroundColor: "{colors.primary-light}"
    textColor: "oklch(97.4% 0.011 258)"
    rounded: "{rounded.field}"
    padding: "0.5rem 1rem"
    height: "2.25rem"
  button-secondary:
    backgroundColor: "{colors.base-200-light}"
    textColor: "{colors.base-content-light}"
    rounded: "{rounded.field}"
    padding: "0.5rem 1rem"
    height: "2.25rem"
  panel:
    backgroundColor: "{colors.base-100-light}"
    textColor: "{colors.base-content-light}"
    rounded: "{rounded.box}"
    padding: "{spacing.panel}"
---

# Design System: Codex Vibe Monitor

## 1. Overview

**Creative North Star: "观测实验室"**

Codex Vibe Monitor 的界面应像一张可信的观测实验台：稳定、清楚、密集，但不冷漠。用户打开它时通常已经有真实代理流量、上游账号、错误、成本或 routing 问题要处理，所以视觉系统必须把信号排序、把风险暴露、把操作入口留在手边。

这是 product UI，不是品牌展示页。熟悉 Linear、Stripe、Raycast、Grafana 或自建运维台的用户应该能迅速相信它，而不是被装饰性动效、模糊层或奇怪控件打断。界面可以保留轻微实验室气质，例如细网格、半透明 surface、实时 pulse，但这些元素必须被限制在服务状态表达的范围内。

**Key Characteristics:**

- 高密度：表格、图表、配置和详情面板可以承载大量信息。
- 克制：primary 只用于当前选择、主操作、重点状态，不作为装饰色铺满页面。
- 稳定：数字使用 tabular nums，表格列与图表轴保持可扫描节奏。
- 双主题：light 和 dark 都是一等公民，不把 dark 当作唯一“观测感”来源。
- 证据优先：每个颜色、徽标、图表和告警都应解释系统状态。

## 2. Colors

色彩策略是 **Restrained product palette**：一组偏冷中性色承载工作区，蓝色作为主交互信号，青绿色、琥珀、绿色和红色承担明确状态或指标语义。

### Primary

- **Signal Blue** (`oklch(57.5% 0.195 258)` light, `oklch(76.3% 0.133 232)` dark): 主操作、当前 nav、focus ring、选中态和 request count 类图表。不要把它用于无语义装饰。

### Secondary

- **Proxy Teal** (`oklch(61.5% 0.118 188)` light, `oklch(79.1% 0.113 189)` dark): 代理节点、token 体量、健康的次级信号。它应与 primary 区分，不要抢主操作权重。

### Tertiary

- **Cost Amber** (`oklch(67.7% 0.154 68)` light, `oklch(81.4% 0.149 84)` dark): 成本、价格、配额压力和需要留意但未失败的运营信号。

### Neutral

- **Lab Paper** (`oklch(98.7% 0.006 255)`): light 主题内容 surface。
- **Mist Rail** (`oklch(95.2% 0.01 255)`): light 主题页面背景和次级 surface。
- **Rule Line** (`oklch(90.1% 0.015 257)`): 边框、分割线、表头底色。
- **Graphite Text** (`oklch(27.2% 0.02 260)`): light 主题主文字。
- **Night Bench** (`oklch(18.4% 0.016 258)`): dark 主题页面背景。
- **Instrument Panel** (`oklch(24.5% 0.019 258)`): dark 主题内容 surface。
- **Cool Chalk** (`oklch(93.6% 0.013 258)`): dark 主题主文字。

### Named Rules

**The Signal Rarity Rule.** Primary 的稀缺性就是它的意义。一个视图内 primary 应优先出现在当前选择、主按钮、focus ring 和关键图表信号上。

**The Color Carries Meaning Rule.** count 蓝、cost 琥珀、token 青绿、success 绿色、warning 琥珀、error 红色的语义不得跨页面互换。

## 3. Typography

**Display Font:** IBM Plex Sans, Avenir Next, Segoe UI, PingFang SC, Hiragino Sans GB, Microsoft YaHei, sans-serif

**Body Font:** IBM Plex Sans, Avenir Next, Segoe UI, PingFang SC, Hiragino Sans GB, Microsoft YaHei, sans-serif

**Label/Mono Font:** 默认 UI 使用同一 sans 栈；代码、版本号、金额和 token 数值使用 `font-mono` utility。

**Character:** 字体系统追求工程产品的可信熟悉感，不使用 display font。中文说明保持自然，英文技术词保留原词；数字默认启用 tabular nums，保证指标和表格列稳定。

### Hierarchy

- **Display** (600, `1.25rem`, `1.2`): 应用品牌、页面主标题或抽屉高层标题。产品内不使用营销级大标题。
- **Headline** (600, `1.125rem`, `1.25`): section heading、重要 panel 标题、dialog 标题。
- **Title** (600, `1rem`, `1.35`): 卡片标题、表单分组、列表项目主标签。
- **Body** (400, `0.875rem`, `1.6`): 说明、表格正文、配置描述。长段落限制在 65 到 75ch。
- **Label** (600, `0.75rem`, `0.08em`): 表头、字段标签、metric label。大写标签必须短，避免中文被过度字距拉开。

### Named Rules

**The Dense Reading Rule.** 密集信息依靠对齐、分组、等宽数字和稳定字号解决，不靠继续缩小文字解决。

## 4. Elevation

系统使用 tonal layering + restrained shadow。普通内容面板通过边框、半透明底色和轻阴影与背景分离；dialog、drawer、popover 才使用更强阴影。模糊层不是默认 elevation，而是少量浮层或 Storybook 预览 surface 的附加效果。

### Shadow Vocabulary

- **Panel Shadow** (`0 18px 40px rgba(15, 23, 42, 0.09)` light): 应用主 surface，适合 dashboard panel 和内容容器。
- **Panel Shadow Dark** (`0 20px 54px rgba(2, 6, 23, 0.55)` dark): dark 主题主 surface，需要避免叠加过多 glow。
- **Dialog Shadow** (`0 32px 90px rgba(15, 23, 42, 0.16)` light, `0 36px 110px rgba(2, 6, 23, 0.62)` dark): 只用于阻塞式或强上下文浮层。
- **Drawer Shadow** (`-32px 0 84px rgba(15, 23, 42, 0.18)`): 右侧详情面板，表达从当前列表滑出的高层信息。

### Named Rules

**The Flat-By-Default Rule.** 静态内容默认靠边框和 tonal background 分层；shadow 只在 panel、drawer、dialog、popover 或 hover/focus 状态中出现。

## 5. Components

组件语言应稳定、可预测、可复用。新增页面先复用 `web/src/components/ui/` primitives，再补业务组件。

### Buttons

- **Shape:** 基础按钮使用 `rounded-md` 或 `--radius-field` (`0.75rem`)；icon-only 和 control-pill 可以使用 `rounded-full`。
- **Primary:** `bg-primary text-primary-content`，默认高度 `h-9`，用于真正主动作。
- **Hover / Focus:** hover 使用轻微明度变化；focus 使用 `focus-visible:ring-2 focus-visible:ring-primary`，不要只依赖边框变色。
- **Secondary / Ghost / Destructive:** secondary 使用 base surface；ghost 只在低风险 inline action 中使用；destructive 使用 error 语义且必须有清楚文案。

### Chips

- **Style:** chip 和 badge 使用圆角胶囊、细边框、低透明语义底色。
- **State:** selected/active 必须同时改变边框、背景和文字权重；不要只靠颜色微差。

### Cards / Containers

- **Corner Style:** 常规 card 使用 `rounded-xl` 或 `--radius-box` (`1rem`)。
- **Background:** 普通内容用 `bg-base-100` 或 `surface-panel`；密集表格容器可以用 `bg-base-100/52` 到 `bg-base-100/85`。
- **Shadow Strategy:** 静态 card 轻阴影或无阴影；浮层、drawer、dialog 才使用强阴影。
- **Border:** 默认 `border-base-300`，状态面板可用语义色低透明边框。
- **Internal Padding:** 密集区从 `p-3` 到 `p-4`；普通 panel 用 `p-5`；大型设置区可用 `p-6`。

### Inputs / Fields

- **Style:** 输入框使用 `h-10`、`rounded-lg`、`border-base-300`、`bg-base-100`。
- **Focus:** `ring-2 ring-primary` 是标准 focus 表达。
- **Error / Disabled:** error 应通过 field feedback 或 Alert 解释；disabled 需要视觉降权且不可交互。

### Navigation

- **Style:** 顶部导航使用 segmented control family。当前路由使用 primary 边框、primary tint 背景和 primary 文本。
- **Responsive:** 小屏允许横向滚动导航，但不能隐藏当前页面上下文。后续若页面继续增长，应评估折叠 nav 或二级导航。

### Data Visualization

- **Metric colors:** count 使用蓝，cost 使用琥珀/橙，token 使用青绿。
- **Charts:** 图表 loading/empty/error 必须保留容器高度，避免实时刷新造成 layout jump。
- **Tables:** 数值列右对齐，金额、token、延迟和时间使用 tabular nums 或 monospace 节奏。

## 6. Do's and Don'ts

### Do:

- **Do** 复用 `web/src/index.css` 的 OKLCH 语义 token 和 `web/src/lib/chartTheme.ts` 的图表语义。
- **Do** 让 primary 保持稀缺，优先用于主动作、当前选择、focus ring 和关键运行信号。
- **Do** 在新增图表前先选择 count、cost、token、success、failure 或 error 语义。
- **Do** 为 icon-only action 提供 `aria-label`，并保留 `focus-visible`。
- **Do** 保持数字、金额、token、latency 的稳定对齐，优先使用 tabular nums。
- **Do** 用 Storybook story 固化新组件、新状态和关键页面截图入口。

### Don't:

- **Don't** 使用 gradient text。
- **Don't** 使用 `border-left` 或 `border-right` 大于 1px 作为彩色侧边强调。
- **Don't** 把 glassmorphism 当默认容器语言；模糊和半透明只能用于少量浮层或状态反馈。
- **Don't** 复制 hero metric 模板、同款卡片网格或通用 SaaS dashboard 构图。
- **Don't** 把观测工具做成深蓝黑底霓虹风格；dark theme 也必须保持克制和可读。
- **Don't** 为单个页面硬编码新的 hex/RGBA 颜色，除非它进入明确的图表或组件 token。
- **Don't** 通过继续缩小文字和触控目标来解决移动端密度问题。
