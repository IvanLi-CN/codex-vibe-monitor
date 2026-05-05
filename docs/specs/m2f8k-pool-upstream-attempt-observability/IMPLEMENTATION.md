# 号池逐次上游尝试明细、三账号 failover 上限与 7+30 保留 - Implementation

## Current State

- Canonical spec: `docs/specs/m2f8k-pool-upstream-attempt-observability/SPEC.md`
- Implementation summary: 进行中

## Migrated Implementation Notes

## 状态

- Status: 进行中
- Created: 2026-03-22
- Last: 2026-04-10
- Note: 在线历史读取语义已由 `#h9r2m` 接管；本 spec 中涉及 archive 回读的描述仅保留为原始设计背景，不再代表当前契约。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust: 覆盖 attempt 落库、3 账号预算、同账号 retry 计数、attempt retention/archive/archive TTL 清理。
- Vitest: 覆盖详情懒加载、非 pool 空态、attempt 列表渲染与错误态。

## 文档更新（Docs to Update）

- `docs/specs/README.md`：登记本 spec 与当前状态。
