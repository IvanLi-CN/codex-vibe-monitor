# 修复 InvocationTable 异常横向滚动 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/26knq-invocation-table-overflow/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-02-26: 创建规格并记录线上复现基线（Dashboard: `clientWidth=1108`、`scrollWidth=1152`、`maxScrollLeft=44`；首行展开按钮默认被裁剪约 `31px`）。
- 2026-02-26: 完成组件宽度修复与 E2E 回归（新增 `web/tests/e2e/invocation-table-layout.spec.ts`），本地验证 `npm run build` 与新增 E2E 均通过。
- 2026-02-26: 完成快车道收敛：PR #56 已创建并打上 `type:patch` + `channel:stable`，CI/checks 通过；review-loop 第 1 轮发现的 HashRouter 路径问题已修复并回归通过。
