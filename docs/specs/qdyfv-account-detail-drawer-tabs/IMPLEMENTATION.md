# 账号详情抽屉统一关闭语义与 Tabs 分组 - Implementation

## Current State

- Canonical spec: `docs/specs/qdyfv-account-detail-drawer-tabs/SPEC.md`
- Implementation summary: 已完成（4/4）

## Migrated Implementation Notes

## 状态

- Status: 已完成（4/4）
- Created: 2026-03-25
- Last: 2026-03-27

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: 账号详情抽屉关闭语义、tabs 复位语义、overlay subtree 挂载回归
- Integration tests: `UpstreamAccounts.test.tsx`、相关 invocation drawer tests
- E2E tests (if applicable): 无新增专属 E2E，沿用现有 smoke 范围

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增本 spec 索引
- `docs/specs/qdyfv-account-detail-drawer-tabs/SPEC.md`: 维护范围、验收与视觉证据
