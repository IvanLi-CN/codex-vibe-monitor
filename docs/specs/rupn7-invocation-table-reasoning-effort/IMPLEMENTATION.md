# InvocationTable 推理强度与详情 reasoningTokens - Implementation

## Current State

- Canonical spec: `docs/specs/rupn7-invocation-table-reasoning-effort/SPEC.md`
- Implementation summary: 已完成

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-03-07
- Last: 2026-03-07

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: Rust 请求体解析、`list_invocations` 投影与 malformed payload 容错回归。
- Integration tests: 启动期 reasoningEffort backfill 回填成功/失败路径。
- E2E tests (if applicable): InvocationTable 列表与详情展示推理强度/`reasoningTokens`。

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增本 spec 索引并更新状态。
