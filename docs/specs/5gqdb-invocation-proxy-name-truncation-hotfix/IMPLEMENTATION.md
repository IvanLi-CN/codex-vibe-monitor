# InvocationTable 桌面代理名省略回归热修 - Implementation

## Current State

- Canonical spec: `docs/specs/5gqdb-invocation-proxy-name-truncation-hotfix/SPEC.md`
- Implementation summary: 已完成

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-03-09
- Last: 2026-03-10

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: `cd web && npm run test -- --run src/components/InvocationTable.test.tsx`
- E2E tests: `cd web && npm run test:e2e -- tests/e2e/invocation-table-layout.spec.ts`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增 hotfix spec 索引并同步状态/备注。
- `docs/specs/5gqdb-invocation-proxy-name-truncation-hotfix/SPEC.md`: 记录实现进度、验证结果与变更说明。
