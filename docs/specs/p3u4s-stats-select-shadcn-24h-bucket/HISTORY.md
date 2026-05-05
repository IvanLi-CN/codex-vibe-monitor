# 统计页选择器切换为 shadcn 并补齐最近 7 天的 24 小时粒度 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/p3u4s-stats-select-shadcn-24h-bucket/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-19: 创建 spec，冻结“Stats 选择器 shadcn 化 + 最近 7 天补 24 小时粒度”范围。
- 2026-03-19: 已完成 `Select` 组件接入、Stats 页替换、文案补充，以及 `Stats.test.tsx` + `bunx tsc -b` 验证。
- 2026-03-19: 为满足 `react-refresh/only-export-components`，将 Stats 页桶位配置抽到 `web/src/pages/stats-options.ts`，行为与验收口径保持不变。
