# OAuth 手动邮箱附着与增强能力判定（#m7a9k）

## 状态

- Status: 已实现，待 PR / CI 收敛
- Created: 2026-03-17
- Last: 2026-03-17

## 背景

- `3n287` 已为 OAuth 创建流接入 MoeMail 生成邮箱、验证码解析与邀请态展示，但单账号新增与 reauth 仍默认只服务“系统生成的新邮箱”。
- 运营侧需要把已经存在、且当前 MoeMail API Key 可以读取的邮箱地址直接绑定到 OAuth 创建页，避免重复生成地址或切回外部邮箱页查看验证码。
- 现有前后端契约使用 `generatedMailboxAddress` 命名，已经不足以表达“生成”与“附着已有地址”两条路径，且会误导删除/过期清理策略。

## 目标 / 非目标

### Goals

- 为单账号 OAuth 新增页与 `accountId` reauth 页增加“手动输入 / 设置邮箱地址”能力。
- 当手动输入地址可被当前 MoeMail API Key 枚举并读取时，继续启用验证码解析、邀请态识别、复制邮箱与复制验证码等增强能力。
- 当地址格式非法、域名不受当前 MoeMail 支持，或虽然域名受支持但地址不可读时，返回非阻塞降级结果：允许 OAuth 主流程继续，但禁用增强能力。
- 把 OAuth 创建 / 完成请求中的邮箱绑定字段统一为 `mailboxAddress`，并为邮箱会话持久化 `generated` / `attached` 来源，避免误删用户已有邮箱。

### Non-goals

- 不扩展批量 OAuth 行级手动邮箱输入；批量场景继续只支持系统生成邮箱。
- 不扩展 API Key 账号创建流，也不支持 MoeMail 之外的邮箱供应商。
- 不把“不支持增强能力的邮箱地址”升级为 OAuth 主流程硬错误。
- 不提供按地址模糊搜索、邮箱历史浏览器或额外邮箱管理页。

## 功能规格

### 后端 / 数据

- `POST /api/pool/upstream-accounts/oauth/mailbox-sessions` 请求体新增可选 `emailAddress`：
  - 缺省时沿用生成新邮箱路径。
  - 提供时，后端先做基础邮箱格式校验，再调用 MoeMail `GET /api/config` 获取支持域名，并通过 `GET /api/emails` 列表按归一化地址精确匹配可读邮箱。
- `pool_oauth_mailbox_sessions` 新增 `mailbox_source`（`generated` / `attached`）列；旧数据按 nullable 兼容。
- `DELETE /api/pool/upstream-accounts/oauth/mailbox-sessions/:sessionId` 与过期清理都必须只对 `generated` 会话调用 MoeMail 远端删除；`attached` 会话仅删本地记录。
- OAuth 创建 / 完成请求体改用 `mailboxAddress`；服务端兼容读取旧字段 `generatedMailboxAddress`，但内部绑定校验与错误消息统一按新命名执行。

### 前端 / 交互

- 单账号 OAuth 新增页与 reauth 页的邮箱区域改为“可编辑邮箱输入 + `Use address` + `Generate`”双入口。
- 当 `Use address` 成功附着可读邮箱时，页面显示附着成功的邮箱 chip 与 `attached` 来源标识；验证码、邀请态、复制按钮与状态轮询继续生效。
- 当手动输入返回 `supported=false` 时，页面保留输入值并显示明确说明；验证码 / 邀请态区禁用，但“生成 OAuth 链接”和“完成登录”按钮仍可继续使用。
- 当输入值与当前已附着 / 已生成的邮箱地址不一致时，页面应清空本地邮箱会话与相关增强状态，避免把旧邮箱结果误用到新地址。

### 状态机与契约

- `OauthMailboxSession` 改为判别联合：
  - `supported=true`：返回 `sessionId`、`emailAddress`、`expiresAt`、`source`
  - `supported=false`：返回 `emailAddress`、`reason`
- `reason` 仅使用 `invalid_format`、`unsupported_domain`、`not_readable` 三种受控值。
- 只有 `supported=true` 的邮箱会话才允许参与状态轮询、验证码 / 邀请摘要展示，以及 OAuth begin / complete 的 `mailboxSessionId + mailboxAddress` 绑定。

## 验收标准

- Given 用户在单账号 OAuth 新增页输入一个当前 MoeMail API Key 可读取的邮箱地址，When 点击 `Use address`，Then 页面附着该地址并继续提供验证码解析、邀请态识别、复制邮箱与复制验证码能力。
- Given 用户在 reauth 页输入一个可读取邮箱地址，When 完成授权，Then begin / complete 请求使用 `mailboxSessionId + mailboxAddress` 通过后端绑定校验，且原账号更新成功。
- Given 用户输入格式错误、域名不受支持或地址不可读，When 点击 `Use address`，Then 页面返回非阻塞提示、禁用增强能力，但 OAuth 主流程不被拦截。
- Given 一个 `attached` 会话被显式删除或因过期被清理，When 清理发生，Then 只删除本地记录，不调用 MoeMail 远端删除；`generated` 会话继续保留原有远端删除行为。
- Given 前端仍传旧字段 `generatedMailboxAddress`，When 服务端处理 OAuth begin / complete，Then 兼容读取成功，但新代码路径统一写入 `mailboxAddress`。

## 质量门槛

- `cargo check`
- `cargo test`
- `cd web && bun run test`
- `cd web && bun run build`
- 浏览器 smoke：单账号 OAuth 新增页与 reauth 页各验证 1 次手动邮箱支持 / 非支持降级路径

## 实现备注

- Rust 侧在 `src/upstream_accounts/mod.rs` 扩展邮箱会话创建入口、MoeMail config/emails 列表读取、`mailbox_source` 持久化与 `mailboxAddress` 绑定兼容。
- Web 侧在 `web/src/lib/api.ts`、`web/src/hooks/useUpstreamAccounts.ts` 与 `web/src/pages/account-pool/UpstreamAccountCreate.tsx` 对齐联合类型、手动附着入口与单账号 OAuth UI。
- Storybook、文案与 Vitest 场景同步覆盖支持 / 不支持 / attached / generated 路径。

## 验证结果

- 2026-03-17: `cargo check`
- 2026-03-17: `cargo test`
- 2026-03-17: `cd /Users/ivan/.codex/worktrees/5aab/codex-vibe-monitor/web && bun run test`
- 2026-03-17: `cd /Users/ivan/.codex/worktrees/5aab/codex-vibe-monitor/web && bun run build`
- 2026-03-17: 浏览器 smoke 待补充（单账号 OAuth 新增 / reauth 的手动邮箱支持与不支持降级）

## 变更记录

- 2026-03-17: 创建增量 spec，显式覆盖 `3n287` 中“仅支持系统生成邮箱增强”的旧边界，并冻结单账号 OAuth / reauth 的手动邮箱附着语义。
- 2026-03-17: 完成后端附着逻辑、邮箱来源清理策略、前端联合类型与手动邮箱交互，并通过本地 Rust / Web 自动化验证。
