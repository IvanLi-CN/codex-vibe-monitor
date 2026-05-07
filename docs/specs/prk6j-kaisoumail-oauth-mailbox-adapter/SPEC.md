# KaisouMail OAuth 邮箱适配（#prk6j）

## 背景 / 问题陈述

- OAuth 创建流需要服务端驱动临时邮箱：生成地址、绑定手动地址、轮询验证码与邀请邮件，并在会话过期或显式删除时清理可回收邮箱。
- 历史实现绑定 MoeMail 的 `X-API-Key` 与 `/api/emails*` 接口；当前项目不再使用 MoeMail，需要切换到 KaisouMail 控制台提供的 Bearer API。
- KaisouMail 生成邮箱地址的 `localPart`、子域名和 root domain 由上游策略决定；项目只负责请求邮箱、保存返回地址并轮询消息。

## 目标 / 非目标

### Goals

- 后端 OAuth mailbox session 继续保持项目内 API 稳定：`POST /api/pool/upstream-accounts/oauth/mailbox-sessions`、`POST /api/pool/upstream-accounts/oauth/mailbox-sessions/status`、`DELETE /api/pool/upstream-accounts/oauth/mailbox-sessions/:sessionId`。
- 外部邮箱服务切到 KaisouMail：认证使用 `Authorization: Bearer <API_KEY>`；邮箱创建使用 `/api/mailboxes` 或 `/api/mailboxes/ensure`；消息读取使用 `/api/messages` 与 `/api/messages/:id`；销毁使用 `DELETE /api/mailboxes/:id`。
- 配置改为 `UPSTREAM_ACCOUNTS_KAISOUMAIL_BASE_URL`、`UPSTREAM_ACCOUNTS_KAISOUMAIL_API_KEY`；默认域名、默认子域名与旧 MoeMail env 明确拒绝并提示迁移或删除。
- 保持现有 `generated` / `attached` 生命周期语义：系统生成或 ensure 出来的邮箱允许远端销毁，附着已有邮箱只删除本地 session。

### Non-goals

- 不新增邮箱管理 UI，不把 KaisouMail API Key 暴露给浏览器。
- 不改变 OAuth 登录、账号池路由、验证码摘要和邀请摘要的项目内响应字段。
- 不创建、撤销或轮换 KaisouMail 真实 API Key。

## 接口契约

- `GET /api/meta` 无需登录即可读取 root domains、TTL 和地址规则；服务端用 `domains[]` 判断手动输入地址是否属于可用 root domain。
- `POST /api/mailboxes` 只传 `expiresInMinutes`，用于系统生成临时邮箱；地址生成完全使用 KaisouMail 上游策略。
- `POST /api/mailboxes/ensure` 请求包含 `address` 和 `expiresInMinutes`，用于手动地址缺失时幂等补建。
- `GET /api/messages?mailbox=<address>&after=<iso>` 返回消息摘要；当前本地 session 仍以 `last_message_id` 做兼容游标，必要时可在后续增量切成时间游标。
- `GET /api/messages/:id` 返回 `{ message }`，正文字段兼容 `content`、`text` 与 `previewText`。

## 验收标准

- Given 只设置旧 `UPSTREAM_ACCOUNTS_MOEMAIL_*`，When 服务启动配置解析，Then 报错提示改用 `UPSTREAM_ACCOUNTS_KAISOUMAIL_BASE_URL` / `UPSTREAM_ACCOUNTS_KAISOUMAIL_API_KEY` 或删除旧默认域名变量。
- Given 设置 `UPSTREAM_ACCOUNTS_KAISOUMAIL_DEFAULT_MAIL_DOMAIN` 或 `UPSTREAM_ACCOUNTS_KAISOUMAIL_DEFAULT_SUBDOMAIN`，When 服务启动配置解析，Then 报错提示删除这些变量。
- Given 未配置完整 KaisouMail env，When 调用 OAuth mailbox session 接口，Then 返回明确未启用错误，且 OAuth 主流程不受影响。
- Given 手动地址属于 `GET /api/meta domains[]` 的 root domain，When 该地址已存在，Then session `source=attached` 且不会远端创建或销毁该 mailbox。
- Given 手动地址属于可用 root domain 但不存在，When 创建 session，Then 服务端调用 KaisouMail ensure 并保存返回 mailbox id，session `source=generated`。
- Given generated session 被显式删除或过期清理，When 清理发生，Then 服务端 best-effort 调用 `DELETE /api/mailboxes/:id`；attached session 不调用远端删除。
- Given mailbox 收到 OpenAI/ChatGPT 验证码或邀请邮件，When 状态刷新，Then 项目内响应继续返回 `latestCode` / `invite` 摘要。

## 参考

- KaisouMail 控制台文档：`https://km.707979.xyz/api-keys/docs`
- 历史来源：`docs/archive/specs/3n287-oauth-temp-mail-automation/SPEC.md`
- 历史来源：`docs/archive/specs/m7a9k-oauth-manual-mailbox-attach/SPEC.md`
