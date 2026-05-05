# Release `latest` 仅指向最新已发布 stable - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/8239m-release-latest-published-stable/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-29：落地 immutable tag / publish-time `latest` 分离，补齐 pending stable、旧 stable rerun/backfill 与 rc 语义回归，并完成本地 `test-release-snapshot` 验证。
- 2026-03-29：创建 PR #266，进入 fast-track 的 CI / review 收敛阶段。
