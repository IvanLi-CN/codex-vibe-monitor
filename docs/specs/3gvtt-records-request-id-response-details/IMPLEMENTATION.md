# `/records` 请求 ID 筛选与异常响应详情抽屉 - Implementation

## Current State

- Canonical spec: `docs/specs/3gvtt-records-request-id-response-details/SPEC.md`
- Implementation summary: 已完成

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-04-04
- Last: 2026-04-04

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust tests: `cargo test build_invocation_filters_normalizes_request_id -- --exact`
- Rust tests: `cargo test response_body_ -- --nocapture`
- Frontend tests: `cd web && bun run test src/pages/Records.test.tsx src/components/InvocationRecordsTable.test.tsx src/lib/api.test.ts src/lib/invocationRecords.test.ts`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增 follow-up spec 索引，标记该 records 增量需求已有规格落点。
- `docs/specs/3gvtt-records-request-id-response-details/SPEC.md`: 固定范围、接口、验收和视觉证据。
