# 全站简单下拉统一为 `SelectField` - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/pqqpf-selectfield-simple-dropdown-rollout/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-22: 创建 spec，冻结全站 simple dropdown 统一收口到 `SelectField` 的范围、接口与验收口径。
- 2026-03-22: 完成 `SelectField` 封装、页面迁移、Storybook 独立展示、源码契约测试与 `.field-select*` 清理。
- 2026-03-22: 本地验证通过 `cd web && bun run test`、`cd web && bun run build`、`cd web && bun run build-storybook`、`cd web && bun run test:e2e -- proxy-model-settings.spec.ts`。
- 2026-03-22: PR #201 已创建并进入快车道收敛，`codex review --base origin/main` 未发现离散阻塞回归。
