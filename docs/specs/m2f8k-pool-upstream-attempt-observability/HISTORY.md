# 号池逐次上游尝试明细、三账号 failover 上限与 7+30 保留 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/m2f8k-pool-upstream-attempt-observability/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-22: 创建 spec，冻结 pool attempts schema、3 账号预算、attempt API、详情懒加载与 7+30 retention 策略。
- 2026-03-23: 更新契约，要求 attempt 在开始时插入 `pending` 行，并通过 `pool_attempts` SSE 实时推送后续状态变化。
