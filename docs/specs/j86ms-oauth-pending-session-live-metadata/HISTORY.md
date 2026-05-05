# 修复新增账号页 OAuth 地址被字段编辑重置 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/j86ms-oauth-pending-session-live-metadata/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录

- 2026-03-25: 创建增量 spec，冻结 pending OAuth login session metadata live-sync 的接口、交互与验收边界。
- 2026-03-25: 完成后端 pending-session metadata PATCH、前端单账号/批量 OAuth 热更新、Storybook/Vitest/Rust 定向回归，并补充本地视觉证据。
