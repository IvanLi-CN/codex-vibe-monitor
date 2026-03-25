# 修复新增账号页 OAuth 地址被字段编辑重置（#j86ms）

## 状态

- Status: 已实现，待 PR / CI 收敛
- Created: 2026-03-25
- Last: 2026-03-25

## 背景

- 新增账号页的单账号 OAuth 与批量 OAuth 都已经支持分组、分组共享备注、标签、母号与邮箱绑定等元数据输入，但这些字段在 OAuth URL 已生成后仍会继续变化。
- 当前实现把这类字段编辑视为“必须重新生成 OAuth URL”的硬失效条件，导致用户在新增流程里只要改动分组或邮箱草稿，就会丢失已经生成的授权地址与 callback 输入。
- 运营反馈这会显著打断手工 OAuth 流程，尤其是在批量场景里，默认分组回填到已有 pending 行时也会把整行 URL 重置。

## 目标 / 非目标

### Goals

- 为 pending OAuth login session 增加独立的 metadata 更新契约，允许在不轮换 `loginId / authUrl / redirectUri / expiresAt` 的前提下热更新最终落库元数据。
- 单账号 OAuth 与批量 OAuth 统一改成“本地草稿 + pending session sync”模型：字段编辑同步 session，不再清空现有 OAuth URL 或 callback 输入。
- 覆盖单账号字段编辑、批量行级编辑、默认分组传播、组备注草稿保存、标签、母号与邮箱绑定变化。
- 保持 `failed / expired / completed` 会话的终态语义不变，这些状态仍不可编辑。

### Non-goals

- 不改变 OAuth PKCE、code exchange、redirect URI 生成规则。
- 不扩展 API Key 创建流，也不改动新增账号页之外的详情编辑或列表页逻辑。
- 不新增新的 SQLite 列；继续复用现有 `pool_oauth_login_sessions` metadata 字段。

## 接口变更

### 后端

- 新增 `PATCH /api/pool/upstream-accounts/oauth/login-sessions/:loginId`。
- 请求体 `UpdateOauthLoginSessionPayload` 支持：
  - `displayName`
  - `groupName`
  - `note`
  - `groupNote`
  - `tagIds`
  - `isMother`
  - `mailboxSessionId`
  - `mailboxAddress`
- 接口只允许更新 `pending` 且未过期的会话；`completed / failed / expired` 必须返回明确错误。
- 返回值继续复用 `LoginSessionStatusResponse`，且不得轮换 OAuth 地址相关字段。

### 前端

- `web/src/lib/api.ts` 与 `useUpstreamAccounts` 增加 `updateOauthLoginSession` 能力。
- 新增账号页在 pending session 存在时，文本字段走去抖同步，离散控件变更走即时同步。
- `Complete OAuth` 在提交 callback 前必须先 flush 未完成的 metadata sync。
- 只有用户显式点击 `Regenerate OAuth URL` 时，才允许替换现有 pending session。

## 功能规格

### Pending session 生命周期

- 当 OAuth URL 已生成且 session 仍为 `pending` 时，编辑 `displayName / groupName / note / groupNote / tagIds / isMother / mailboxSessionId / mailboxAddress` 只更新当前 login session 的 metadata。
- 服务端更新时必须复用现有 display name 唯一性校验、tag 校验、组备注草稿适用性校验与邮箱绑定校验。
- callback 完成落库与手动 `Complete OAuth` 都必须使用 session 中最后一次同步后的 metadata，而不是最初生成 URL 时的旧值。

### 单账号 OAuth

- 编辑显示名称、分组、分组备注、标签、母号、邮箱绑定或邮箱草稿分离时，不再显示“Generate a fresh OAuth URL”提示。
- 当已绑定邮箱输入与当前 mailbox session 发生分离时，页面仍会停用旧邮箱增强态，但 pending OAuth URL 保持可复制、可完成，并把 session metadata 更新为当前邮箱绑定状态。

### 批量 OAuth

- 行内编辑 `groupName / note / isMother / mailbox binding` 时，已有 pending 行只同步 metadata，不清空 `callbackUrl`、`session` 或 `sessionHint`。
- 顶部默认分组传播到继承行时，若该行已有 pending session，也只更新 metadata，不要求重新生成 OAuth URL。
- 批量行完成 OAuth 登录后，最终账号详情必须体现最后一次编辑后的元数据。

## 验收标准

- Given 单账号 OAuth 已生成 URL，When 用户编辑显示名称、分组、分组备注、标签、母号或邮箱绑定，Then `Copy OAuth URL` 与 `Complete OAuth login` 继续可用，页面不再要求重新生成 URL。
- Given 批量 OAuth 某行已生成 URL，When 用户编辑该行 metadata 或顶部默认分组回填到该行，Then 该行现有 URL、callback 与 pending session 保持可用。
- Given pending session 在 URL 生成后被多次更新，When callback 或手动完成登录最终落库，Then 账号使用最后一次同步后的 metadata。
- Given login session 已经 `completed`、`failed` 或 `expired`，When 前端尝试更新 metadata，Then 服务端明确拒绝更新。

## 质量门槛

- `cargo test`
- `cd web && bun run test`
- `cd web && bun run build`
- `cd web && bun run build-storybook`

## 实现备注

- 本增量为 `g4ek6` OAuth 创建流追加 pending-session metadata sync 能力，不回写主 spec。
- 本增量显式替换 `e5w9m` 中“邮箱编辑后必须清空 pending OAuth URL”的旧边界。
- 本增量补齐 `m7a9k` 与 `thyxm` 已引入的邮箱绑定 / 分组备注 metadata，使其在 pending OAuth session 上也可以热更新。

## 验证结果

- 2026-03-25: `cargo test update_oauth_login_session -- --nocapture`
- 2026-03-25: `cd /Users/ivan/.codex/worktrees/f3f3/codex-vibe-monitor/web && bun run test -- UpstreamAccountCreate.test.tsx`
- 2026-03-25: `cd /Users/ivan/.codex/worktrees/f3f3/codex-vibe-monitor/web && bun run build`
- 2026-03-25: `cd /Users/ivan/.codex/worktrees/f3f3/codex-vibe-monitor/web && bun run build-storybook`
- 2026-03-25: `cargo test` 仍被现存基线用例 `tests::pool_route_non_capture_request_body_read_timeout_applies_to_replay_stream` 阻塞。
- 2026-03-25: `cd /Users/ivan/.codex/worktrees/f3f3/codex-vibe-monitor/web && bun run test` 仍被现存基线失败阻塞，主要落在 `src/pages/account-pool/UpstreamAccounts.test.tsx`、`src/pages/Records.test.tsx`、`src/pages/Live.test.tsx` 与 `src/components/AccountTagFilterCombobox.test.tsx`。

## 变更记录

- 2026-03-25: 创建增量 spec，冻结 pending OAuth login session metadata live-sync 的接口、交互与验收边界。
- 2026-03-25: 完成后端 pending-session metadata PATCH、前端单账号/批量 OAuth 热更新、Storybook/Vitest/Rust 定向回归，并补充本地视觉证据。
