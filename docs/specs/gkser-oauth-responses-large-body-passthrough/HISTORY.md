# OAuth `/v1/responses` 大包体直通与 distinct-account 记账修复 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/gkser-oauth-responses-large-body-passthrough/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录

- 2026-04-09: 创建 spec，冻结 OAuth `/v1/responses` large-body passthrough、small-body rewrite 保留与 distinct-account 记账修复范围。
- 2026-04-09: 完成 file-backed passthrough、gzip stream hint 线性解析、buffered success hop-by-hop header 过滤与 targeted regressions。
