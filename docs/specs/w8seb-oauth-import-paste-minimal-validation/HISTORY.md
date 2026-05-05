# OAuth 导入最小校验修复与单条粘贴入列 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/w8seb-oauth-import-paste-minimal-validation/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录

- 2026-04-02：创建 follow-up spec，冻结“未消费字段非阻断 + 单条粘贴预校验并入现有导入列表”的范围与验收标准。
- 2026-04-02：完成后端最小校验修复、单条粘贴入列交互、Vitest / Storybook / Rust 回归与本地视觉证据，进入 PR 收敛。
- 2026-04-14：把导入页文件/粘贴阶段收敛为本地轻校验 + 去重 + 入列；`验证并预览` 成为唯一服务端校验入口，并补齐重复拦截的 Vitest / Storybook 覆盖。
