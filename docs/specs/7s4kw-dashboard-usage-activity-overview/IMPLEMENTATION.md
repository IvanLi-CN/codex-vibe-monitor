# Dashboard：把“历史”并入“活动总览”，并将“今日统计信息”改为单行 KPI - Implementation

## Current State

- Canonical spec: `docs/specs/7s4kw-dashboard-usage-activity-overview/SPEC.md`
- Implementation summary: 已实现，待 PR / CI / review-proof 收敛

## Migrated Implementation Notes

## 状态

- Status: 已实现，待 PR / CI / review-proof 收敛
- Created: 2026-04-07
- Last: 2026-04-07

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Frontend targeted tests:
  - `cd web && bun run test -- src/components/DashboardActivityOverview.test.tsx src/components/UsageCalendar.test.tsx src/components/TodayStatsOverview.test.tsx src/pages/Dashboard.test.tsx`
- Storybook build:
  - `cd web && bun run build-storybook`

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `docs/specs/7s4kw-dashboard-usage-activity-overview/SPEC.md`
