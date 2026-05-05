# Dashboard：合并 24h / 7d 活动总览卡片 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/dzbnx-dashboard-activity-overview-merge/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-20: 创建 spec，冻结“Dashboard 合并 24h / 7d 活动总览卡片”范围与验收标准。
- 2026-03-20: 已完成 `DashboardActivityOverview`、`WeeklyHourlyHeatmap` 嵌入能力、页面/组件回归测试，以及 `bun run build`、定向 Vitest、Playwright 本地烟测。
- 2026-03-20: `bun run test` 仍被仓库现存 `UpstreamAccountCreate.test.tsx` 两个 5s timeout 用例阻断；本次新增用例已独立验证通过，待在 PR 收敛阶段作为已知非本次回归记录。
- 2026-03-20: PR #192 已进入 `mergeable_state=clean`，GitHub PR checks 全绿，`codex review --base origin/main` 未发现离散阻塞回归，快车道按 merge-ready 收口。
