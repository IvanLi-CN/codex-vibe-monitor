# Dashboard：把“历史”并入“活动总览”，并将“今日统计信息”改为单行 KPI - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/7s4kw-dashboard-usage-activity-overview/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-04-07: 创建 follow-up spec，冻结“三段切换合并历史 + 今日 KPI 单行 + 不做双栏 + merge-ready 收口”的范围与验收标准。
- 2026-04-07: 已完成 Dashboard 页面合并、`UsageCalendar` 嵌入/受控模式、`TodayStatsOverview` 单行 KPI，以及相关 Vitest 回归新增。
- 2026-04-07: 历史视图文案从“使用活动”改为“历史”，并将多月日历范围从 `90d` 升级为 `6mo`，同时同步 Storybook、Vitest 与 E2E 夹具。
- 2026-04-07: 根据主人反馈，移除总览内嵌历史视图中的重复标题与时区说明，仅保留日历本体。
- 2026-04-07: 根据主人反馈继续收紧历史视图月份标签与热图之间的垂直间距，消除重叠并避免留白过大；最新 Storybook 视觉证据已归档。
