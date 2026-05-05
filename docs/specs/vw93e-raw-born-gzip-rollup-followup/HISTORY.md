# raw 保真降本与历史维护追平 follow-up - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/vw93e-raw-born-gzip-rollup-followup/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-04-13: 创建 follow-up spec，冻结 born-gzip、bounded historical rollup auto-heal 与 upstream-rejected cooldown 的范围与验收。
- 2026-04-13: 完成 born-gzip raw capture、bounded historical rollup auto-heal、upstream-rejected 6h cooldown、README/deployment 同步，以及本地 + shared-testbox 验证。
- 2026-04-17: 修正 maintenance-upstream-rejected cooldown 的实现漂移：`cooldown_until` 改为显式真相源，并为 legacy 空字段行补齐 bounded reconciliation。
