# Responses overload early gate preview-cap follow-up - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/br38t-responses-overload-preview-cap-followup/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录

- 2026-04-22: 创建 `#bk2pt` follow-up spec，冻结“preview cap 不得提前关闭 metadata-only overload 透明重试窗，且永不对 downstream-visible output 做回放”的实现边界。
- 2026-04-22: 完成 early gate buffer cap 解耦，新增超长 metadata-only overload gate / integration 回归；本地 `cargo fmt`、`cargo check` 与 5 条 targeted cargo tests 已通过。
