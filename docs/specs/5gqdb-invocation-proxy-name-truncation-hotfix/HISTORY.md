# InvocationTable 桌面代理名省略回归热修 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/5gqdb-invocation-proxy-name-truncation-hotfix/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-09: 创建 hotfix spec，冻结根因、修复边界、回归样例与快车道发布口径。
- 2026-03-09: 已完成桌面代理列热修、长代理名回归断言与本地 `vitest/build/storybook/playwright` 验证，等待 PR/checks/review-loop 收敛。
- 2026-03-09: 快车道完成 PR [#105](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/105) 创建、标签收敛、checks 通过与本地 codex review 清零。
- 2026-03-10: 补充 `InvocationTable` DOM 区域截图到 spec 资产与 PR 视觉证据，便于直接复核长代理名省略效果。
