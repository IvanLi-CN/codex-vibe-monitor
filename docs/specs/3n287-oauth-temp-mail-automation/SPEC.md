# OAuth 临时邮箱自动化与验证码/邀请态集成（#3n287）

## 状态

- Status: 已实现，待 PR / CI 收敛
- Created: 2026-03-16
- Last: 2026-03-16

## 背景

- 当前上游账号 OAuth 创建流已经支持单个与批量登录，但邮箱输入、验证码提取与被邀请状态仍需要人工切到第三方邮箱服务查看，流程割裂且容易漏看。
- 本次对接的临时邮箱服务固定为 MoeMail，且实际项目已验证可通过 `X-API-Key` 读取已创建邮箱、邮件与 webhook 能力，因此适合作为 OAuth 创建流内的服务端代理来源。
- 用户要求把“生成邮箱 -> 使用邮箱登录 -> 在创建页直接读验证码/邀请态”的闭环直接集成进现有 OAuth 页面，并明确约束：生成后名称必须与生成邮箱完整地址严格一致，否则单个模式新增能力整体置灰、批量模式对应行不挂接这些增强能力。

## 目标 / 非目标

### Goals

- 在现有单个 OAuth 与批量 OAuth 创建页中新增服务端驱动的 MoeMail 临时邮箱会话，支持一键生成邮箱、轮询最新状态、删除会话与过期清理。
- 在创建流内解析 OpenAI/ChatGPT 验证码邮件与 workspace/business 邀请邮件，并以适合单个/批量场景的 UI 呈现出来。
- 强化邮箱绑定语义：生成邮箱后，名称必须与生成邮箱完整地址严格相等；失配时单个模式整体置灰，批量模式该行不使用本次新增的邮箱增强能力。
- 扩展前后端契约与测试，确保内部 API、轮询状态机、复制交互和失效状态可稳定回归。

### Non-goals

- 不把 MoeMail 暴露给浏览器，也不在前端持有或显示 MoeMail API Key。
- 不扩展到 API Key 账号创建流，不支持非 MoeMail 供应商。
- 不做完整邮件历史页，只处理 OAuth 创建流中所需的最新验证码 / 邀请摘要。
- 不放宽名称绑定规则，不支持前缀、大小写宽松匹配或“自定义别名仍保留增强能力”。

## 功能规格

### 后端 / 数据

- `AppConfig` 新增 `UPSTREAM_ACCOUNTS_MOEMAIL_BASE_URL`、`UPSTREAM_ACCOUNTS_MOEMAIL_API_KEY`、`UPSTREAM_ACCOUNTS_MOEMAIL_DEFAULT_DOMAIN`；任一缺失时，邮箱会话接口返回明确“未启用”错误，不影响原有 OAuth 登录能力。
- 新增内部邮箱会话存储，至少保存：本地 `sessionId`、MoeMail `emailId`、生成邮箱地址、域名、最近一次验证码摘要、最近一次邀请摘要、最后扫描的邮件标识、创建/更新时间与过期时间；会话为服务端 opaque id。
- 新增内部 API：
  - `POST /api/pool/upstream-accounts/oauth/mailbox-sessions`
  - `POST /api/pool/upstream-accounts/oauth/mailbox-sessions/status`
  - `DELETE /api/pool/upstream-accounts/oauth/mailbox-sessions/:sessionId`
- 后台维护逻辑需对过期邮箱会话做 best-effort 清理，并在删除本地会话前尝试删除 MoeMail 端邮箱；远端删除失败不得阻断本地回收。

### 解析规则

- 验证码提取采用“主题优先、正文兜底、最新命中优先”：先匹配主题中的 `Your ChatGPT code is <digits>` / `Your OpenAI code is <digits>` 一类模式，再回退到 HTML/Text 中靠近 `verification code` 语义的 4-8 位数字块。
- 邀请通知只识别 OpenAI/ChatGPT workspace/business 模板：要求主题命中 `has invited you`，且正文包含 `Join workspace`、`Accept invitation` 等 CTA；若没有独立邀请码，则把邀请链接本身作为可复制内容。
- 同一邮箱多封邮件命中时，状态接口只返回“最新有效验证码”和“最新有效邀请摘要”；无关邮件不得误判为验证码或邀请。

### 前端 / 交互

- 单个 OAuth 页面新增独立邮箱输入框，生成按钮集成在输入框右内侧；生成成功后立即把完整邮箱写入输入框并选中文本；只有当输入框被清空后才允许重新生成。
- 生成邮箱后，单个模式的“名称”输入必须与生成邮箱完整地址严格一致；一旦不一致，验证码卡片、邀请卡片、复制按钮与 invited 指示全部置灰，并停止把轮询结果作为可用增强能力展示；恢复一致后重新启用。
- 单个模式新增专门区域展示：最新验证码、邀请码/邀请链接、复制按钮，以及完整尺寸 invited 状态指示。
- 批量 OAuth 每行新增邮箱增强状态：操作区提供验证码复制按钮；无验证码时置灰，有新验证码时高亮，复制成功后切换为更弱主题色但仍可再次点击；hover 需展示验证码内容。
- 批量 OAuth 的 invited 状态放在左侧序号区域，用主题色表示被邀请；若该行名称与生成邮箱不一致，则该行不挂接本次新增邮箱增强能力。
- 批量与单个轮询默认每 5 秒执行一次；批量模式必须通过单次批量状态查询请求合并读取活跃邮箱会话。

