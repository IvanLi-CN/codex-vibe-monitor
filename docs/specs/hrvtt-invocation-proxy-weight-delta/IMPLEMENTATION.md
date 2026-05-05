# 请求详情补齐代理信息与本次权重变化 - Implementation

## Current State

- Canonical spec: `docs/specs/hrvtt-invocation-proxy-weight-delta/SPEC.md`
- Implementation summary: 已完成

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-03-02
- Last: 2026-03-02

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: Rust `list_invocations` 投影回归 + malformed payload 容错回归。
- Integration tests: 代理 capture 路径 payload 包含 `proxyWeightDelta`。
- E2E tests (if applicable): InvocationTable 展开详情时可见权重变化字段。

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增本 spec 索引并更新状态。
