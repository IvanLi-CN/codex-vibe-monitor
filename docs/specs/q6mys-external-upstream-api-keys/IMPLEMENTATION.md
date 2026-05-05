# 第三方上游账号开放 API 与 APIKey 管理 - Implementation

## Current State

- Canonical spec: `docs/specs/q6mys-external-upstream-api-keys/SPEC.md`
- Implementation summary: 待实现

## Migrated Implementation Notes

## 状态

- Status: 待实现
- Created: 2026-04-17
- Last: 2026-04-17

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: external API key create / rotate / disable / auth；external OAuth upsert / patch / relogin helper。
- Integration tests: route-level 401/403/404/409 映射、同 client 幂等、跨 client 隔离、relogin 同步恢复。
- E2E tests (if applicable): none。

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 增加 spec 索引行
- `docs/specs/q6mys-external-upstream-api-keys/contracts/http-apis.md`: 记录开放接口与内部管理接口
- `docs/specs/q6mys-external-upstream-api-keys/contracts/db.md`: 记录 schema 变更与 rollout
