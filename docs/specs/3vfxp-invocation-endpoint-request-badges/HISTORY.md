# InvocationTable 请求类型 Badge 化 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/3vfxp-invocation-endpoint-request-badges/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-22: 创建 spec，冻结 recognized endpoint 集合、摘要 badge / 详情 raw endpoint 边界与验证口径。
- 2026-03-22: 完成 helper、InvocationTable 摘要区、人类可读 badge、i18n、Storybook、Vitest 与独立租约端口上的 Playwright 布局回归；等待 PR 收敛完成 M3。
- 2026-03-22: PR #203 已收敛到 merge-ready，远端 checks 全绿，review-loop 未发现需修复阻塞项，完成 M3 并将 spec 置为已完成。
- 2026-03-22: 根据 Storybook 复查结果，增强 endpoint badge 在亮暗主题下的色相分离与填充/边框对比，避免 `Responses`、`Chat`、`远程压缩` 在暗色表格里视觉趋同。
