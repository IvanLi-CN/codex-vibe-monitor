# 批量 OAuth 邮箱气泡编辑与邮箱动作解耦（#e5w9m）

## 状态

- Status: 进行中
- Created: 2026-03-17
- Last: 2026-03-17

## 背景

- `m7a9k` 已经为单账号 OAuth / reauth 落地“手动输入并附着邮箱地址”，但批量 OAuth 行级邮箱仍只有“生成”入口，无法在批量表格内直接补录或替换已有邮箱地址。
- 当前单账号邮箱按钮复用同一个忙碌态，只要触发 `Use address` 或 `Generate` 任一动作，两个按钮都会同时转圈，和真实执行中的动作不一致。
- 运营侧反馈批量场景需要在保持表格紧凑的前提下补一个轻量入口，因此本增量不新增独立字段行，而是把编辑入口塞进邮箱 chip 的悬浮气泡。

## 目标 / 非目标

### Goals

- 单账号 OAuth 邮箱区把 `Use address` / `Generate` 改成 icon 按钮，并且只让当前执行中的动作显示 spin，另一个按钮仅禁用不转圈。
- 批量 OAuth 行级邮箱 chip 新增悬浮气泡编辑入口：默认气泡展示 `Edit mailbox` icon，进入编辑态后在同一气泡内完成邮箱地址输入、提交与取消。
- 批量邮箱提交沿用现有 `POST /api/pool/upstream-accounts/oauth/mailbox-sessions { emailAddress }` 契约；受支持地址继续启用验证码/邀请增强，不支持地址保留降级提示但不阻断 OAuth 主流程。
- 当批量行已经生成过 OAuth URL 且邮箱绑定发生变化时，前端必须清空旧 callback / login session，要求该行重新生成 OAuth URL。

### Non-goals

- 不新增后端 API、数据库列或 MoeMail 解析规则。
- 不把批量邮箱编辑扩展成完整邮箱管理页、历史邮箱列表或模糊搜索。
- 不改变 API Key 创建流或非 MoeMail 邮箱供应商边界。

## 功能规格

### 单账号 OAuth

- 邮箱区 `Use address` 按钮使用 `check-bold` icon，`Generate` 按钮使用 `auto-fix` icon。
- 单账号邮箱忙碌态改为判别动作状态；`attach` 中只让 `Use address` 转圈，`generate` 中只让 `Generate` 转圈。
- 两个按钮在忙碌期都保持禁用，邮箱输入框也保持禁用，防止在同一个邮箱动作中途篡改输入。

### 批量 OAuth

- 每行邮箱 chip 改为“可复制邮箱 + 悬浮气泡编辑器”：
  - 非编辑态：气泡展示当前邮箱内容与 `Edit mailbox` icon；
  - 编辑态：气泡内展示邮箱输入框、`Submit mailbox` icon、`Cancel mailbox edit` icon。
- 批量行的外部 `Generate` 入口改为 icon-only 按钮，并与行内 `Submit mailbox` 动作互斥；任一邮箱动作进行中时，该行其他邮箱按钮都必须禁用。
- 批量手动附着受支持邮箱成功后：
  - 更新 `mailboxSessionId + mailboxAddress` 绑定；
  - 清空旧邮箱状态摘要；
  - 若该行已有 pending OAuth URL，则清空 callback / session 并提示重新生成。
- 批量手动附着返回 `supported=false` 时：
  - 保留用户输入值；
  - 清空 `mailboxSession`，禁用验证码/邀请增强；
  - 若之前没有邮箱会话绑定，则保持当前 OAuth URL 可继续使用；
  - 若之前已有邮箱会话绑定，则该行改为需要重新生成 OAuth URL。

## 验收标准

- Given 单账号邮箱区正在执行 `Use address`，When UI 渲染忙碌态，Then 仅 `Use address` 按钮转圈，`Generate` 仅禁用不转圈。
- Given 批量 OAuth 某行已有邮箱 chip，When 用户 hover 该 chip，Then 气泡内可见 `Edit mailbox` icon，点击后进入编辑态并显示输入框与提交/取消 icon。
- Given 批量 OAuth 某行提交一个受支持邮箱地址，When 提交成功，Then 该行后续生成 OAuth URL 时会带上新的 `mailboxSessionId + mailboxAddress`。
- Given 批量 OAuth 某行提交一个不支持读取的邮箱地址，When 提交成功，Then 该行保留输入地址并显示降级提示，但不阻断没有邮箱绑定的 OAuth URL 继续使用。
- Given 批量 OAuth 某行已经生成过 OAuth URL 且随后改绑到新的受支持邮箱，When 绑定成功，Then 旧 OAuth URL 不再可复制/提交，用户必须重新生成。

## 质量门槛

- `cd web && bun run test`
- `cd web && bun run build`
- `cd web && bun run build-storybook`

## 实现备注

- 主要变更位于 `web/src/pages/account-pool/UpstreamAccountCreate.tsx`、`web/src/components/account-pool/OauthMailboxChip.tsx`、`web/src/i18n/translations.ts` 与相应的 Storybook / Vitest 文件。
- 本增量显式覆盖 `m7a9k` 中“批量 OAuth 不扩展手动邮箱输入”的旧边界；单账号邮箱契约与服务端 `mailboxAddress` 语义保持不变。
