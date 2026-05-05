# OAuth 上游 `x-codex-installation-id` 代理侧稳定改写 - Implementation

## Current State

- Canonical spec: `docs/specs/jm3hb-oauth-installation-id-rewrite/SPEC.md`
- Implementation summary: 已完成

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-04-11
- Last: 2026-04-11

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: OAuth body rewrite / seed helper / installation id 派生稳定性
- Integration tests: mock upstream 验证 rewrite body 与 passthrough body 的转发观测值
- E2E tests (if applicable): None

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增索引并在收尾时同步状态
