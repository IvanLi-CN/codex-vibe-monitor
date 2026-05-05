# OAuth 配额耗尽账号误标为上游拒绝修复 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/js2gr-oauth-quota-exhausted-rate-limit-status/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-25: 创建 follow-up spec，冻结 OAuth quota-exhausted 账号误标为“上游拒绝”的修复目标、验收标准与视觉证据位置。
- 2026-03-25: 完成后端状态派生修复、前端 Storybook 场景与测试回归，并补齐 Storybook mock-only 截图与 `chrome-devtools` smoke 证据。
- 2026-03-25: 快车道收敛到 PR #231，视觉证据已获主人批准并随分支提交，spec 状态切换为完成。
