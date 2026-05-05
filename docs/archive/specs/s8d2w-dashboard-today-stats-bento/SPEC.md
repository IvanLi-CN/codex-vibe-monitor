# Dashboard：将“配额概览”替换为“今日统计信息（#s8d2w）

## 状态

- Status: 已完成
- Created: 2026-02-26
- Last: 2026-02-26

## 背景 / 问题陈述

- 当前 Dashboard 左上角展示“配额概览”，该区域信息对当前使用场景价值下降。
- 主人明确要求将该区域改为“今日统计信息”，并保持其它区域（使用活动、24h 统计/热图、最近实况）的结构与功能不发生本质变化。

## 目标 / 非目标

### Goals

- 左上角替换为“今日统计信息”卡片（Bento 风格）。
- 数据口径改为 `today`（自然日），展示 5 项 KPI：调用总数、成功、失败、总成本、总 Tokens。
- 保持 Dashboard 其余区域的交互与信息结构稳定不变。
- 新增中英文文案键，确保双语一致。

### Non-goals

- 不改动后端 API、统计口径实现或数据库结构。
- 不改动右侧“使用活动”组件逻辑。
- 不改动下方“最近 24 小时统计/热图”和“最近实况”区域行为。

## 范围（Scope）

### In scope

- `web/src/pages/Dashboard.tsx`：顶部左侧组件替换与数据源改为 `useSummary('today')`。
- `web/src/components/TodayStatsOverview.tsx`：新增独立组件。
- `web/src/i18n/translations.ts`：新增 `dashboard.today.*` 双语文案。
- `docs/specs/README.md`：新增规格索引条目并更新状态。

### Out of scope

- `src/` Rust 后端实现与接口定义。
- `web/src/components/UsageCalendar.tsx`、`web/src/components/WeeklyHourlyHeatmap.tsx`、
  `web/src/components/Last24hTenMinuteHeatmap.tsx`、`web/src/components/InvocationTable.tsx` 的行为改造。

## 验收标准（Acceptance Criteria）

- Given 进入 Dashboard，When 查看顶部左侧区域，Then 显示“今日统计信息”而非“配额概览”。
- Given 今日统计区域，When 数据加载完成，Then 正确展示 5 项 KPI（调用总数、成功、失败、总成本、总 Tokens）。
- Given 数据加载失败，When 渲染今日统计区域，Then 显示错误提示且不影响其他区域渲染。
- Given 切换中英文，When 查看今日统计区域，Then 标题、副标题与角标文案正确切换。
- Given 浏览不同设备宽度（375/768/1024/1440），When 查看顶部区，Then Bento 布局无横向溢出，右侧“使用活动”功能保持可用。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 新建 Spec 并登记到 `docs/specs/README.md`。
- [x] M2: 新增 `TodayStatsOverview` 组件并完成 Bento 视觉布局。
- [x] M3: Dashboard 顶部左侧替换完成，接入 `useSummary('today')`。
- [x] M4: 完成 i18n 双语文案与构建验证。
- [x] M5: 完成视觉验收并保留浏览器会话供确认。

## 进度备注

- 已完成左上卡替换与今日统计接入，右侧“使用活动”和下方 24h/实况区域保持原有功能。
- 已完成 `npm run lint` 与 `npm run build` 验证；当前项目未定义 `npm run test`，`npx vitest run` 会包含 e2e spec 导致失败，需后续补齐测试脚本分流。
- Playwright 已完成桌面与移动视口核验，并保留会话供人工确认。
