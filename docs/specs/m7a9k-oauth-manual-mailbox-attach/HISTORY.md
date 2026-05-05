# OAuth 手动邮箱附着与增强能力判定 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/m7a9k-oauth-manual-mailbox-attach/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录

- 2026-03-17: 创建增量 spec，显式覆盖 `3n287` 中“仅支持系统生成邮箱增强”的旧边界，并冻结单账号 OAuth / reauth 的手动邮箱附着语义。
- 2026-03-17: 完成后端附着逻辑、邮箱来源清理策略、前端联合类型与手动邮箱交互，并通过本地 Rust / Web 自动化验证。
