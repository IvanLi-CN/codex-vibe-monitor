# 不同计划 OAuth 账号共存时取消重复 warning - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/96qgn-oauth-mixed-plan-duplicate-warning/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-04-04: 创建 follow-up spec，冻结“不同有效计划类型的 OAuth 账号不再视为重复”的范围与验收标准。
- 2026-04-04: 后端 duplicate clustering 改为只在“同有效计划类型”或“任一侧计划未知”时保留 `sharedChatgptAccountId` / `sharedChatgptUserId` warning，mixed-plan 已知组合不再告警。
- 2026-04-04: 补齐 Rust mixed-plan / unknown-plan 回归，新增 roster/detail/create 前端测试与 `Mixed Plan Coexistence` Storybook 场景。
- 2026-04-04: 本地验证通过 `cargo fmt`、目标 Rust tests、`cd web && bun run test -- src/components/UpstreamAccountsTable.test.tsx src/pages/account-pool/UpstreamAccounts.test.tsx src/pages/account-pool/UpstreamAccountCreate.test.tsx` 与 `cd web && bun run build-storybook`；mock-only 视觉证据已落盘，截图提交仍待主人授权。
- 2026-04-04: mixed-plan Storybook fixture 已移除来自主人截图的显示名、邮箱、共享身份字段与路由 key 掩码，统一替换为 `fixture` / `example.invalid` 合成值，并刷新本地视觉证据。
