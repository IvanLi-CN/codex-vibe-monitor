# InvocationTable 推理强度徽标色阶优化 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/hbqe3-invocation-reasoning-effort-badge-colors/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-07: 初始化规格，锁定“推理强度颜色梯度优化 + Storybook/测试同步”范围。
- 2026-03-07: 完成徽标色阶实现与 Storybook 文档更新；已通过 `cd web && npm run test -- --run src/components/InvocationTable.test.tsx`、`cd web && npm run build`、`cd web && npm run build-storybook`。
- 2026-03-07: 快车道推进到 PR #94，并按 review-loop 修复 Tailwind opacity token 发射缺口与原型链键误命中问题；当前 checks 全绿。
- 2026-03-07: 根据主人确认补充 Storybook 视觉证据截图，并同步到 spec/PR 证据链。
