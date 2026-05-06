# KaisouMail OAuth 邮箱适配 - History

## Key Decisions

- 2026-05-07: 新建 KaisouMail mailbox adapter 规范，取代 OAuth mailbox 链路对 MoeMail 的外部 API 依赖。
- 2026-05-07: 保留项目内 OAuth mailbox session API 与 `generated` / `attached` 生命周期语义，降低前端和 OAuth 主流程改动面。

## Change Log

- 2026-05-07: 记录 KaisouMail Bearer API、配置 env、消息读取和远端清理契约。
