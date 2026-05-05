# Dashboard 工作中对话卡片头部压缩 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/s8zhn-dashboard-working-conversations-header-compact/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-04-08: 新建 follow-up spec 并登记 `docs/specs/README.md`。
- 2026-04-08: 工作中对话卡片头部压成单行，bare hash formatter 复用到卡片与抽屉 header。
- 2026-04-08: 补齐 Vitest 与 Storybook 覆盖，锁定“无 raw key 可见文本、无 `WC-` 前缀、交互不回退”。
- 2026-04-08: 完成 lint / targeted Vitest / build / Storybook build，并生成本地视觉证据。
- 2026-04-08: 在主人确认截图可提交后，把最终视觉证据写回 spec，并继续推进 PR 到 merge-ready。
- 2026-04-24: 复用同一 topic spec 记录后续修复：账号 chip 改为桌面优先单行截断，并把 compact endpoint 可见性同步到 Dashboard 卡片与 README dense 证据源。
