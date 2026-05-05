# 402 `deactivated_workspace` 账号状态改判为上游拒绝 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/kfgvy-upstream-http-402-upstream-rejected/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-26: 创建 follow-up spec，冻结 402 `deactivated_workspace` 账号应显示“上游拒绝”的修复范围与验收口径。
- 2026-03-26: 完成后端结构化 `402` 状态派生修复、route/sync 双回归、Storybook 402 场景、前端断言与本地视觉证据采集。
- 2026-03-26: 分支 `th/9t4zq-upstream-http-402-rejected` 已推送，PR #244 已创建并打上 `type:patch` / `channel:stable`，进入 merge-ready 状态。
- 2026-04-08: 回填 sync-classified hard-unavailable follow-up，要求旧 quota / 429 marker 不得再盖掉新的 `upstream_http_402`；Storybook 402 场景同步加入历史 quota 事件，固定“上游拒绝 + 不可用”的最终展示。
