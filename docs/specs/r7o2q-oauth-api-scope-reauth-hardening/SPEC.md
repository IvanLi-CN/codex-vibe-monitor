# OAuth 池账号 API Scope 与重授权误判修复（#r7o2q）

## 状态

- Status: 已完成
- Created: 2026-03-16
- Last: 2026-03-16

## 背景 / 问题陈述

- 当前号池里的 Codex OAuth 账号可以成功同步 usage，但首次被用于 `/v1/responses` 代理时会立即收到 `401/403`。
- 现网排查确认旧 OAuth token 缺少 `api.responses.write` / `api.model.read` 等 API scopes；同时路由阶段把任意 OAuth `401/403` 都直接标成 `needs_reauth`，导致 UI 误导成“授权失效”。
- 结果是账号会表现为“同步后暂时正常，过一会又需要重新授权”，但根因其实分成两类：`scope 不足 / 权限不足` 与 `token 真失效`。

## 目标 / 非目标

### Goals

- OAuth 登录申请必须包含当前代理请求所需的 API scopes，并补齐 Codex OAuth authorize 参数中的 API audience / prompt，避免新授权账号缺少 API 权限。
- OAuth 路由阶段只在明确凭据失效时才转 `needs_reauth`；缺少 scopes、普通权限不足等 `401/403` 必须转 `error` 并保留诊断信息。
- 路由失败的上游 JSON 错误要提取精简 message / code / request id，用于 `last_error` 和 invocation payload 排障。
- 号池 UI 要明确提示：旧 OAuth 账号在部署后需要重新授权一次才能拿到新增 scopes；缺 scope 时优先显示权限提示，而不是笼统“需要重新授权”。

### Non-goals

- 不做数据库批量回填，也不尝试自动让旧 token 获得新增 scopes。
- 不改变 API Key 账号的路由与状态机语义。
- 不修改 101 的运行配置或部署拓扑。

## 需求（Requirements）

### MUST

- OAuth authorize URL 至少包含：现有登录/usage scopes、`api.model.read`、`api.responses.write`、`audience=https://api.openai.com/v1`、`prompt=login`。
- 路由阶段的 OAuth `401/403` 只有在错误文本明确指向 `invalid_grant`、token invalidated、必须重新登录等场景时，才可落 `needs_reauth`。
- `Missing scopes`、`insufficient permissions` 这类 API 权限问题必须落 `error`，并把上游 message 写入 `last_error`。
- 当上游返回结构化 JSON 错误时，invocation payload 需要记录 `upstreamErrorCode`、`upstreamErrorMessage`、`upstreamRequestId`；HTML 或非 JSON 拦截页不得把整页内容塞进错误摘要。
- 文档必须明确：这次发布后，现有 OAuth 账号需要重新授权一次；重新授权后的账号不应再因 scope 缺失被循环打回。

## 验收标准（Acceptance Criteria）

- 新创建或重新授权的 OAuth 账号能持续通过 `/v1/responses` 路由，不会在首次实际请求后立刻掉回异常状态。
- 旧 OAuth 账号若仍携带旧 scopes，路由时会进入 `error` 并显示缺 scope 诊断，而不是误标成 `needs_reauth`。
- refresh/token 端的明确失效仍会把账号转成 `needs_reauth`。
- 相关 Rust / Web 测试覆盖 authorize URL scope、路由期 `Missing scopes`、明确 invalidated token、以及 HTML 错误体忽略分支。

## 实现备注

- 这次修复是 `g4ek6` 号池账号管理的一次增量补丁，重点在“OAuth API 权限契约”与“状态机误判收紧”。
- 部署后的操作说明应面向运维清晰写出：先部署，再对旧 OAuth 账号执行一次重新授权。
