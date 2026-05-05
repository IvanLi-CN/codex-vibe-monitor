# Dashboard 工作中对话卡片：1660 宽屏四栏 follow-up - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/mbnns-dashboard-working-conversations-wide-4col/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-04-07: 新建 follow-up spec，冻结 `desktop1660` 四栏合同、Storybook 宽屏入口、Dashboard 页面级 E2E 护栏与截图授权边界。
- 2026-04-07: 本地实现已补齐 Tailwind `desktop1660` screen、工作中对话 `1 / 2 / 3 / 4` 栏合同、8 卡 Storybook mock、Dashboard 页面级宽屏 fixture 与 Playwright 列数回归；同时修正 Dashboard 日历月标签在宽屏 mock 下的 5px 页面级外溢，避免用放宽断言掩盖真实 overflow。
- 2026-04-07: 本地验证已完成 lint、targeted Vitest、frontend build、Storybook build、Dashboard 宽屏 E2E 与 review-loop；主人已确认视觉结果可继续，最终截图已写回 spec 并与本地实现一起推进到 PR merge-ready。
