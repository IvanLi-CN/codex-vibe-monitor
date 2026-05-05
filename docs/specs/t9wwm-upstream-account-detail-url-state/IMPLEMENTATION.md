# 上游账号详情改为 URL / ID 驱动并跨页面统一 - Implementation

## Current State

- Canonical spec: `docs/specs/t9wwm-upstream-account-detail-url-state/SPEC.md`
- Implementation summary: 已实现

## Migrated Implementation Notes

## 状态

- Status: 已实现
- Created: 2026-03-28
- Last: 2026-03-28

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cd web && bun run test -- src/pages/account-pool/UpstreamAccounts.test.tsx`
- `cd web && bun run test -- src/components/InvocationTable.test.tsx`
- `cd web && bun run test -- src/components/InvocationRecordsTable.test.tsx`
- `cd web && bun run test -- src/components/PromptCacheConversationTable.test.tsx`
- `cd web && bun run build`
- `cd web && bun run build-storybook`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增本 spec 索引并记录 fast-track follow-up
- `docs/specs/t9wwm-upstream-account-detail-url-state/SPEC.md`: 维护范围、验收、视觉证据与 PR 事实
