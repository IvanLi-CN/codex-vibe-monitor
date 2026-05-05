# 浮层提示磨砂隔离一致性修复 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/gmycv-overlay-prompt-frosting-consistency/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-25: 创建 follow-up spec，冻结浮层提示磨砂隔离一致性修复的范围、验收标准与视觉证据路径。
- 2026-03-25: 新增共享 `floating-surface` helper，并让 `bubble.ts`、更新横幅、通用 tooltip、inline chart tooltip 与系统通知统一消费同一套 overlay surface token。
- 2026-03-25: 补齐 `UpdateAvailableBanner`、`InfoTooltip`、`Overlay Surface Gallery` Storybook 预览，以及 shared surface contract / 组件交互回归测试。
- 2026-03-25: 本地验证通过：`cd web && bun run test`、`cd web && bun run build`、`cd web && bun run build-storybook`；已通过 Storybook + `chrome-devtools` 生成浅/深主题 mock-only 视觉证据。
- 2026-03-25: 根据本地 review 修正 `bubble.ts` 阴影被 `!shadow-none` 覆盖的问题，并让自定义实心 tooltip 在显式覆盖背景时一并退出共享 blur。
- 2026-03-25: 刷新 Overlay Surface Gallery 证据入口，改为稳定展示 tooltip、InfoTooltip、inline chart tooltip 与系统通知 toast；浅/深主题截图已重新落盘。
- 2026-03-25: 快车道远端步骤待继续；因本次 evidence 需要随 spec / PR 一起提交图片文件，push 前仍需主人确认是否允许提交这些截图资源。
