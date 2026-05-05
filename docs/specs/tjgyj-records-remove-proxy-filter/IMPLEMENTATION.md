# `/records` 移除代理筛选 - Implementation

## Current State

- Canonical spec: `docs/specs/tjgyj-records-remove-proxy-filter/SPEC.md`
- Implementation summary: 已完成（5/5）

## Migrated Implementation Notes

## 状态

- Status: 已完成（5/5）
- Created: 2026-04-06
- Last: 2026-04-06

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust tests: `cargo test build_invocation_filters_ -- --nocapture`
- Rust tests: `cargo test fetch_invocation_suggestions_ -- --nocapture`
- Frontend tests: `cd web && bunx vitest run src/pages/Records.test.tsx src/lib/invocationRecords.test.ts src/lib/api.test.ts src/components/InvocationRecordsTable.test.tsx`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增本次 follow-up spec 索引。
- `docs/specs/tjgyj-records-remove-proxy-filter/SPEC.md`: 记录筛选退场范围、契约、验证和视觉证据。
