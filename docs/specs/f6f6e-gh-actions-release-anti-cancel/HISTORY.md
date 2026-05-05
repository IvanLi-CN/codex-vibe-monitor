# GH Actions 防取消发布链路全面对齐 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/f6f6e-gh-actions-release-anti-cancel/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-14: 创建 strict anti-cancel release topology spec，冻结三段式 workflow + final quality-gates 升级范围。
- 2026-03-14: 完成 workflow split、final quality-gates contract、release backfill 入口与本地 contract/self-tests。
- 2026-03-15: 将发布链路进一步收敛为“PR 标签校验 → 全局串行 `CI Main` 写/补 snapshot → 全局串行 `Release` 按最早未发布 snapshot 排队发布”，删除 artifact、rollout 与 legacy fallback 复杂度。
- 2026-04-29: 增加 `CI Main Gate`，把上游 `CI Main` 非成功结论从 silent skipped 转为显式 failed release；同时允许同仓库 quality-gates contract PR 使用当前分支 contract 自证更新后的拓扑。
