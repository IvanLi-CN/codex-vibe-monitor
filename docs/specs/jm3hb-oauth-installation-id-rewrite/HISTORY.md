# OAuth 上游 `x-codex-installation-id` 代理侧稳定改写 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/jm3hb-oauth-installation-id-rewrite/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-04-11: 初始化规格，冻结 deployment seed + `account_id` 派生方案。
- 2026-04-11: 完成 OAuth `/v1/responses` installation id 稳定改写、SQLite seed 持久化与回归测试。
- 2026-04-11: 收敛 review finding，恢复 file-backed `/v1/responses` 的压缩 / 超大 body passthrough 护栏，并补充对应回归测试。
