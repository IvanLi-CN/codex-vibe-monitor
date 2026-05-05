# 账号详情抽屉统一关闭语义与 Tabs 分组 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/qdyfv-account-detail-drawer-tabs/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-25: 创建 spec，冻结共享抽屉壳层、tabs taxonomy、视觉证据与 fast-track merge-ready 收口标准。
- 2026-03-25: 完成共享 drawer shell、号池详情 tabs、Invocation 只读详情 tabs、i18n、Vitest 与 Storybook 覆盖；本地定向 `vitest` 与 `web build` 已通过，并根据最新反馈把配额卡并回概览页签，等待重新抓取 mock-only 视觉证据。
- 2026-03-25: 已按最新反馈重拍 mock-only 视觉证据，截图提交授权已获确认，进入 PR / CI 收敛阶段。
- 2026-03-25: PR #230 已完成 labels、远端 checks 与 `codex review --base origin/main` 收敛，快车道终态更新为 merge-ready。
- 2026-03-27: 统一账号详情抽屉的缺失窗口占位契约；共享 usage card 在 `window == null` 时统一显示 ASCII `-`，仅在 `window != null && history 为空` 时保留既有图表 empty state，并补充号池详情 / Invocation 只读抽屉的 Storybook 与集成测试覆盖。
