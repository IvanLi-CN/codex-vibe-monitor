# 请求列表 Fast 模式标识（service tier 版） - Implementation

## Current State

- Canonical spec: `docs/specs/rw32e-invocation-fast-mode-indicator/SPEC.md`
- Implementation summary: 已完成（5/5）

## Migrated Implementation Notes

## 状态

- Status: 已完成（5/5）
- Created: 2026-03-07
- Last: 2026-03-07

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: Rust 覆盖非流 / 流响应 `service_tier` 提取、`list_invocations` 字段投影与历史回填。
- Integration tests: proxy capture / XY record 持久化后 payload 能携带 `serviceTier`。
- E2E tests (if applicable): invocation table 的 Playwright 回归验证 priority 图标显示、详情字段存在、flex 不点亮。

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增规格索引并同步状态。
