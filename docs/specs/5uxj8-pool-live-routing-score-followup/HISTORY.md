# pool `/v1/*` live 路由显式综合打分 follow-up - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/5uxj8-pool-live-routing-score-followup/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录

- 2026-04-18: 创建 follow-up spec，冻结“live 先定账号、显式综合打分、node shunt 仅降权不拒派、concrete owner block 保留账号身份”的实现边界。
- 2026-04-18: 完成 live 显式评分、assigned-account blocked 持久化与 Invocation / Dashboard 失败语义修正；本地 `cargo test`、Vitest、Vite build 与 Storybook 视觉证据已通过。
