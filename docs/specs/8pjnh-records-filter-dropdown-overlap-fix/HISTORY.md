# 请求记录筛选下拉遮挡修复 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/8pjnh-records-filter-dropdown-overlap-fix/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-12: 创建 hotfix spec，冻结页面级抬层方案、回归范围与视觉证据要求。
- 2026-03-12: 已完成 Records 页层级热修、Vitest / Playwright / build 验证，并补充本地 mock overlap 视觉证据。
- 2026-03-12: PR #116 checks 全部成功，codex review loop 清零，无剩余阻塞项。
- 2026-03-12: 补充 1279px 非 xl 窄桌面断点的 Playwright 遮挡 smoke，降低 breakpoint 回归风险。
- 2026-03-12: 将 front-end Vitest 与 Records overlay Playwright 定点回归接入 CI gate，避免仅本地守护。
