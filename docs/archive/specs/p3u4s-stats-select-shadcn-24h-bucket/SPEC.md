# 统计页选择器切换为 shadcn 并补齐最近 7 天的 24 小时粒度（#p3u4s）

## 状态

- Status: 已实现
- Created: 2026-03-19
- Last: 2026-03-19

## 背景 / 问题陈述

- 统计页右上角的时间范围与聚合粒度控件仍使用原生 `select`，与仓库现有 shadcn 风格基础组件不一致。
- 主人明确指出“最近 7 天”缺少 `24 小时` 尺度，当前只能选择 `1h / 6h / 12h`，不利于查看更平滑的 7 日走势。
- 若继续保持现状，Stats 页会同时存在视觉基线不统一和关键分析粒度缺失两个问题。

## 目标 / 非目标

### Goals

- 将 Stats 页顶部两个选择器与错误范围选择器统一切换为 shadcn 风格 `Select` 组件。
- 为 `最近 7 天` 增加 `每 24 小时` 聚合粒度，同时保持底层 `bucket=1d` 语义不变。
- 补充页面级回归测试，锁住“非原生 select + 7d 含 24h 桶位”两个行为。

### Non-goals

- 不重构 Stats 页整体布局、图表组件或后端统计接口。
- 不一次性迁移 `Live`、`Records`、`Settings` 等其它页面的原生 `select`。
- 不新增新的时间范围、后端分桶规则或统计页信息架构。

## 范围（Scope）

### In scope

- `web/package.json` 与 `web/bun.lock`：补充 `@radix-ui/react-select` 依赖。
- `web/src/components/ui/select.tsx`：新增项目内 shadcn 风格 `Select` 封装。
- `web/src/pages/Stats.tsx`：替换 3 处原生选择器，并为 `7d` 增加 `1d` bucket 选项。
- `web/src/pages/stats-options.ts`：承载 Stats 页范围与桶位配置，避免页面文件导出非组件触发 lint 失败。
- `web/src/i18n/translations.ts`：新增“每 24 小时 / Every 24 hours”文案。
- `web/src/pages/Stats.test.tsx`：新增页面级回归测试。
- `docs/specs/README.md` 与当前 spec：记录本次 fast-flow 交付状态。

### Out of scope

- Rust 后端、SQLite、SSE 或 `/api/stats/*` 接口实现。
- `web/src/pages/Live.tsx`、`web/src/pages/Records.tsx` 等其它页面的选择器迁移。
- Storybook 或真实程序截图资产。

## 验收标准（Acceptance Criteria）

- Given 打开 Stats 页，When 查看顶部时间范围与粒度控件，Then 控件使用 `button[role="combobox"]` 语义而不是原生 `select`。
- Given `range=最近 7 天`，When 查看粒度选项，Then 可见并可选择 `每 24 小时`（`bucket=1d`）。
- Given 切换错误范围控件，When 页面渲染，Then 错误范围也使用同一套 shadcn `Select` 组件。
- Given 运行前端页面测试与 TypeScript 构建，When 执行本次改动相关命令，Then 全部通过。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: `cd web && bunx vitest run src/pages/Stats.test.tsx`

### Quality checks

- TypeScript build: `cd web && bunx tsc -b`

## 文档更新（Docs to Update）

- `docs/specs/README.md`：新增本 spec 索引并同步状态。
- `docs/specs/p3u4s-stats-select-shadcn-24h-bucket/SPEC.md`：记录本轮实现与验证结论。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 新建 spec 并冻结“仅 Stats 选择器 + 7d/24h”范围。
- [x] M2: 引入 shadcn 风格 `Select` 组件与 `@radix-ui/react-select` 依赖。
- [x] M3: Stats 页 3 处原生选择器完成替换，且 `7d` 增加 `1d` 桶位。
- [x] M4: 补齐页面级回归测试并通过 TypeScript 构建。
- [ ] M5: 完成 fast-flow 提交、PR、checks、review-loop 与收尾。

## 方案概述（Approach, high-level）

- 复用仓库当前 Radix/shadcn 基础设施，在 `web/src/components/ui/` 下补一份项目内 `Select` 实现，避免 Stats 页继续手写原生选择器样式。
- 对 `7d` 直接增加 `value='1d'` 选项，并单独映射文案为“每 24 小时”，避免用户看到“每天”时误解为自然日视角变更。
- 用独立 `Stats.test.tsx` 锁住 DOM 语义与桶位配置，降低后续回归概率。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：仓库内仍有其它页面使用原生 `select`，本轮只修复 Stats 页，不保证全站视觉统一。
- 风险：若后续决定对 `1d` 统一展示为“每天”，需要同步调整 Stats 与其它页面的文案策略。
- 需要决策的问题：None。
- 假设（需主人确认）：None。

## 变更记录（Change log）

- 2026-03-19: 创建 spec，冻结“Stats 选择器 shadcn 化 + 最近 7 天补 24 小时粒度”范围。
- 2026-03-19: 已完成 `Select` 组件接入、Stats 页替换、文案补充，以及 `Stats.test.tsx` + `bunx tsc -b` 验证。
- 2026-03-19: 为满足 `react-refresh/only-export-components`，将 Stats 页桶位配置抽到 `web/src/pages/stats-options.ts`，行为与验收口径保持不变。

## 参考（References）

- `docs/specs/jpg66-settings-shadcn-refresh/SPEC.md`
- `docs/specs/8dun3-stats-success-failure-ttfb/SPEC.md`
- `web/src/pages/Stats.tsx`