## 接口契约

- OAuth 创建页前端类型新增 `OauthMailboxSession`、`OauthMailboxStatus`、`OauthInviteSummary`，并扩展现有 OAuth 草稿 / 批量行状态，保存 `mailboxSessionId`、`generatedMailboxAddress`、复制状态与最新解析结果。
- OAuth 创建 / 完成流程请求体增加 `mailboxSessionId` 与 `generatedMailboxAddress` 绑定字段，供服务端校验并记录该次登录与生成邮箱的对应关系。
- `src/upstream_accounts/mod.rs`、`web/src/lib/api.ts`、`web/src/pages/account-pool/UpstreamAccountCreate.tsx` 必须对齐同一命名与字段语义。

## 验收标准

- Given MoeMail env 缺失，When 调用邮箱会话接口，Then 返回明确禁用错误，且原有 OAuth 登录流仍可正常使用。
- Given 单个 OAuth 页面生成邮箱成功，When 输入框写入新邮箱，Then 该邮箱文本立即被全选，且未清空前生成按钮不可再次生成。
- Given 单个模式名称与生成邮箱不一致，When 邮箱状态轮询仍返回验证码或邀请，Then 页面新增的验证码/邀请能力整体置灰；恢复一致后重新启用。
- Given 批量 OAuth 某行已生成邮箱且收到验证码，When 用户悬浮并点击复制按钮，Then 可看到验证码内容、按钮可复制；复制成功后样式变弱，但仍可点击；当收到新验证码时按钮重新高亮。
- Given 批量 OAuth 某行名称与生成邮箱不一致，When 行状态更新，Then 该行不显示或不启用本次新增邮箱增强能力，左侧 invited 主题态也不生效。
- Given 邀请邮件缺少独立邀请码，When 单个模式展示邀请摘要，Then 复制按钮复制邀请 CTA 链接本身。

## 质量门槛

- `cargo check`
- `cargo test`
- `cd web && bun run test`
- `cd web && bun run build`
- 浏览器 smoke：本地验证单个 OAuth 与批量 OAuth 的生成邮箱、名称绑定门禁、验证码复制与 invited 状态。

## 实现备注

- Rust 侧在 `src/upstream_accounts/mod.rs` 中落地 MoeMail client、邮箱会话表、状态聚合与过期清理，并把 OAuth login session 扩展为可记录 `mailboxSessionId` / `generatedMailboxAddress` 绑定信息。
- `src/main.rs` 已挂载 `POST /api/pool/upstream-accounts/oauth/mailbox-sessions`、`POST /api/pool/upstream-accounts/oauth/mailbox-sessions/status`、`DELETE /api/pool/upstream-accounts/oauth/mailbox-sessions/:sessionId` 三个内部接口。
- 前端在 `web/src/lib/api.ts`、`web/src/hooks/useUpstreamAccounts.ts` 与 `web/src/pages/account-pool/UpstreamAccountCreate.tsx` 对齐邮箱会话契约，并在单个 / 批量 OAuth 页面分别落地严格名称绑定门禁、5 秒轮询、复制状态机与邀请态呈现。
- 文案、测试与 Storybook 场景已同步到 `web/src/i18n/translations.ts`、`web/src/pages/account-pool/UpstreamAccountCreate.test.tsx`、`web/src/components/UpstreamAccountCreatePage.stories.tsx`。

## 验证结果

- 2026-03-16：`cargo check`
- 2026-03-16：`cargo test`
- 2026-03-16：`cd /Users/ivan/.codex/worktrees/9b13/codex-vibe-monitor/web && bun run test`
- 2026-03-16：`cd /Users/ivan/.codex/worktrees/9b13/codex-vibe-monitor/web && bun run build`
- 2026-03-16：`cd /Users/ivan/.codex/worktrees/9b13/codex-vibe-monitor/web && bun run build-storybook`
- 2026-03-16：浏览器 smoke（`http://127.0.0.1:60080/#/account-pool/upstream-accounts/new?mode=oauth` 与 `?mode=batchOauth`）确认：
  - 单个 OAuth 生成邮箱后会立即写入邮箱框与显示名称，且邮箱框内容被完整选中。
  - 未清空邮箱框前“生成”按钮保持禁用；当显示名称改成与生成邮箱不一致时，验证码 / 邀请区域整体进入置灰状态。
  - 批量 OAuth 第一行生成邮箱后会同步写入显示名称与邮箱框，且在名称严格匹配时出现验证码复制按钮；当名称改为不一致时，该行邮箱增强按钮会直接移除。

## 变更记录

- 2026-03-16: 创建增量 spec，冻结 MoeMail env 契约、邮箱绑定规则、验证码/邀请解析语义，以及单个/批量 UI 门禁与轮询行为。
- 2026-03-16: 完成前后端实现、文案与测试，并补充本地浏览器 smoke 结果。
