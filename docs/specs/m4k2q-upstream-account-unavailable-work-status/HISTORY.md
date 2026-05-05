# 号池工作状态新增“不可用（不可调度）” - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/m4k2q-upstream-account-unavailable-work-status/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-04-08: 回填 sync-classified hard-unavailable follow-up；旧 quota / 429 marker 不得再把 `401/402/403` 维护同步导回 `rate_limited`，新增后端回归与 Storybook stale-quota 402 场景。
