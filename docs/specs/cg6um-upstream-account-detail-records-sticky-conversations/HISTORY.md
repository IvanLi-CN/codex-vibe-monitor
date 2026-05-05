# 上游账号详情调用记录与 Sticky 对话对齐 Live 交互 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/cg6um-upstream-account-detail-records-sticky-conversations/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-30: 创建 spec，冻结共享账号详情 `调用记录` tab、Sticky 选择模型、历史抽屉与 visual evidence gate。
- 2026-03-30: 完成后端查询扩展、共享抽屉 `调用记录` tab、Sticky 富交互组件、Storybook/Vitest/Rust 验证，并生成待主人审批的 mock-only 视觉证据。
- 2026-03-31: 主人批准后，将详情抽屉 `路由` tab 的最终 mock-only 截图写入 spec assets，并在 `## Visual Evidence` 中落盘引用。
- 2026-04-27: 账号详情 `调用记录` tab 新增账号级活动总览，扩展 stats summary/timeseries `upstreamAccountId` 契约，并补充 populated / empty Storybook 视觉证据。
