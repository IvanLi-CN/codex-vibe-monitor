# Live Prompt Cache 对话表改成“上游账号 / 总计”双列复合展示 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/7y5yf-live-prompt-cache-upstream-summary-columns/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-21: 新建 spec，冻结 Prompt Cache Key 对话表“上游账号 / 总计”双列复合展示方案。
- 2026-03-21: 完成后端 `upstreamAccounts[]` 聚合、前端双列布局、i18n、测试与 Storybook 示例同步，等待快车道 PR 收口。
- 2026-03-21: PR #196 已创建并收敛到 merge-ready，所需 checks 全部通过。
