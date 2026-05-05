# Dashboard：工作中对话卡片替换 - Implementation

## Current State

- Canonical spec: `docs/specs/w3t3w-dashboard-working-conversations-cards/SPEC.md`
- Implementation summary: 已完成（6/6，PR #295）

## Migrated Implementation Notes

## 状态

- Status: 已完成（6/6，PR #295）
- Created: 2026-04-04
- Last: 2026-04-06

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust targeted tests: `cargo test prompt_cache_conversation`
- Frontend targeted tests: `cd web && bunx vitest run src/pages/Dashboard.test.tsx src/hooks/usePromptCacheConversations.test.tsx`
- Storybook interaction proof: `CreatedAtDescendingOrder` story `play` 必须断言 `pck-created-newest -> pck-created-middle -> pck-created-oldest`
- Storybook build: `cd web && bun run storybook:build`

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `docs/specs/w3t3w-dashboard-working-conversations-cards/SPEC.md`
