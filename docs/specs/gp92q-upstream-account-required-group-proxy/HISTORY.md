# 上游账号强制分组代理约束 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/gp92q-upstream-account-required-group-proxy/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录

- 2026-03-29: 创建 spec，冻结“新增账号必须选分组 + 所有账号上下文请求强制走分组绑定代理 + OAuth exchange/refresh 纳入代理”的实现边界。
- 2026-03-30: 完成前后端约束收口、Storybook 场景补齐与视觉证据回填，严格分组代理校验覆盖新增账号、导入验证与账号上下文请求。
