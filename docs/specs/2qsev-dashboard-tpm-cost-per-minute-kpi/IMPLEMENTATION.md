# Dashboard 今日 KPI 上下文统计卡片 - Implementation

## Current State

- Canonical spec: `docs/specs/2qsev-dashboard-tpm-cost-per-minute-kpi/SPEC.md`
- Implementation summary: 已实现，待 PR / CI / review-proof 收敛

## Migrated Implementation Notes

## 状态

- Status: 已实现，待 PR / CI / review-proof 收敛
- Created: 2026-04-10
- Last: 2026-04-30

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: `dashboardTodayRateSnapshot` 速率计算覆盖活跃尾段、前置 0 不稀释、当前部分分钟参与、活动后静默计入分母、零活动窗口；`dashboardKpiComparisons` 覆盖工作分钟日均、百分比差异、缓存命中和并行对话快照。
- Integration tests: `TodayStatsOverview.test.tsx`、`DashboardActivityOverview.test.tsx`、`Dashboard.test.tsx` 覆盖 6 tile 与 partial fallback。
- E2E tests (if applicable): None。

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增索引项并在实现完成后同步状态
- `docs/specs/2qsev-dashboard-tpm-cost-per-minute-kpi/SPEC.md`: 同步进度与视觉证据
