# 全站简单下拉统一为 `SelectField`（#pqqpf）

## 状态

- Status: 待实现
- Created: 2026-03-22
- Last: 2026-03-22

## 背景 / 问题陈述

- 仓库内简单下拉当前分成三套口径：原生 `<select>`、页面直拼 low-level `Select` primitives，以及已封装的 searchable combobox。
- 用户明确要求“项目中所有下拉必须是 shadcn/ui 组件封装的”，且 Storybook 中必须能单独展示封装后的组件，不能继续出现浏览器原生 select 外观。
- 若继续让业务页面各自决定用原生 select 还是直接拼 Radix primitives，视觉、交互、测试和 Storybook 证据口径都会继续分叉。

## 目标 / 非目标

### Goals

- 新增项目级 `SelectField`，作为 simple dropdown 的唯一公开入口。
- 将 `Stats`、`Live`、`Records`、`Settings`、`account-pool/Tags`、`account-pool/UpstreamAccounts` 当前 simple dropdown 全量迁移到 `SelectField`。
- 新增 `SelectField` 独立 Storybook 展示，并同步更新相关 page stories、Vitest、Playwright 与源码契约测试。
- 保持 searchable combobox 系列不受影响，只收敛 simple dropdown。

### Non-goals

- 不改造 `FilterableCombobox`、`AccountTagFilterCombobox`、`UpstreamAccountGroupCombobox` 等 searchable combobox 组件。
- 不修改 Rust 后端、API 契约或页面整体信息架构。
- 不在本轮自动完成 PR merge 与本地 cleanup；快车道终点固定为 merge-ready。

## 范围（Scope）

### In scope

- `web/src/components/ui/select-field.tsx`：新增项目级 simple dropdown 封装，内部基于现有 `ui/select.tsx`。
- `web/src/components/ui/select-field.stories.tsx`：新增独立 Storybook showcase。
- `web/src/pages/Stats.tsx`、`Live.tsx`、`Records.tsx`、`Settings.tsx`、`account-pool/Tags.tsx`、`account-pool/UpstreamAccounts.tsx`：迁移 simple dropdown。
- 相关 stories、Vitest、Playwright、源码契约测试，以及 `docs/ui/storybook.md`、`docs/ui/components.md`、`docs/specs/README.md`。

### Out of scope

- combobox 搜索体验、命令面板交互模式、后端筛选参数。
- 任何非 simple dropdown 的基础组件重构。
- PR 合并、release 或 post-merge cleanup。

## 接口契约（Interfaces & Contracts）

- `SelectField` props：
  - `options: Array<{ value: string; label: string; disabled?: boolean }>`
  - `value: string`
  - `onValueChange: (value: string) => void`
  - `label?: string`
  - `name?: string`
  - `placeholder?: string`
  - `size?: 'default' | 'sm'`
  - `disabled?: boolean`
  - `className?: string`
  - `triggerClassName?: string`
  - `id?: string`
  - `data-testid?: string`
  - `aria-label?: string`
- 行为约束：
  - 业务页与业务 stories 不得继续直接 import `components/ui/select` low-level primitives。
  - `SelectField` 必须支持空字符串值选项，避免 “全部 / 任意” 这类筛选项因 Radix value 限制丢失。
  - 若传入 `name`，组件需提供稳定表单值承载，避免失去既有 query/测试锚点。

## 验收标准（Acceptance Criteria）

- Given 扫描 `web/src` 业务代码与 stories，When 检查 simple dropdown，Then 不再出现原生 `<select>`，也不再有页面/业务 stories 直接 import `components/ui/select`。
- Given 打开 `Stats`、`Live`、`Records`、`Settings`、`Tags`、`UpstreamAccounts`，When 查看 simple dropdown，Then 全部使用 `button[role="combobox"]` + listbox 语义，不再触发浏览器原生 select 外观。
- Given 打开 Storybook，When 查看 `SelectField` story，Then 至少可复核默认、placeholder、`sm`、disabled 四种状态。
- Given 运行前端验证命令，When 执行 Vitest、build、storybook build 与相关 Playwright，Then 本轮改动全部通过。
- Given 进入快车道 PR 收敛，When latest PR checks 与 review 收口完成，Then 状态达到 merge-ready 而不是 merged。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cd web && bun run test`
- `cd web && bun run test:e2e -- proxy-model-settings.spec.ts`

### Quality checks

- `cd web && bun run build`
- `cd web && bun run build-storybook`

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `docs/specs/pqqpf-selectfield-simple-dropdown-rollout/SPEC.md`
- `docs/ui/storybook.md`
- `docs/ui/components.md`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 新建 spec，冻结 `SelectField` API、迁移范围与 fast-track merge-ready 收口标准。
- [ ] M2: 新增 `SelectField` 封装并接管 `Stats` 当前 direct primitive 组装。
- [ ] M3: 完成 `Live`、`Records`、`Settings`、`Tags`、`UpstreamAccounts` simple dropdown 迁移，并清零生产代码与 stories 中的 `.field-select*` 使用。
- [ ] M4: 新增 `SelectField` Storybook showcase，更新相关 tests/stories，并加上源码契约测试。
- [ ] M5: 完成 fast-track 提交、push、PR、checks 与 review-loop 收敛到 merge-ready。

## 方案概述（Approach, high-level）

- 保留 `web/src/components/ui/select.tsx` 作为 low-level Radix/shadcn primitive，实现层只对 `SelectField` 和 low-level 自身测试开放。
- `SelectField` 负责收口 label、size、placeholder、hidden input、test id 与 className 扩展，避免页面继续手写 `SelectTrigger/Content/Item` 栈。
- 页面迁移时保持现有 option 文案、状态流与数值转换逻辑不变，只替换渲染层与测试交互方式。
- 用源码契约测试把“禁止原生 `<select>` / 禁止页面直引 low-level select”固化到 CI 口径。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：`SelectField` 需要兼容空字符串值选项，否则 `All / Any` 这类筛选项会在 Radix 中失效。
- 风险：既有 Vitest 和 Playwright 断言大量依赖 `HTMLSelectElement` / `selectOption()`，迁移时若漏改会直接导致验证失败。
- 风险：部分页面当前高度和边距依赖 `.field-select*`，迁移后需逐页补 `triggerClassName`，避免视觉回退。
- 需要决策的问题：None。
- 假设（需主人确认）：None。

## 变更记录（Change log）

- 2026-03-22: 创建 spec，冻结全站 simple dropdown 统一收口到 `SelectField` 的范围、接口与验收口径。

## 参考（References）

- `docs/specs/p3u4s-stats-select-shadcn-24h-bucket/SPEC.md`
- `docs/specs/jpg66-settings-shadcn-refresh/SPEC.md`
- `docs/ui/components.md`
