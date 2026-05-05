# InvocationTable 代理节点展示热修（#f7nqn) - Implementation

## Current State

- Canonical spec: `docs/specs/f7nqn-invocation-proxy-display-restore-hotfix/SPEC.md`
- Implementation summary: 已完成

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-03-17

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo check`
- `cargo test proxy_capture_persist_and_broadcast_emits_records_summary_and_quota`
- `cargo test list_invocations_projects_payload_context_fields`
- `cargo test capture_target_pool_route_retries_first_chunk_failure_and_persists_single_invocation`
- `cd web && bunx vitest run src/components/InvocationTable.test.tsx`
- `cd web && bun run build-storybook`

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `docs/specs/f7nqn-invocation-proxy-display-restore-hotfix/SPEC.md`
