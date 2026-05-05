# 上游账号详情调用记录与 Sticky 对话对齐 Live 交互 - Implementation

## Current State

- Canonical spec: `docs/specs/cg6um-upstream-account-detail-records-sticky-conversations/SPEC.md`
- Implementation summary: 已实现，待 PR 收敛

## Migrated Implementation Notes

## 状态

- Status: 已实现，待 PR 收敛
- Created: 2026-03-30
- Last: 2026-04-27

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust: `cargo test upstream_account_sticky -- --nocapture`
- Rust: `cargo test invocation_records -- --nocapture`
- Web: `cd web && bun run test -- src/components/StickyKeyConversationTable.test.tsx src/hooks/useUpstreamStickyConversations.test.tsx src/hooks/useUpstreamStickyConversations.test.ts src/pages/account-pool/UpstreamAccounts.test.tsx src/lib/api.test.ts`
- Storybook: `cd web && bun run build-storybook`
- Visual evidence: Storybook canvas 截图覆盖账号详情 `调用记录 + 活动总览` 的 populated 与 empty 状态，并写入本 spec。

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `docs/specs/cg6um-upstream-account-detail-records-sticky-conversations/SPEC.md`
