# 建立全局 UI 规范文档体系 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/quhzx-ui-guidelines-system/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-18: 创建 spec，冻结 docs-only UI 规范补档范围、验收标准与 fast-track 交付路径。
- 2026-03-18: 完成 `docs/ui/` 六份文档、README 入口与本地验证；进入 PR 交付与 review 收敛阶段。
- 2026-03-18: 修复 review 指出的 specs 索引表渲染问题与 foundations spacing 约束缺口，随后同步 `origin/main`、更新 PR #173 到 mergeable clean，并确认 checks green / review-loop clear。
- 2026-07-07: 同步前端当前组织方式：页面级和领域级 UI 实现迁入 `web/src/features/<domain>/`，Storybook 扫描同时覆盖 `web/src/components/ui` 与 `web/src/features`。
