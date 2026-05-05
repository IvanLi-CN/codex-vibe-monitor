# 全站 segmented control family 统一与 Dashboard 样式修复 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/h5k2r-segmented-control-family/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-24: 创建 spec，冻结 segmented control family 的范围、接口与 merge-ready 收口目标。
- 2026-03-24: 完成共享 primitive、现存调用点迁移、Storybook 新 story、Vitest 回归与本地 `bun run test` / `bun run build` / `bun run build-storybook` 验证。
- 2026-03-24: PR #220 完成 labels、远端 checks 与 `codex review --base origin/main` 收敛，快车道终态更新为 merge-ready。
- 2026-03-24: 根据截图复核修正深色主题 active 文本色，并补充深色 / 亮色裁剪验收图到 `./assets/`。
