# 后端结构债双 PR 快车道 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/9vau7-backend-structure-dual-pr-followup/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-04-12: 创建双 PR 后端结构债 fast-track spec，冻结 PR1/PR2 范围与 merge+cleanup 终点。
- 2026-04-12: PR1 已完成 `proxy + upstream_accounts/routing` 真模块化并合入 `main`，本地 `cargo fmt/check/test` 与 shared-testbox `proxy-parallel/raw` smoke 通过。
- 2026-04-12: PR2 已完成 `api + hourly_rollups router builders` 结构收敛，新增 `scripts/shared-testbox-api-read-smoke`，本地 `cargo fmt/check/test` 与 shared-testbox `api-read/proxy-parallel/raw` smoke 通过。
