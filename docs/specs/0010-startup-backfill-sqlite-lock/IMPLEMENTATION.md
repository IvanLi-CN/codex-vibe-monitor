# 启动回填 SQLite 锁冲突修复 - Implementation

## Current State

- Canonical spec: `docs/specs/0010-startup-backfill-sqlite-lock/SPEC.md`
- Migrated from legacy source: `docs/plan/0010:startup-backfill-sqlite-lock/PLAN.md`
- Legacy source retention: pending delete approval
- Implementation summary: See companion notes and linked PR/check history for implementation context.

## Migrated Implementation Notes

## Testing

- `cargo fmt --check`
- `cargo test`
- `cargo check`
- 覆盖：
  - 回填批处理幂等路径
  - 锁冲突重试成功路径
  - 锁冲突重试失败路径
  - 非锁错误“立即失败且不重试”路径
  - SQLite 连接参数默认值（WAL + busy timeout）

## Milestones

- [x] M1 启动连接参数显式化（WAL + busy timeout）
- [x] M2 回填算法重构为批处理读写
- [x] M3 启动回填锁冲突重试包装器
- [x] M4 测试补齐与回归验证
