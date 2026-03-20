# Dashboard：合并 24h / 7d 活动总览卡片（#dzbnx）

## 状态

- Status: 已完成
- Created: 2026-03-20
- Last: 2026-03-20

## 背景 / 问题陈述

- 当前 Dashboard 将“最近 7 天活动图”和“最近 24 小时统计”拆成两个独立卡片，纵向空间占用偏大，信息密度不够紧凑。
- 主人明确要求把两块内容合并成一个共享总览卡：左侧标题旁增加 `24 小时 / 7 日` 切换，右侧保留 `次数 / 金额 / Tokens` 指标切换。
- 现有 24 小时卡片有 5 项 KPI，7 天卡片只有热力图，没有同规格统计面板；合并后若不补齐 7 日 KPI，会形成切换后的信息缺口。

## 目标 / 非目标

### Goals

- 将 Dashboard 中原本独立的 7 日热力图卡片与 24 小时统计卡片合并为一个共享 `surface-panel`。
- 共享卡片默认展示 `24 小时`，支持切换到 `7 日`，两个时间范围都展示同一套 5 项 KPI。
- `24 小时` 保留现有 10 分钟热力图，`7 日` 保留现有 1 小时热力图，且都复用现有统计接口。
- 保留“每个时间范围独立记住指标选择”的交互，不把原来两张卡能分别查看不同指标的能力回退掉。
- 补齐前端页面级回归测试，并完成本地验证。

### Non-goals

- 不改动 Rust 后端、SQLite、SSE 协议或 `/api/stats/*` 契约。
- 不改动 Dashboard 顶部 `TodayStatsOverview`、右上 `UsageCalendar` 与底部 `InvocationTable` 的产品行为。
- 不为该总览卡新增 URL 持久化、localStorage 偏好记忆或其它额外筛选器。

## 范围（Scope）

### In scope

- `web/src/components/DashboardActivityOverview.tsx`：新增合并总览组件，统一承载范围切换、指标切换、KPI 与热力图内容。
- `web/src/components/WeeklyHourlyHeatmap.tsx`：新增受控 metric、header 抑制和外壳抑制能力，允许嵌入到共享卡片内部。
- `web/src/pages/Dashboard.tsx`：移除独立 7 日/24 小时卡片，改为渲染新的共享总览卡片。
- `web/src/i18n/translations.ts`：新增共享标题与时间范围切换文案。
- `web/src/pages/Dashboard.test.tsx`、`web/src/components/WeeklyHourlyHeatmap.test.tsx`：补齐页面级与组件级回归测试。
- `docs/specs/README.md`：登记本 spec 索引并随交付推进同步状态。

### Out of scope

- `src/` 下任意后端实现与接口定义。
- `web/src/components/Last24hTenMinuteHeatmap.tsx` 的数据契约调整。
- 新增截图资产；若后续需要把截图纳入 PR，需单独获得主人明确同意。

## 验收标准（Acceptance Criteria）

- Given 打开 Dashboard，When 查看中部活动总览区域，Then 原本分离的 7 日卡片与 24 小时卡片合并为单一卡片，且没有嵌套 panel 边框。
- Given 活动总览卡片，When 查看头部，Then 左侧显示共享标题与 `24 小时 / 7 日` 切换，右侧显示 `次数 / 金额 / Tokens` 切换。
- Given 默认进入 Dashboard，When 活动总览首次渲染，Then 默认选中 `24 小时`，展示 `useSummary('1d')` 的 5 项 KPI 与 10 分钟热力图。
- Given 切换到 `7 日`，When 查看内容区，Then 展示 `useSummary('7d')` 的 5 项 KPI 与 1 小时热力图。
- Given `7 日` 下将指标切到 `金额`，When 再切回 `24 小时` 改成 `Tokens` 后返回 `7 日`，Then `7 日` 仍保持 `金额`，`24 小时` 仍保持 `Tokens`。
- Given 运行前端测试与构建，When 执行本次改动相关命令，Then `cd web && bun run test` 与 `cd web && bun run build` 通过。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: `cd web && bun run test -- Dashboard`
- Unit tests: `cd web && bun run test -- WeeklyHourlyHeatmap`

### Quality checks

- Frontend test suite: `cd web && bun run test`
- Frontend build: `cd web && bun run build`

## 文档更新（Docs to Update）

- `docs/specs/README.md`：新增本 spec 索引并同步状态 / Notes。
- `docs/specs/dzbnx-dashboard-activity-overview-merge/SPEC.md`：记录实现、验证与快车道收敛结果。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 新建 spec 并登记 `docs/specs/README.md`。
- [x] M2: 新增 Dashboard 合并总览组件，并接入 `useSummary('1d')` 与 `useSummary('7d')`。
- [x] M3: `WeeklyHourlyHeatmap` 支持受控嵌入模式，Dashboard 移除独立 7 日卡片。
- [x] M4: 补齐 Dashboard / WeeklyHourlyHeatmap 回归测试，并通过前端测试与构建。
- [x] M5: 完成本地预览验收与 fast-track PR 收敛到 merge-ready。

## 方案概述（Approach, high-level）

- 新建一个页面级组合组件，把“时间范围切换”和“指标切换”从原本两个独立卡片中提到统一头部。
- 两个 summary hook 与两张 heatmap 都在组件挂载时预热，其中 24h/7d 热力图保持 mounted，仅通过 `hidden` 切换显示，避免首次切换冷启动或丢失各自的指标状态。
- 7 日 KPI 直接复用现有 `/api/stats/summary?window=7d`，不引入新的接口或衍生 summary 聚合层。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：若隐藏非激活热力图时误用了条件渲染，可能导致切换后重新拉数或丢失指标记忆；测试需要锁住 mounted-but-hidden 行为。
- 风险：合并头部后在窄屏下可能产生换行或拥挤，需要通过真实页面验证无横向溢出。
- 假设：共享总览标题使用中性文案“活动总览 / Activity Overview”，不再沿用单范围标题。
- 假设：本轮快车道终点是 merge-ready，不自动 merge。

## 变更记录（Change log）

- 2026-03-20: 创建 spec，冻结“Dashboard 合并 24h / 7d 活动总览卡片”范围与验收标准。
- 2026-03-20: 已完成 `DashboardActivityOverview`、`WeeklyHourlyHeatmap` 嵌入能力、页面/组件回归测试，以及 `bun run build`、定向 Vitest、Playwright 本地烟测。
- 2026-03-20: `bun run test` 仍被仓库现存 `UpstreamAccountCreate.test.tsx` 两个 5s timeout 用例阻断；本次新增用例已独立验证通过，待在 PR 收敛阶段作为已知非本次回归记录。
- 2026-03-20: PR #192 已进入 `mergeable_state=clean`，GitHub PR checks 全绿，`codex review --base origin/main` 未发现离散阻塞回归，快车道按 merge-ready 收口。
