# Proxy usage 解析补全与默认来源口径回归 - Implementation

## Current State

- Canonical spec: `docs/specs/0009-proxy-usage-backfill-and-source-scope/SPEC.md`
- Migrated from legacy source: `docs/plan/0009:proxy-usage-backfill-and-source-scope/PLAN.md`
- Legacy source retention: pending delete approval
- Implementation summary: See companion notes and linked PR/check history for implementation context.

## Migrated Implementation Notes

## Testing

- `cargo fmt`
- `cargo test`
- `cargo check`
- 线上最小验证（部署后）：
  - `GET /api/stats`：`totalTokens > 0` 且总量不再只等于 proxy 小样本。
  - `GET /api/invocations?limit=20`：最新 `proxy` 记录 token 字段出现非空值。

## Milestones

- [x] M1 gzip usage 解析修复与缺失原因可观测
- [x] M2 默认来源口径回归到 All
- [x] M3 启动期历史回填（含配置开关）
- [x] M4 回归测试与线上验证
