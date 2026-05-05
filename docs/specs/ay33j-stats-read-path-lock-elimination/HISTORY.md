# Dashboard / stats 读链路 SQLite 锁冲突治理 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/ay33j-stats-read-path-lock-elimination/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-04-12: 创建 stats read-path lock elimination spec，冻结根因、范围、验收与 merge-ready 终点。
- 2026-04-12: 完成共享 stats 读链路去写化、后台 catch-up orchestration 与锁竞争回归；本地 `cargo fmt/check/test` 全绿，Dashboard smoke 未再出现 `database is locked`。
