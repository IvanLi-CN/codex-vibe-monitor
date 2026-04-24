# 实现跟踪（#swze7）

## Backend

- [x] SQLite schema 增加 `pool_upstream_accounts.verified_email`
- [x] SQLite schema 增加 `pool_oauth_login_sessions.email`
- [x] OAuth callback / relogin / refresh 区分 `email` 与 `verifiedEmail`
- [x] display name mixed-plan 豁免改为 same-upstream + different-known-plan only
- [x] Rust tests 覆盖 migration / refresh / mixed-plan guard

## Web

- [x] create payload / detail payload / story runtime 补齐 `email` 与 `verifiedEmail`
- [x] 单 OAuth 新增页复用 mailbox 输入承载 email 与 email chooser
- [x] 批量 OAuth 行复用 mailbox chip / popover 编辑邮箱并接入 email chooser
- [x] API Key 新增页支持 email 输入
- [x] 详情编辑页复用现有 edit tab 的 email 字段与 verifiedEmail 提示
- [x] 关键完成态加计划 badge

## Validation

- [x] cargo fmt / check / test
- [x] web vitest / build / build-storybook
- [x] 视觉证据入 spec assets
- [ ] PR 收敛到 merge-ready
