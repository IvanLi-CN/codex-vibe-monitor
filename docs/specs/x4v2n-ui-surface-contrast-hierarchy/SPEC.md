# UI surface 对比层级收口（#x4v2n）

## 背景 / 问题陈述

- 多个页面直接组合 `border-base-300/*`、`bg-base-100/*`、高透明白底和局部 shadow，light/dark 主题下容易出现边框过亮、内部区块抢正文层级、dialog chrome 与内容区亮度混杂的问题。
- Dashboard 批量清理确认弹窗、Settings 高密度配置卡、Account Pool 账号详情与路由设置弹窗都暴露了同一类问题：页面各自调透明度，而不是通过共享 surface vocabulary 维持层级。
- 根级 `DESIGN.md` 与 `docs/ui/*` 仍把 `border-base-300` / `bg-base-100` 描述为直接使用入口，后续新增页面容易继续复制旧写法。

## 目标 / 非目标

### Goals

- 建立共享 neutral surface vocabulary：普通 card、内部低对比区块、嵌入配置块、输入控件、菜单浮层和 dialog chrome 都有明确 class 入口。
- 让 `Card`、`Input`、`SelectTrigger`、`SelectContent` 等基础 primitive 默认消化 surface 颜色与边框，减少页面级重复组合。
- 修复 Dashboard、Settings、Account Pool 中高可见度的 surface 对比失衡，确保 light/dark 主题都保持克制层级。
- 同步 `DESIGN.md` 与 `docs/ui/*`，把共享 surface 入口写成后续新增规则。

### Non-goals

- 不做全站视觉重设计，不改产品信息架构、布局密度或品牌方向。
- 不一次性清除所有历史 literal color、图表颜色或 feature-specific 状态色。
- 不新增第三套主题、不调整主题切换机制。
- 不把 Storybook 本身的 React Refresh 重复符号问题纳入本 spec 修复范围。

## 范围（Scope）

### In scope

- `web/src/index.css` 里的主题 surface token 与共享 surface utility class。
- `web/src/components/ui/card.tsx`、`input.tsx`、`select.tsx` 的默认 surface。
- Dashboard 批量路由/清理确认 dialog chrome 与 destructive callout。
- Settings 页面配置卡、forward proxy 区块、选择控件和表格容器。
- Account Pool 上游账号能力卡、路由设置弹窗、详情抽屉中的高可见度 card/dialog surface。
- `DESIGN.md`、`docs/ui/README.md`、`docs/ui/foundations.md`、`docs/ui/components.md`、`docs/ui/patterns.md`。

### Out of scope

- 图表语义色、计划 badge、热力图色带与已有业务状态色。
- Storybook runtime 修复。
- 截图资产入库；本次视觉证据通过 Codex thread 回传。

## 需求（Requirements）

### MUST

- `web/src/index.css` 必须提供 `surface-card`、`surface-subtle`、`surface-inset`、`field-surface`、`menu-surface`、`dialog-chrome-surface`。
- `Card` 默认必须使用 `surface-card`，`Input` 与 `SelectTrigger` 默认必须使用 `field-surface`，`SelectContent` 默认必须使用 `menu-surface`。
- Dashboard 的批量路由绑定与清理确认 dialog header/footer 必须使用 `dialog-chrome-surface`，清理确认 callout 必须使用低饱和 destructive surface，而不是亮白内部卡。
- Settings 与 Account Pool 的高可见度卡片、配置组、dialog chrome 必须优先复用共享 surface class。
- `DESIGN.md` 与 `docs/ui/*` 必须记录共享 surface vocabulary，并禁止普通页面继续复制高透明 `bg-base-100` 与亮 `border-base-300` 组合作为默认容器语言。
- light/dark 主题必须各有视觉证据覆盖，且验证入口必须可复现。

### SHOULD

- 页面私有 surface 只用于业务状态、图表语义或 feature spec 已约束的特殊展示。
- 静态内容区保持 tonal layering，不新增默认化 blur/glass；blur 只保留在 dialog chrome 或浮层上下文。
- 后续新页面若命中同类层级需求，应先使用共享 class，再考虑新增 token。

## 验收标准（Acceptance Criteria）

- Given Dashboard 批量清理确认 dialog 打开，When 处于 dark theme，Then destructive callout 的边框和底色不应比正文和按钮更抢眼，dialog header/footer 与正文内容区有清晰但克制的层级。
- Given Settings 页面在 dark theme 下展示代理、模型和 forward proxy 配置，When 扫描页面，Then 卡片内部块不会出现亮白边框堆叠，输入和下拉控件保持统一 field surface。
- Given Account Pool 上游账号页在 light theme 下展示能力卡和账号详情，When 查看 card 与 dialog，Then 默认容器不再使用页面私有白色渐变或强边框。
- Given 新增基础 Card/Input/Select 使用默认 class，When 不传入页面私有背景类，Then 组件仍能在 light/dark 下获得一致 surface、border、focus 层级。
- Given 阅读 `DESIGN.md` 与 `docs/ui/foundations.md`，When 查找 surface 使用规则，Then 能看到共享 surface vocabulary 和直接复制 `bg-base-100` / `border-base-300` 的禁止边界。

## Visual Evidence

- source_type: web_demo_route
  route: `/#/dashboard`
  target_program: mock-only
  capture_scope: page/dialog
  requested_viewport: desktop
  theme: dark
  submission_gate: approved in Codex thread
  state: Dashboard bulk clear/reset destructive dialog and dashboard surface hierarchy
  evidence_note: Captured through Web Demo because Storybook preview is currently blocked by an unrelated React Refresh duplicate-symbol error in `src/theme/context.tsx`.
  image: returned in Codex thread, not committed as a repository asset.
- source_type: web_demo_route
  route: `/#/settings`
  target_program: mock-only
  capture_scope: page
  requested_viewport: desktop
  theme: dark
  submission_gate: approved in Codex thread
  state: Settings high-density configuration surface hierarchy
  evidence_note: Verifies shared `surface-inset` / `surface-subtle` / `field-surface` usage across dense configuration cards.
  image: returned in Codex thread, not committed as a repository asset.
- source_type: web_demo_route
  route: `/#/account-pool/upstream-accounts`
  target_program: mock-only
  capture_scope: page
  requested_viewport: desktop
  theme: light
  submission_gate: approved in Codex thread
  state: Account Pool card and dialog-adjacent surface hierarchy
  evidence_note: Verifies Account Pool no longer depends on private white gradient cards for high-visibility surfaces.
  image: returned in Codex thread, not committed as a repository asset.

## 方案概述（Approach, high-level）

- 在 `web/src/index.css` 中用 OKLCH theme token 计算共享 surface 背景、边框、shadow 和 destructive callout token。
- 将基础 primitive 默认外观迁移到共享 surface class，让业务页面继承统一 layer contract。
- 先治理 Dashboard、Settings、Account Pool 的高可见度区域，保留图表、badge、业务状态色等不在本次范围内的既有实现。
- 将新 surface vocabulary 同步到根级设计文档和内部 UI 文档，作为后续页面实现入口。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：仍有历史页面和 badge 体系使用 literal color 或直接 utility class，本 spec 不承诺一次性清完。
- 风险：Storybook preview 当前存在无关 React Refresh 重复符号问题，视觉证据使用 Web Demo 替代；Storybook 修复需要独立处理。
- 假设：`color-mix(...)` 与 OKLCH token 继续是当前前端支持的基础能力。
- 开放问题：无。
