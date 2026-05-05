# 上游账号分组共享备注 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/thyxm-upstream-account-group-notes/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录

- 2026-03-14: 创建增量 spec，冻结上游账号分组共享备注的数据模型、交互边界与验收标准。
- 2026-03-14: 完成后端组备注持久化、前端复用弹窗与批量/单账号/API Key/详情编辑入口接入，并通过本地自动化验证与浏览器 smoke。
- 2026-03-14: 补充 Storybook 视觉证据，覆盖复用组备注弹窗、详情抽屉入口，以及批量 OAuth 行内组备注按钮位置。
- 2026-03-25: `j86ms` 为 pending OAuth login session 补齐 `PATCH` metadata sync，新增账号页中的分组备注草稿编辑不再要求重新生成 OAuth URL。
- 2026-04-01: 补充新分组草稿场景的 owner-facing 视觉证据，记录当前共享设置弹窗在草稿态下的无限回显与自动路由空态。
