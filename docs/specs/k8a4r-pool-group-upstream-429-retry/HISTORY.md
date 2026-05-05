# 号池分组级上游 429 重试与随机回退 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/k8a4r-pool-group-upstream-429-retry/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录

- 2026-03-30: 创建 spec，冻结分组级上游 `429` 重试的 schema、接口、运行时与 UI 验收口径。
- 2026-03-30: 完成 schema / API / pool runtime / Storybook UI 实现，并补充本地视觉证据。
- 2026-03-30: 合入 `origin/main` 的上游账号筛选持久化基线后，重新完成前端门禁与 Storybook 证据抓取，并将截图授权状态切换为 `approved`。
- 2026-03-30: 远端 checks 与 fresh review-proof 已收敛，spec 状态切换为已完成。
