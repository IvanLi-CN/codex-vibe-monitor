# Dashboard：修复 UsageCalendar 加载骨架右偏 + 首行骨架按真实两卡布局（#rzxey）

## 状态

- Status: 已完成
- Created: 2026-03-05
- Last: 2026-03-05

## 背景 / 问题陈述

- 刷新 Dashboard（`/dashboard`）时，右侧 UsageCalendar 在加载期间会出现明显的水平错位：骨架阶段卡片整体向右偏移，待数据返回后再跳回，产生可感知的 layout shift。
- 现状原因：UsageCalendar 使用 `react-activity-calendar` 的 `loading` 模式，内部会渲染“整年空数据”且隐藏 weekday/month labels，导致组件在加载期与加载完成后的固有宽度/布局结构不一致，从而影响 Dashboard 顶部 `grid-cols-[minmax(0,1fr)_max-content]` 的列分配。

## 目标 / 非目标

### Goals

- 消除刷新时 UsageCalendar 在加载期间的可感知水平错位（layout shift）。
- 加载骨架按真实界面结构渲染：
  - Dashboard 首行在加载期仍保持「今日统计信息」+「使用活动」两个卡片并排（桌面宽度下）。
  - UsageCalendar 在加载期展示与真实一致的 weekday labels 与 month label overlay，并保持 90d 网格尺度（约 13~14 周列）。
- 当 timeseries points 为空或全为 0 时，展示“空日历”（最低色）而不是无限 skeleton。

### Non-goals

- 不改变 Dashboard 顶部最终布局比例与 grid track 定义（除非验证仍存在 shift，才考虑追加兜底策略）。
- 不改动后端统计接口与数据口径。

## 范围（Scope）

### In scope

- `web/src/components/UsageCalendar.tsx`：
  - 移除对 `ActivityCalendar loading` 的依赖，改为“永远传入非空 90d activities”。
  - 加载期通过 `skeletonMode`（如 `animate-pulse` + 禁用 tooltip 事件）表达“正在加载”，而不是改变布局结构。
  - 当 points 为空/全 0，仍渲染空日历并保持结构稳定。
- `web/tests/e2e/usage-calendar.spec.ts`：
  - 增加回归用例：对 `range=90d&bucket=1d` 的请求注入延迟，断言加载前后卡片位置稳定、首行两卡可见。
- `docs/specs/README.md`：登记本 spec 的索引条目，并随实现推进同步状态。

### Out of scope

- `src/` Rust 后端实现与 API 定义调整。
- 其它 Dashboard 区块（24h/7d 热图、InvocationTable）逻辑与布局改造。

## 验收标准（Acceptance Criteria）

- Given 桌面宽度（>= 1024px）刷新 Dashboard，When UsageCalendar 数据仍在加载，Then 首行保持两卡并排，UsageCalendar 不出现“明显右偏再跳回”的错位感。
- Given UsageCalendar 加载期，When 渲染骨架，Then weekday labels 与 month label overlay 的占位结构与加载完成后保持一致。
- Given timeseries points 为空或全为 0，When 渲染 UsageCalendar，Then 显示空日历（最低色）而非无限 skeleton，页面无报错。
- Given Playwright E2E 注入 timeseries 延迟，When 对比加载前后 `usage-calendar-card` 的 `boundingBox.x`，Then 变化不超过 `2px`，且加载期两卡标题可见。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 新建 Spec 并登记到 `docs/specs/README.md`。
- [x] M2: UsageCalendar：用 90d 零值占位替换 `react-activity-calendar` 内置 loading 骨架，保证布局稳定。
- [x] M3: UsageCalendar：空数据/全 0 时展示空日历（最低色），不再无限 skeleton。
- [x] M4: Playwright：新增延迟回归用例并通过。
- [x] M5: 本地验证通过（lint/test/build）并完成视觉验收。

## 进度备注

- 已移除 `react-activity-calendar` 内置 `loading` 骨架，改为始终渲染 90d activities（缺失数据自动补 0）以保持布局结构一致。
- 加载期对 blocks 增加 `animate-pulse`，并禁用 tooltip 事件，避免骨架阶段产生误导性交互。
- 新增 E2E：注入 `range=90d&bucket=1d` 延迟，断言加载前后首行两卡并排且 `usage-calendar-card` 的 x 位置稳定（≤2px）。
- 已完成 `web` 的 `npm run lint`、`npm run test`、`npm run build`；Playwright 回归关注点已覆盖在 UsageCalendar spec 内。
