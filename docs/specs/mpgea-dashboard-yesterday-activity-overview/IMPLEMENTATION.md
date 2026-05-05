# Dashboard 活动总览增加“昨日”页签 - Implementation

## Current State

- Canonical spec: `docs/specs/mpgea-dashboard-yesterday-activity-overview/SPEC.md`
- Implementation summary: 已完成

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-04-11
- Last: 2026-04-11

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust：named range / summary-window helper 回归 + 至少一个 yesterday summary/timeseries API 级过滤回归。
- Frontend：`DashboardActivityOverview` / `Dashboard` / `useStats` / `useTimeseries` / `DashboardTodayActivityChart` 定向 Vitest。
- Storybook：新增 / 更新 yesterday 场景并通过 `build-storybook`。

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `docs/specs/mpgea-dashboard-yesterday-activity-overview/SPEC.md`
