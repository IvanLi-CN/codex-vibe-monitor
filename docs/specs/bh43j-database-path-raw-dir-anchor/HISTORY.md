# 数据库环境变量重命名与 raw 路径锚点修复 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/bh43j-database-path-raw-dir-anchor/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-09: 创建规格，冻结 `DATABASE_PATH` 替代 `XY_DATABASE_PATH`、legacy env fail-fast 与 raw/archive 相对路径锚定数据库目录的边界。
- 2026-03-09: 完成后端实现、文档迁移、PR #106 与 review-loop 收敛；在合并最新 `main` 后继续保持回归测试落在 `src/tests/mod.rs`。
