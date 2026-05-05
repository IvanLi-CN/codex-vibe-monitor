# OAuth 临时邮箱自动化与验证码/邀请态集成 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/3n287-oauth-temp-mail-automation/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录

- 2026-03-16: 创建增量 spec，冻结 MoeMail env 契约、邮箱绑定规则、验证码/邀请解析语义，以及单个/批量 UI 门禁与轮询行为。
- 2026-03-16: 完成前后端实现、文案与测试，并补充本地浏览器 smoke 结果。
- 2026-03-16: 根据最新反馈改为紧凑邮箱 UI，允许名称与邮箱不同，并补齐相应 Storybook 场景。
- 2026-03-17: 标记单账号 OAuth / reauth 的手动邮箱附着能力已转入增量 spec `m7a9k`，避免继续把“仅支持系统生成邮箱”误视为当前边界。
