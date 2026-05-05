# InvocationTable 账号归因、时延压缩展示与当前页账号抽屉 - Implementation

## Current State

- Canonical spec: `docs/specs/7n2ex-invocation-account-latency-drawer/SPEC.md`
- Implementation summary: 已完成

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-03-16
- Last: 2026-03-17

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: Rust invocation payload / query projection tests；`web/src/components/InvocationTable.test.tsx`
- Integration tests: `/api/invocations` 返回新增字段并被前端类型消费
- E2E tests (if applicable): `web/tests/e2e/invocation-table-layout.spec.ts`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 登记本 spec 与进度
